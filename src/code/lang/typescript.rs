// SPDX-FileCopyrightText: 2026 Igor Santos
// SPDX-License-Identifier: Apache-2.0

//! TypeScript, TSX and JavaScript symbol and import extraction. One walker
//! serves all three grammars, because their node kinds overlap.

use super::{clean_signature, Extracted, Import, Symbol};
use std::borrow::Cow;
use tree_sitter::Node;

/// Nesting deeper than this (namespaces inside namespaces) is not followed.
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

fn join(prefix: &str, name: &str) -> String {
    if prefix.is_empty() {
        name.to_string()
    } else {
        format!("{prefix}.{name}")
    }
}

fn is_function_value(node: Node) -> bool {
    matches!(
        node.kind(),
        "arrow_function"
            | "function_expression"
            | "function"
            | "generator_function"
            | "generator_function_expression"
    )
}

/// What one declaration contributes: where its name is, which node supplies
/// the range, and where the signature ends.
struct Decl<'t> {
    name: Node<'t>,
    /// Supplies the line and byte range. The `export` keyword is included.
    range: Node<'t>,
    /// The signature runs from the start of `range` to here.
    head_end: usize,
}

/// Where a declaration sits: its name prefix, its parent symbol and its depth.
#[derive(Clone, Copy)]
struct Scope<'p> {
    prefix: &'p str,
    parent: Option<usize>,
    depth: usize,
}

impl Walker<'_> {
    fn items(&mut self, parent_node: Node, prefix: &str, parent: Option<usize>, depth: usize) {
        if depth > MAX_DEPTH {
            return;
        }
        let scope = Scope {
            prefix,
            parent,
            depth,
        };
        let mut cursor = parent_node.walk();
        let children: Vec<Node> = parent_node.named_children(&mut cursor).collect();
        for node in children {
            self.statement(node, node, false, scope);
        }
    }

    /// `outer` is the node whose range the symbol takes: the export statement
    /// when there is one, else the declaration itself.
    fn statement(&mut self, node: Node, outer: Node, exported: bool, scope: Scope) {
        match node.kind() {
            "export_statement" => self.export_statement(node, scope),
            "import_statement" => self.import_statement(node),
            "function_declaration" | "generator_function_declaration" | "function_signature" => {
                self.declared(node, outer, "function", exported, scope);
            }
            "class_declaration" | "abstract_class_declaration" => {
                self.container(node, outer, "class", exported, scope);
            }
            "interface_declaration" => {
                self.container(node, outer, "interface", exported, scope);
            }
            "type_alias_declaration" => {
                self.declared(node, outer, "type_alias", exported, scope);
            }
            "enum_declaration" => {
                self.declared(node, outer, "enum", exported, scope);
            }
            "internal_module" | "module" => {
                if let Some(index) = self.declared(node, outer, "namespace", exported, scope) {
                    if let Some(body) = node.child_by_field_name("body") {
                        let name = self.out.symbols[index].qualified_name.clone();
                        self.items(body, &name, Some(index), scope.depth + 1);
                    }
                }
            }
            "lexical_declaration" | "variable_declaration" => {
                self.variables(node, outer, exported, scope);
            }
            "expression_statement" => {
                // TypeScript wraps `namespace A {}` in an expression statement.
                let mut cursor = node.walk();
                let inner: Vec<Node> = node.named_children(&mut cursor).collect();
                for child in inner {
                    if child.kind() == "internal_module" {
                        self.statement(child, outer, exported, scope);
                    }
                }
            }
            _ => {}
        }
    }

    /// A class or interface, then its members.
    fn container(
        &mut self,
        node: Node,
        outer: Node,
        kind: &'static str,
        exported: bool,
        scope: Scope,
    ) {
        if let Some(index) = self.declared(node, outer, kind, exported, scope) {
            let name = self.out.symbols[index].qualified_name.clone();
            self.members(node, &name, Some(index), exported);
        }
    }

    fn export_statement(&mut self, node: Node, scope: Scope) {
        if let Some(decl) = node.child_by_field_name("declaration") {
            self.statement(decl, node, true, scope);
            return;
        }
        if let Some(source) = node.child_by_field_name("source") {
            // `export { a } from "m"` and `export * from "m"` depend on `m`.
            self.out.imports.push(Import {
                module: unquote(&text(source, self.source)),
                names: Some(export_names(node, self.source)),
                line: node.start_position().row + 1,
            });
            return;
        }
        if let Some(value) = node.child_by_field_name("value") {
            // `export default function f() {}` and `export default class C {}`.
            match value.kind() {
                "function_expression" | "function" | "generator_function" => {
                    self.declared(value, node, "function", true, scope);
                }
                "class" => self.container(value, node, "class", true, scope),
                _ => {}
            }
        }
    }

    /// A declaration whose name and body are fields of the node itself.
    fn declared(
        &mut self,
        node: Node,
        outer: Node,
        kind: &'static str,
        exported: bool,
        scope: Scope,
    ) -> Option<usize> {
        let name = node.child_by_field_name("name")?;
        let head_end = node
            .child_by_field_name("body")
            .map_or(node.end_byte(), |b| b.start_byte());
        self.symbol(
            Decl {
                name,
                range: outer,
                head_end,
            },
            kind,
            exported,
            scope,
        )
    }

    /// `const f = () => {}` is a function. Any other `const` is a const.
    fn variables(&mut self, node: Node, outer: Node, exported: bool, scope: Scope) {
        let is_const = node
            .child(0)
            .is_some_and(|k| text(k, self.source) == "const");
        let mut cursor = node.walk();
        let declarators: Vec<Node> = node
            .named_children(&mut cursor)
            .filter(|n| n.kind() == "variable_declarator")
            .collect();
        for declarator in declarators {
            let Some(name) = declarator.child_by_field_name("name") else {
                continue;
            };
            if name.kind() != "identifier" {
                continue;
            }
            let (kind, head_end) = match declarator.child_by_field_name("value") {
                Some(v) if is_function_value(v) => (
                    "function",
                    v.child_by_field_name("body")
                        .map_or(v.end_byte(), |b| b.start_byte()),
                ),
                _ if is_const => ("const", declarator.end_byte()),
                _ => continue,
            };
            // A statement that declares several names shares its range.
            let decl = Decl {
                name,
                range: outer,
                head_end,
            };
            self.symbol(decl, kind, exported, scope);
        }
    }

    /// Methods of a class or interface body, qualified by `owner`.
    fn members(
        &mut self,
        owner_node: Node,
        owner: &str,
        parent: Option<usize>,
        owner_exported: bool,
    ) {
        let Some(body) = owner_node.child_by_field_name("body") else {
            return;
        };
        let scope = Scope {
            prefix: owner,
            parent,
            depth: 0,
        };
        let mut cursor = body.walk();
        let children: Vec<Node> = body.named_children(&mut cursor).collect();
        for node in children {
            let Some(name) = node.child_by_field_name("name") else {
                continue;
            };
            let head_end = match node.kind() {
                "method_definition" | "method_signature" | "abstract_method_signature" => node
                    .child_by_field_name("body")
                    .map_or(node.end_byte(), |b| b.start_byte()),
                "public_field_definition" | "field_definition" => {
                    match node.child_by_field_name("value") {
                        Some(v) if is_function_value(v) => v
                            .child_by_field_name("body")
                            .map_or(v.end_byte(), |b| b.start_byte()),
                        _ => continue,
                    }
                }
                _ => continue,
            };
            let hidden = node.children(&mut node.walk()).any(|c| {
                c.kind() == "accessibility_modifier"
                    && matches!(text(c, self.source).as_ref(), "private" | "protected")
            }) || name.kind() == "private_property_identifier";
            let decl = Decl {
                name,
                range: node,
                head_end,
            };
            self.symbol(decl, "method", owner_exported && !hidden, scope);
        }
    }

    fn symbol(
        &mut self,
        decl: Decl,
        kind: &'static str,
        exported: bool,
        scope: Scope,
    ) -> Option<usize> {
        let name = text(decl.name, self.source).to_string();
        if name.is_empty() {
            return None;
        }
        let start = decl.range.start_byte();
        let head_end = decl.head_end.max(start).min(self.source.len());
        let raw = String::from_utf8_lossy(&self.source[start..head_end]);
        let (signature, signature_cut) = clean_signature(
            raw.trim_end_matches(|c: char| c == ';' || c == ',' || c.is_whitespace()),
        );
        self.out.symbols.push(Symbol {
            qualified_name: join(scope.prefix, &name),
            name,
            kind,
            signature: Some(signature),
            signature_cut,
            start_byte: start,
            end_byte: decl.range.end_byte(),
            start_line: decl.range.start_position().row + 1,
            end_line: decl.range.end_position().row + 1,
            parent: scope.parent,
            exported,
        });
        Some(self.out.symbols.len() - 1)
    }

    fn import_statement(&mut self, node: Node) {
        let Some(source) = node.child_by_field_name("source") else {
            return;
        };
        let mut cursor = node.walk();
        let clause = node
            .named_children(&mut cursor)
            .find(|n| n.kind() == "import_clause");
        let names = clause.map(|c| self.clause_names(c));
        self.out.imports.push(Import {
            module: unquote(&text(source, self.source)),
            names,
            line: node.start_position().row + 1,
        });
    }

    fn clause_names(&self, clause: Node) -> String {
        let mut parts = Vec::new();
        let mut cursor = clause.walk();
        for child in clause.named_children(&mut cursor) {
            if child.is_extra() {
                continue;
            }
            if child.kind() == "named_imports" {
                let mut inner = child.walk();
                for spec in child.named_children(&mut inner) {
                    if !spec.is_extra() {
                        parts.push(collapse(&text(spec, self.source)));
                    }
                }
            } else {
                parts.push(collapse(&text(child, self.source)));
            }
        }
        parts.join(", ")
    }
}

fn collapse(s: &str) -> String {
    s.split_whitespace().collect::<Vec<_>>().join(" ")
}

fn unquote(s: &str) -> String {
    s.trim_matches(|c| c == '"' || c == '\'' || c == '`')
        .to_string()
}

/// Names re-exported by `export { a, b as c } from "m"`, or `*`.
fn export_names(node: Node, source: &[u8]) -> String {
    let mut cursor = node.walk();
    let mut parts = Vec::new();
    for child in node.named_children(&mut cursor) {
        match child.kind() {
            "export_clause" => {
                let mut inner = child.walk();
                for spec in child.named_children(&mut inner) {
                    if !spec.is_extra() {
                        parts.push(collapse(&text(spec, source)));
                    }
                }
            }
            "namespace_export" => parts.push(collapse(&text(child, source))),
            _ => {}
        }
    }
    if parts.is_empty() {
        "*".to_string()
    } else {
        parts.join(", ")
    }
}

#[cfg(test)]
mod tests {
    use super::super::{extract as extract_any, parse, Import, Lang, Symbol};

    fn run(lang: Lang, src: &str) -> (Vec<Symbol>, Vec<Import>) {
        let parsed = parse(lang, src.as_bytes()).expect("parse");
        let out = extract_any(lang, &parsed, src.as_bytes());
        (out.symbols, out.imports)
    }

    fn find<'a>(symbols: &'a [Symbol], qualified: &str) -> &'a Symbol {
        symbols
            .iter()
            .find(|s| s.qualified_name == qualified)
            .unwrap_or_else(|| panic!("no symbol {qualified} in {symbols:#?}"))
    }

    #[test]
    fn every_declaration_kind_is_extracted_with_its_range_and_export_flag() {
        // Arrange
        let src = "\
export function top(a: number): number {
  return a;
}

class Point {
  x = 1;
  private secret(): void {}
  area(): number { return 0; }
  handler = () => {};
}

export interface Shape {
  area(): number;
  name: string;
}

type Id = number;

export enum Color { Red }

export const make = (n: number) => n + 1;

const LIMIT = 3;

namespace Geo {
  export function dist() {}
}

export default class Main {
  run() {}
}
";

        // Act
        let (symbols, _) = run(Lang::TypeScript, src);

        // Assert
        let kinds: Vec<(&str, &str, bool)> = symbols
            .iter()
            .map(|s| (s.qualified_name.as_str(), s.kind, s.exported))
            .collect();
        assert_eq!(
            kinds,
            vec![
                ("top", "function", true),
                ("Point", "class", false),
                ("Point.secret", "method", false),
                ("Point.area", "method", false),
                ("Point.handler", "method", false),
                ("Shape", "interface", true),
                ("Shape.area", "method", true),
                ("Id", "type_alias", false),
                ("Color", "enum", true),
                ("make", "function", true),
                ("LIMIT", "const", false),
                ("Geo", "namespace", false),
                ("Geo.dist", "function", true),
                ("Main", "class", true),
                ("Main.run", "method", true),
            ]
        );
        let top = find(&symbols, "top");
        assert_eq!((top.start_line, top.end_line), (1, 3));
        assert_eq!(
            top.signature.as_deref(),
            Some("export function top(a: number): number")
        );
        assert_eq!(
            find(&symbols, "make").signature.as_deref(),
            Some("export const make = (n: number) =>")
        );
        assert_eq!(
            find(&symbols, "Point.area").parent,
            symbols.iter().position(|s| s.name == "Point")
        );
        let point = find(&symbols, "Point");
        assert_eq!((point.start_line, point.end_line), (5, 10));
    }

    #[test]
    fn a_tsx_component_is_found() {
        // Arrange
        let src = "export function App() {\n  return <div>hi</div>;\n}\nconst Card = () => <b/>;\n";

        // Act
        let (symbols, _) = run(Lang::Tsx, src);

        // Assert
        assert_eq!(find(&symbols, "App").kind, "function");
        assert!(find(&symbols, "App").exported);
        assert_eq!(find(&symbols, "Card").kind, "function");
    }

    #[test]
    fn a_mjs_file_uses_the_javascript_grammar() {
        // Arrange
        let path = "lib/util.mjs";
        let src = "export class Box {\n  open() {}\n  #hidden() {}\n}\nexport function wrap() {}\n";

        // Act
        let lang = Lang::for_path(path).expect("language");
        let (symbols, _) = run(lang, src);

        // Assert
        assert_eq!(lang, Lang::JavaScript);
        assert_eq!(find(&symbols, "Box.open").kind, "method");
        assert!(find(&symbols, "Box.open").exported);
        assert!(!find(&symbols, "Box.#hidden").exported);
        assert!(find(&symbols, "wrap").exported);
    }

    #[test]
    fn imports_and_reexports_record_module_names_and_line() {
        // Arrange
        let src = "\
import fs from \"fs\";
import { a, b as c } from './mod';
import * as path from 'path';
import \"./side-effect\";
export { x } from \"./x\";
export * from \"./all\";
";

        // Act
        let (_, imports) = run(Lang::TypeScript, src);

        // Assert
        let got: Vec<(&str, Option<&str>, usize)> = imports
            .iter()
            .map(|i| (i.module.as_str(), i.names.as_deref(), i.line))
            .collect();
        assert_eq!(
            got,
            vec![
                ("fs", Some("fs"), 1),
                ("./mod", Some("a, b as c"), 2),
                ("path", Some("* as path"), 3),
                ("./side-effect", None, 4),
                ("./x", Some("x"), 5),
                ("./all", Some("*"), 6),
            ]
        );
    }

    #[test]
    fn ranges_are_right_after_multibyte_text() {
        // Arrange
        let src = "// caf\u{e9}\nexport function \u{fc}n\u{ef}() {}\n";

        // Act
        let (symbols, _) = run(Lang::TypeScript, src);

        // Assert
        let f = find(&symbols, "\u{fc}n\u{ef}");
        assert_eq!(f.start_line, 2);
        assert_eq!(
            &src[f.start_byte..f.end_byte],
            "export function \u{fc}n\u{ef}() {}"
        );
    }

    #[test]
    fn signatures_of_199_200_and_201_characters_come_back_cut_only_at_the_cap() {
        for (len, want_len, cut) in [
            (199usize, 199usize, false),
            (200, 200, false),
            (201, 200, true),
        ] {
            // Arrange: the signature is `function <name>()`, 11 characters past the name.
            let name = "a".repeat(len - 11);
            let src = format!("function {name}() {{}}\n");

            // Act
            let (symbols, _) = run(Lang::JavaScript, &src);

            // Assert
            let sig = symbols[0].signature.clone().expect("signature");
            assert_eq!(sig.chars().count(), want_len, "{len}");
            assert_eq!(symbols[0].signature_cut, cut, "{len}");
        }
    }

    #[test]
    fn the_shared_edge_cases_hold_for_typescript_tsx_and_javascript() {
        let broken = "function broken( {";
        let edge = super::super::edge_cases;
        edge(
            Lang::TypeScript,
            "function thing(): void {}",
            "thing",
            broken,
            "//",
        );
        edge(
            Lang::Tsx,
            "function thing() { return <b/>; }",
            "thing",
            broken,
            "//",
        );
        edge(
            Lang::JavaScript,
            "function thing() {}",
            "thing",
            broken,
            "//",
        );
    }
}
