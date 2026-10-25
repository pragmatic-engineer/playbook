// SPDX-FileCopyrightText: 2026 Igor Santos
// SPDX-License-Identifier: Apache-2.0

//! Go symbol and import extraction. Only package level declarations are
//! symbols; function bodies are not walked.

use super::{clean_signature, Extracted, Import, Symbol, MAX_SIGNATURE_CHARS};
use std::borrow::Cow;
use tree_sitter::Node;

/// Pointer and generic wrappers around a receiver type are unwrapped up to this depth.
const MAX_DEPTH: usize = 64;

struct Walker<'a> {
    source: &'a [u8],
    out: Extracted,
}

/// The node's source text, with any invalid UTF-8 replaced.
fn text<'a>(node: Node, source: &'a [u8]) -> Cow<'a, str> {
    String::from_utf8_lossy(&source[node.start_byte()..node.end_byte()])
}

pub fn extract(root: Node, source: &[u8]) -> Extracted {
    let mut walker = Walker {
        source,
        out: Extracted::default(),
    };
    let mut cursor = root.walk();
    let children: Vec<Node> = root.named_children(&mut cursor).collect();
    for node in children {
        match node.kind() {
            "function_declaration" => walker.function(node),
            "method_declaration" => walker.method(node),
            "type_declaration" | "const_declaration" | "var_declaration" => {
                walker.declaration(node)
            }
            "import_declaration" => walker.import_declaration(node),
            _ => {}
        }
    }
    walker.link_methods();
    walker.out
}

/// Go exports a name when its first character is an upper case letter.
fn is_exported(name: &str) -> bool {
    name.chars().next().is_some_and(char::is_uppercase)
}

/// The type name of a receiver: `*Server[T]` is `Server`.
fn receiver_type(receiver: Node, source: &[u8]) -> Option<String> {
    let param = receiver
        .named_children(&mut receiver.walk())
        .find(|n| n.kind() == "parameter_declaration")?;
    let mut current = param.child_by_field_name("type")?;
    for _ in 0..MAX_DEPTH {
        match current.kind() {
            "pointer_type" => current = current.named_child(0)?,
            "generic_type" => current = current.child_by_field_name("type")?,
            "parenthesized_type" => current = current.named_child(0)?,
            _ => break,
        }
    }
    (current.kind() == "type_identifier").then(|| text(current, source).to_string())
}

impl Walker<'_> {
    fn function(&mut self, node: Node) {
        let Some(name) = node.child_by_field_name("name") else {
            return;
        };
        let head_end = node
            .child_by_field_name("body")
            .map_or(node.end_byte(), |b| b.start_byte());
        let name = text(name, self.source).to_string();
        self.push(name.clone(), name, "function", node, node, head_end);
    }

    fn method(&mut self, node: Node) {
        let Some(name) = node.child_by_field_name("name") else {
            return;
        };
        let head_end = node
            .child_by_field_name("body")
            .map_or(node.end_byte(), |b| b.start_byte());
        let name = text(name, self.source).to_string();
        let qualified = match node
            .child_by_field_name("receiver")
            .and_then(|r| receiver_type(r, self.source))
        {
            Some(recv) => format!("{recv}.{name}"),
            None => name.clone(),
        };
        self.push(name, qualified, "method", node, node, head_end);
    }

    /// `type`, `const` and `var` declarations, single or grouped.
    fn declaration(&mut self, decl: Node) {
        let mut cursor = decl.walk();
        let mut specs: Vec<Node> = Vec::new();
        for child in decl.named_children(&mut cursor) {
            if child.kind() == "var_spec_list" {
                let mut inner = child.walk();
                specs.extend(child.named_children(&mut inner));
            } else {
                specs.push(child);
            }
        }
        let grouped = decl.children(&mut decl.walk()).any(|c| c.kind() == "(")
            || decl
                .named_children(&mut decl.walk())
                .any(|c| c.kind() == "var_spec_list");
        for spec in specs {
            // A lone spec takes the whole declaration, so the range and the
            // signature start at the keyword.
            let outer = if grouped { spec } else { decl };
            match spec.kind() {
                "type_spec" | "type_alias" => self.type_spec(spec, outer),
                "const_spec" => self.value_spec(spec, outer, "const"),
                "var_spec" => self.value_spec(spec, outer, "variable"),
                _ => {}
            }
        }
    }

    fn type_spec(&mut self, spec: Node, outer: Node) {
        let Some(name) = spec.child_by_field_name("name") else {
            return;
        };
        let ty = spec.child_by_field_name("type");
        let kind = match (spec.kind(), ty.map(|t| t.kind())) {
            ("type_alias", _) => "type_alias",
            (_, Some("struct_type")) => "struct",
            (_, Some("interface_type")) => "interface",
            _ => "type",
        };
        // The head stops at the opening brace of a struct or interface body.
        let head_end = ty
            .and_then(|t| {
                t.children(&mut t.walk())
                    .find(|c| c.kind() == "{" || c.kind() == "field_declaration_list")
            })
            .map_or(spec.end_byte(), |b| b.start_byte());
        let name_text = text(name, self.source).to_string();
        let index = self.push(
            name_text.clone(),
            name_text.clone(),
            kind,
            outer,
            outer,
            head_end,
        );
        if let (Some(index), Some(ty)) = (index, ty) {
            if ty.kind() == "interface_type" {
                self.interface_methods(ty, &name_text, index);
            }
        }
    }

    fn interface_methods(&mut self, iface: Node, iface_name: &str, parent: usize) {
        let mut cursor = iface.walk();
        let elems: Vec<Node> = iface
            .named_children(&mut cursor)
            .filter(|n| n.kind() == "method_elem")
            .collect();
        for elem in elems {
            let Some(name) = elem.child_by_field_name("name") else {
                continue;
            };
            let name = text(name, self.source).to_string();
            let index = self.push(
                name.clone(),
                format!("{iface_name}.{name}"),
                "method",
                elem,
                elem,
                elem.end_byte(),
            );
            if let Some(index) = index {
                self.out.symbols[index].parent = Some(parent);
            }
        }
    }

    fn value_spec(&mut self, spec: Node, outer: Node, kind: &'static str) {
        let mut cursor = spec.walk();
        let names: Vec<Node> = spec.children_by_field_name("name", &mut cursor).collect();
        for name in names {
            let name = text(name, self.source).to_string();
            self.push(name.clone(), name, kind, outer, outer, outer.end_byte());
        }
    }

    fn push(
        &mut self,
        name: String,
        qualified_name: String,
        kind: &'static str,
        outer: Node,
        sig_node: Node,
        head_end: usize,
    ) -> Option<usize> {
        if name.is_empty() || name == "_" {
            return None;
        }
        let sig_start = sig_node.start_byte();
        // Only the first characters are kept, so a huge statement is not decoded whole.
        let wanted = head_end.min(self.source.len());
        let end = wanted.min(sig_start.saturating_add(MAX_SIGNATURE_CHARS * 4 + 4));
        let raw = String::from_utf8_lossy(&self.source[sig_start.min(end)..end]);
        let (signature, mut signature_cut) = clean_signature(raw.trim_end());
        signature_cut |= end < wanted;
        let exported = is_exported(&name);
        self.out.symbols.push(Symbol {
            name,
            qualified_name,
            kind,
            signature: Some(signature),
            signature_cut,
            start_byte: outer.start_byte(),
            end_byte: outer.end_byte(),
            start_line: outer.start_position().row + 1,
            end_line: outer.end_position().row + 1,
            parent: None,
            exported,
        });
        Some(self.out.symbols.len() - 1)
    }

    /// Hangs each method declared with a receiver under its type, when the
    /// type is declared in the same file.
    fn link_methods(&mut self) {
        for i in 0..self.out.symbols.len() {
            let sym = &self.out.symbols[i];
            if sym.kind != "method" || sym.parent.is_some() {
                continue;
            }
            let Some((recv, _)) = sym.qualified_name.split_once('.') else {
                continue;
            };
            let parent = self.out.symbols.iter().position(|t| {
                t.qualified_name == recv && matches!(t.kind, "struct" | "interface" | "type")
            });
            self.out.symbols[i].parent = parent;
        }
    }

    fn import_declaration(&mut self, decl: Node) {
        let mut cursor = decl.walk();
        let mut specs: Vec<Node> = Vec::new();
        for child in decl.named_children(&mut cursor) {
            match child.kind() {
                "import_spec" => specs.push(child),
                "import_spec_list" => {
                    let mut inner = child.walk();
                    specs.extend(
                        child
                            .named_children(&mut inner)
                            .filter(|n| n.kind() == "import_spec"),
                    );
                }
                _ => {}
            }
        }
        for spec in specs {
            let Some(path) = spec.child_by_field_name("path") else {
                continue;
            };
            let module = text(path, self.source)
                .trim_matches(|c| c == '"' || c == '`')
                .to_string();
            let alias = spec
                .child_by_field_name("name")
                .map(|n| format!("as {}", text(n, self.source)));
            self.out.imports.push(Import {
                module,
                names: alias,
                line: spec.start_position().row + 1,
            });
        }
    }
}

/// A small Go file shared by the outline and refs tests.
#[cfg(test)]
pub(crate) const FIXTURE: &str = "\
package store

import (
\t\"errors\"
\tlog \"log\"
)

// MaxItems caps a store.
const MaxItems = 100

var ErrFull = errors.New(\"full\")

type Item struct {
\tName string
}

type Store interface {
\tPut(item Item) error
}

type memory struct {
\titems []Item
}

func New() *memory { return &memory{} }

func (m *memory) Put(item Item) error {
\tif len(m.items) >= MaxItems {
\t\tlog.Println(\"full\")
\t\treturn ErrFull
\t}
\tm.items = append(m.items, item)
\treturn nil
}
";

#[cfg(test)]
mod tests {
    use super::super::{extract as extract_any, find, parse, Import, Lang, Symbol};

    fn run(src: &str) -> (Vec<Symbol>, Vec<Import>) {
        let parsed = parse(Lang::Go, src.as_bytes()).expect("parse");
        let out = extract_any(Lang::Go, &parsed, src.as_bytes());
        (out.symbols, out.imports)
    }

    #[test]
    fn functions_structs_interfaces_types_consts_and_vars_get_their_kinds() {
        // Arrange
        let src = "\
package demo

func Run(n int) error { return nil }

type Server struct {
\tName string
}

type Store interface {
\tGet(key string) ([]byte, error)
}

type ID int

type Alias = ID

const Max = 10

var Default = \"x\"
";

        // Act
        let (symbols, _) = run(src);

        // Assert
        let got: Vec<(&str, &str)> = symbols
            .iter()
            .map(|s| (s.qualified_name.as_str(), s.kind))
            .collect();
        assert_eq!(
            got,
            vec![
                ("Run", "function"),
                ("Server", "struct"),
                ("Store", "interface"),
                ("Store.Get", "method"),
                ("ID", "type"),
                ("Alias", "type_alias"),
                ("Max", "const"),
                ("Default", "variable"),
            ]
        );
    }

    #[test]
    fn methods_are_qualified_by_their_receiver_and_hang_under_its_type() {
        // Arrange
        let src = "\
package demo

type Server struct{}

func (s *Server) Start() {}
func (s Server) Name() string { return \"\" }
func (b *Box[T]) Put(v T) {}
";

        // Act
        let (symbols, _) = run(src);

        // Assert
        let start = find(&symbols, "Server.Start");
        assert_eq!((start.kind, start.name.as_str()), ("method", "Start"));
        assert_eq!(start.parent, Some(0));
        assert_eq!(start.signature.as_deref(), Some("func (s *Server) Start()"));
        assert_eq!(find(&symbols, "Server.Name").parent, Some(0));
        let put = find(&symbols, "Box.Put");
        assert_eq!(put.parent, None);
    }

    #[test]
    fn grouped_const_and_var_blocks_give_one_symbol_per_name() {
        // Arrange
        let src = "\
package demo

const (
\tA = iota
\tB
)

var x, y int

var (
\tz = 1
)
";

        // Act
        let (symbols, _) = run(src);

        // Assert
        let got: Vec<(&str, &str, usize)> = symbols
            .iter()
            .map(|s| (s.name.as_str(), s.kind, s.start_line))
            .collect();
        assert_eq!(
            got,
            vec![
                ("A", "const", 4),
                ("B", "const", 5),
                ("x", "variable", 8),
                ("y", "variable", 8),
                ("z", "variable", 11),
            ]
        );
    }

    #[test]
    fn a_name_is_exported_only_when_it_starts_with_an_upper_case_letter() {
        // Arrange
        let src = "package demo\nfunc Public() {}\nfunc private() {}\nfunc _() {}\n";

        // Act
        let (symbols, _) = run(src);

        // Assert
        assert_eq!(symbols.len(), 2);
        assert!(find(&symbols, "Public").exported);
        assert!(!find(&symbols, "private").exported);
    }

    #[test]
    fn signatures_stop_before_the_body_and_the_struct_brace() {
        // Arrange
        let src = "package demo\n\nfunc Add(a, b int) int {\n\treturn a + b\n}\n\ntype P struct {\n\tX int\n}\n\nconst Pi = 3.14\n";

        // Act
        let (symbols, _) = run(src);

        // Assert
        assert_eq!(
            find(&symbols, "Add").signature.as_deref(),
            Some("func Add(a, b int) int")
        );
        assert_eq!(
            find(&symbols, "P").signature.as_deref(),
            Some("type P struct")
        );
        assert_eq!(
            find(&symbols, "Pi").signature.as_deref(),
            Some("const Pi = 3.14")
        );
        assert_eq!(find(&symbols, "P").start_line, 7);
        assert_eq!(find(&symbols, "P").end_line, 9);
    }

    #[test]
    fn local_declarations_inside_function_bodies_are_not_symbols() {
        // Arrange
        let src = "package demo\nfunc F() {\n\tconst k = 1\n\ttype t struct{}\n\tvar v int\n}\n";

        // Act
        let (symbols, _) = run(src);

        // Assert
        let names: Vec<&str> = symbols.iter().map(|s| s.name.as_str()).collect();
        assert_eq!(names, vec!["F"]);
    }

    #[test]
    fn imports_record_path_alias_and_line_for_every_form() {
        // Arrange
        let src = "\
package demo

import \"fmt\"
import (
\t\"os\"
\tio2 \"io\"
\t_ \"embed\"
\t. \"strings\"
)
";

        // Act
        let (_, imports) = run(src);

        // Assert
        let got: Vec<(&str, Option<&str>, usize)> = imports
            .iter()
            .map(|i| (i.module.as_str(), i.names.as_deref(), i.line))
            .collect();
        assert_eq!(
            got,
            vec![
                ("fmt", None, 3),
                ("os", None, 5),
                ("io", Some("as io2"), 6),
                ("embed", Some("as _"), 7),
                ("strings", Some("as ."), 8),
            ]
        );
    }

    #[test]
    fn the_shared_edge_cases_hold_for_go() {
        super::super::edge_cases(Lang::Go, "func Thing() {}", "Thing", "func broken(", "//");
    }
}
