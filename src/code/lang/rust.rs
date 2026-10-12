// SPDX-FileCopyrightText: 2026 Igor Santos
// SPDX-License-Identifier: Apache-2.0

//! Rust symbol and import extraction.

use super::{clean_signature, Extracted, Import, Symbol};
use std::borrow::Cow;
use tree_sitter::Node;

/// Nesting deeper than this (modules inside modules) is not followed.
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
    walker.items(root, "", None, 0);
    walker.out
}

/// The name of an impl target: `Wrapper<T>` is `Wrapper`, `&Foo` is `Foo`.
fn type_name(node: Node, source: &[u8]) -> String {
    let mut current = node;
    // Followed in a loop and capped, so a hostile chain of `&` cannot overflow the stack.
    for _ in 0..MAX_DEPTH {
        let field = match current.kind() {
            "generic_type" | "reference_type" | "pointer_type" => "type",
            "dynamic_type" => "trait",
            _ => break,
        };
        match current.child_by_field_name(field) {
            Some(next) => current = next,
            None => break,
        }
    }
    text(current, source).to_string()
}

fn join(prefix: &str, name: &str) -> String {
    if prefix.is_empty() {
        name.to_string()
    } else {
        format!("{prefix}::{name}")
    }
}

impl Walker<'_> {
    fn items(&mut self, parent_node: Node, prefix: &str, parent: Option<usize>, depth: usize) {
        if depth > MAX_DEPTH {
            return;
        }
        let mut cursor = parent_node.walk();
        let children: Vec<Node> = parent_node.named_children(&mut cursor).collect();
        for node in children {
            self.item(node, prefix, parent, depth);
        }
    }

    fn item(&mut self, node: Node, prefix: &str, parent: Option<usize>, depth: usize) {
        let simple = match node.kind() {
            "function_item" | "function_signature_item" => Some("function"),
            "struct_item" => Some("struct"),
            "enum_item" => Some("enum"),
            "union_item" => Some("union"),
            "type_item" => Some("type_alias"),
            "const_item" => Some("const"),
            "static_item" => Some("static"),
            "macro_definition" => Some("macro"),
            _ => None,
        };
        if let Some(kind) = simple {
            self.symbol(node, kind, prefix, parent);
            return;
        }
        match node.kind() {
            "trait_item" => {
                if let Some(index) = self.symbol(node, "trait", prefix, parent) {
                    let name = self.out.symbols[index].qualified_name.clone();
                    self.members(node, &name, Some(index));
                }
            }
            "mod_item" => {
                if let Some(index) = self.symbol(node, "module", prefix, parent) {
                    if let Some(body) = node.child_by_field_name("body") {
                        let name = self.out.symbols[index].qualified_name.clone();
                        self.items(body, &name, Some(index), depth + 1);
                    }
                }
            }
            "impl_item" => {
                if let Some(target) = node.child_by_field_name("type") {
                    let qualified = join(prefix, &type_name(target, self.source));
                    self.members(node, &qualified, parent);
                }
            }
            "foreign_mod_item" => {
                if let Some(body) = node.child_by_field_name("body") {
                    self.items(body, prefix, parent, depth + 1);
                }
            }
            "use_declaration" => self.use_declaration(node),
            _ => {}
        }
    }

    /// Functions, constants and types inside an impl or trait body, qualified by `owner`.
    fn members(&mut self, owner_node: Node, owner: &str, parent: Option<usize>) {
        let Some(body) = owner_node.child_by_field_name("body") else {
            return;
        };
        let mut cursor = body.walk();
        let children: Vec<Node> = body.named_children(&mut cursor).collect();
        for node in children {
            let kind = match node.kind() {
                "function_item" | "function_signature_item" => "method",
                "const_item" => "const",
                "type_item" | "associated_type" => "type_alias",
                _ => continue,
            };
            self.symbol(node, kind, owner, parent);
        }
    }

    fn symbol(
        &mut self,
        node: Node,
        kind: &'static str,
        prefix: &str,
        parent: Option<usize>,
    ) -> Option<usize> {
        let name_node = node.child_by_field_name("name")?;
        let name = text(name_node, self.source).to_string();
        if name.is_empty() {
            return None;
        }
        // A macro has no body field, so its head ends with its name.
        let head_end = if node.kind() == "macro_definition" {
            name_node.end_byte()
        } else {
            node.child_by_field_name("body")
                .map_or(node.end_byte(), |b| b.start_byte())
        };
        let raw = String::from_utf8_lossy(&self.source[node.start_byte()..head_end]);
        let (signature, signature_cut) =
            clean_signature(raw.trim_end_matches(|c: char| c == ';' || c.is_whitespace()));
        self.out.symbols.push(Symbol {
            qualified_name: join(prefix, &name),
            name,
            kind,
            signature: Some(signature),
            signature_cut,
            start_byte: node.start_byte(),
            end_byte: node.end_byte(),
            start_line: node.start_position().row + 1,
            end_line: node.end_position().row + 1,
            parent,
            exported: node
                .children(&mut node.walk())
                .any(|c| c.kind() == "visibility_modifier"),
        });
        Some(self.out.symbols.len() - 1)
    }

    fn use_declaration(&mut self, node: Node) {
        let Some(arg) = node.child_by_field_name("argument") else {
            return;
        };
        let line = node.start_position().row + 1;
        let (module, names) = self.use_parts(arg);
        self.out.imports.push(Import {
            module,
            names,
            line,
        });
    }

    fn field_text(&self, node: Node, field: &str) -> Option<String> {
        node.child_by_field_name(field)
            .map(|n| text(n, self.source).to_string())
    }

    fn use_parts(&self, arg: Node) -> (String, Option<String>) {
        match arg.kind() {
            "scoped_identifier" => (
                self.field_text(arg, "path").unwrap_or_default(),
                self.field_text(arg, "name"),
            ),
            "scoped_use_list" => (
                self.field_text(arg, "path").unwrap_or_default(),
                arg.child_by_field_name("list").map(|l| self.list_names(l)),
            ),
            "use_list" => (String::new(), Some(self.list_names(arg))),
            "use_wildcard" => {
                let full = text(arg, self.source);
                let module = full.trim_end_matches('*').trim_end_matches("::");
                (module.to_string(), Some("*".to_string()))
            }
            "use_as_clause" => {
                let alias = self.field_text(arg, "alias").unwrap_or_default();
                let (module, name) = match arg.child_by_field_name("path") {
                    Some(p) if p.kind() == "scoped_identifier" => (
                        self.field_text(p, "path").unwrap_or_default(),
                        self.field_text(p, "name"),
                    ),
                    Some(p) => (text(p, self.source).to_string(), None),
                    None => (String::new(), None),
                };
                match name {
                    Some(name) => (module, Some(format!("{name} as {alias}"))),
                    None => (module, Some(format!("as {alias}"))),
                }
            }
            _ => (text(arg, self.source).to_string(), None),
        }
    }

    fn list_names(&self, list: Node) -> String {
        let mut cursor = list.walk();
        list.named_children(&mut cursor)
            .filter(|n| !n.is_extra())
            .map(|n| text(n, self.source).to_string())
            .collect::<Vec<_>>()
            .join(", ")
    }
}

#[cfg(test)]
mod tests {
    use super::super::{extract as extract_any, parse, Import, Lang, Symbol};

    fn run(src: &str) -> (Vec<Symbol>, Vec<Import>) {
        let parsed = parse(Lang::Rust, src.as_bytes()).expect("parse");
        let out = extract_any(Lang::Rust, &parsed, src.as_bytes());
        (out.symbols, out.imports)
    }

    fn find<'a>(symbols: &'a [Symbol], qualified: &str) -> &'a Symbol {
        symbols
            .iter()
            .find(|s| s.qualified_name == qualified)
            .unwrap_or_else(|| panic!("no symbol {qualified} in {symbols:#?}"))
    }

    #[test]
    fn every_declaration_kind_is_extracted_with_its_range() {
        // Arrange
        let src = "\
pub fn top(a: i32) -> i32 {
    a
}

struct Point {
    x: i32,
}

enum Color {
    Red,
}

trait Shape {
    fn area(&self) -> f64;
}

type Id = u64;

const MAX: usize = 3;

static NAME: &str = \"x\";

mod inner {
    pub fn helper() {}
}

macro_rules! twice {
    ($e:expr) => { $e * 2 };
}
";

        // Act
        let (symbols, _) = run(src);

        // Assert
        let kinds: Vec<(&str, &str)> = symbols
            .iter()
            .map(|s| (s.qualified_name.as_str(), s.kind))
            .collect();
        assert_eq!(
            kinds,
            vec![
                ("top", "function"),
                ("Point", "struct"),
                ("Color", "enum"),
                ("Shape", "trait"),
                ("Shape::area", "method"),
                ("Id", "type_alias"),
                ("MAX", "const"),
                ("NAME", "static"),
                ("inner", "module"),
                ("inner::helper", "function"),
                ("twice", "macro"),
            ]
        );
        let top = find(&symbols, "top");
        assert_eq!((top.start_line, top.end_line), (1, 3));
        assert_eq!(top.signature.as_deref(), Some("pub fn top(a: i32) -> i32"));
        let helper = find(&symbols, "inner::helper");
        assert_eq!(
            helper.parent,
            symbols.iter().position(|s| s.name == "inner")
        );
        assert_eq!(
            (
                find(&symbols, "Point").start_line,
                find(&symbols, "Point").end_line
            ),
            (5, 7)
        );
    }

    #[test]
    fn impl_methods_are_qualified_by_their_type_and_enclosing_modules() {
        // Arrange
        let src = "\
mod a {
    struct Type;
    impl Type {
        fn method(&self) {}
    }
    impl<T> Wrapper<T> {
        fn new() -> Self { todo!() }
    }
    impl Display for Type {
        fn fmt(&self) {}
    }
}
";

        // Act
        let (symbols, _) = run(src);

        // Assert
        for want in ["a::Type::method", "a::Wrapper::new", "a::Type::fmt"] {
            assert_eq!(find(&symbols, want).kind, "method", "{want}");
        }
        assert_eq!(find(&symbols, "a::Type::method").name, "method");
    }

    #[test]
    fn visibility_unions_extern_blocks_trait_types_and_macro_heads_are_extracted() {
        // Arrange: the `pub fn` body holds an invalid byte, so the item text is not valid UTF-8.
        let mut src = b"pub fn shown() { /* \xff */ }\nfn hidden() {}\n".to_vec();
        src.extend_from_slice(
            b"union U { a: u32 }\n\
extern \"C\" { fn from_c(); }\n\
trait T { type Item; }\n\
macro_rules! twice { ($e:expr) => { $e * 2 }; }\n\
impl dyn T { fn d(&self) {} }\n\
impl T for &&Foo { fn f(&self) {} }\n\
impl fmt::Display for x::Y { fn g(&self) {} }\n",
        );
        let parsed = parse(Lang::Rust, &src).expect("parse");

        // Act
        let symbols = extract_any(Lang::Rust, &parsed, &src).symbols;

        // Assert
        assert!(find(&symbols, "shown").exported);
        assert!(!find(&symbols, "hidden").exported);
        assert_eq!(find(&symbols, "U").kind, "union");
        assert_eq!(find(&symbols, "from_c").kind, "function");
        assert_eq!(find(&symbols, "T::Item").kind, "type_alias");
        assert_eq!(
            find(&symbols, "twice").signature.as_deref(),
            Some("macro_rules! twice")
        );
        assert_eq!(find(&symbols, "T::d").kind, "method");
        assert_eq!(find(&symbols, "Foo::f").kind, "method");
        assert_eq!(find(&symbols, "x::Y::g").kind, "method");
    }

    #[test]
    fn a_long_chain_of_references_in_an_impl_target_does_not_overflow() {
        // Arrange
        let src = format!("impl {}Foo {{ fn f() {{}} }}\n", "&".repeat(100_000));

        // Act, Assert: it returns instead of overflowing the stack.
        let _ = run(&src);
    }

    #[test]
    fn use_with_an_alias_and_a_comment_inside_a_list() {
        // Arrange
        let src = "use serde as s;\nuse std::{io, // note\n fmt};\n";

        // Act
        let (_, imports) = run(src);

        // Assert
        assert_eq!(imports[0].module, "serde");
        assert_eq!(imports[0].names.as_deref(), Some("as s"));
        assert_eq!(imports[1].names.as_deref(), Some("io, fmt"));
    }

    #[test]
    fn ranges_are_right_after_multibyte_text() {
        // Arrange
        let src = "struct Caf\u{e9};\nfn \u{fc}n\u{ef}() {}\n";

        // Act
        let (symbols, _) = run(src);

        // Assert
        let cafe = find(&symbols, "Caf\u{e9}");
        assert_eq!(cafe.start_byte, 0);
        assert_eq!(&src[cafe.start_byte..cafe.end_byte], "struct Caf\u{e9};");
        let uni = find(&symbols, "\u{fc}n\u{ef}");
        assert_eq!(uni.start_line, 2);
        assert_eq!(&src[uni.start_byte..uni.end_byte], "fn \u{fc}n\u{ef}() {}");
    }

    #[test]
    fn imports_record_module_names_and_line_for_every_use_form() {
        // Arrange
        let src = "\
use std::collections::HashMap;
use std::io::{Read, Write};
use crate::code::*;
use std::fmt::Result as FmtResult;
use serde;
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
                ("std::collections", Some("HashMap"), 1),
                ("std::io", Some("Read, Write"), 2),
                ("crate::code", Some("*"), 3),
                ("std::fmt", Some("Result as FmtResult"), 4),
                ("serde", None, 5),
            ]
        );
    }

    #[test]
    fn signatures_of_199_200_and_201_characters_come_back_cut_only_at_the_cap() {
        for (len, want_len, cut) in [
            (199usize, 199usize, false),
            (200, 200, false),
            (201, 200, true),
        ] {
            // Arrange: the signature is `fn <name>()`, five characters longer than the name.
            let name = "a".repeat(len - 5);
            let src = format!("fn {name}() {{}}\n");

            // Act
            let (symbols, _) = run(&src);

            // Assert
            let sig = symbols[0].signature.clone().expect("signature");
            assert_eq!(sig.chars().count(), want_len, "{len}");
            assert_eq!(symbols[0].signature_cut, cut, "{len}");
        }
    }

    #[test]
    fn partial_syntax_keeps_the_symbols_before_the_error() {
        // Arrange
        let src = "fn good() {}\nfn broken( {\n";

        // Act
        let parsed = parse(Lang::Rust, src.as_bytes()).expect("parse");
        let out = extract_any(Lang::Rust, &parsed, src.as_bytes());

        // Assert
        assert!(parsed.partial);
        assert!(out.symbols.iter().any(|s| s.name == "good"));
    }

    #[test]
    fn an_empty_file_and_crlf_endings_extract_cleanly() {
        // Arrange, Act
        let (empty, _) = run("");
        let (crlf, _) = run("\u{feff}fn a() {}\r\nfn b() {}\r\n");

        // Assert
        assert!(empty.is_empty());
        assert_eq!(find(&crlf, "b").start_line, 2);
    }

    #[test]
    fn invalid_utf8_bytes_do_not_stop_extraction() {
        // Arrange
        let mut bytes = b"fn before() {}\n// ".to_vec();
        bytes.extend_from_slice(&[0xFF, 0xFE]);
        bytes.extend_from_slice(b"\nfn after() {}\n");

        // Act
        let parsed = parse(Lang::Rust, &bytes).expect("parse");
        let out = extract_any(Lang::Rust, &parsed, &bytes);

        // Assert
        let names: Vec<&str> = out.symbols.iter().map(|s| s.name.as_str()).collect();
        assert!(
            names.contains(&"before") && names.contains(&"after"),
            "{names:?}"
        );
    }

    #[test]
    fn the_shared_edge_cases_hold_for_rust() {
        super::super::edge_cases(Lang::Rust, "fn thing() {}", "thing", "fn broken( {", "//");
    }
}
