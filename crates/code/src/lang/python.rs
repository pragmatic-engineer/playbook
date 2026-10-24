// SPDX-FileCopyrightText: 2026 Igor Santos
// SPDX-License-Identifier: Apache-2.0

//! Python symbol and import extraction.

use super::{clean_signature, join_dotted, Extracted, Import, Symbol, MAX_SIGNATURE_CHARS};
use std::borrow::Cow;
use tree_sitter::Node;

/// Nesting deeper than this (defs inside defs) is not followed.
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
    walker.block(root, "", None, Owner::Module, 0);
    walker.out
}

/// What encloses a statement list.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Owner {
    Module,
    Class,
    Function,
}

/// Compound statements whose bodies stay in the enclosing scope.
fn is_transparent(kind: &str) -> bool {
    matches!(
        kind,
        "if_statement"
            | "elif_clause"
            | "else_clause"
            | "try_statement"
            | "except_clause"
            | "except_group_clause"
            | "finally_clause"
            | "with_statement"
            | "for_statement"
            | "while_statement"
            | "match_statement"
            | "case_clause"
            | "block"
    )
}

/// Identifier targets of an assignment's left side.
fn collect_identifiers<'t>(node: Node<'t>, out: &mut Vec<Node<'t>>, depth: usize) {
    if depth > MAX_DEPTH {
        return;
    }
    match node.kind() {
        "identifier" => out.push(node),
        "pattern_list" | "tuple_pattern" | "list_pattern" => {
            let mut cursor = node.walk();
            for child in node.named_children(&mut cursor) {
                collect_identifiers(child, out, depth + 1);
            }
        }
        _ => {}
    }
}

impl Walker<'_> {
    fn block(
        &mut self,
        parent_node: Node,
        prefix: &str,
        parent: Option<usize>,
        owner: Owner,
        depth: usize,
    ) {
        if depth > MAX_DEPTH {
            return;
        }
        let mut cursor = parent_node.walk();
        let children: Vec<Node> = parent_node.named_children(&mut cursor).collect();
        for node in children {
            self.statement(node, prefix, parent, owner, depth);
        }
    }

    fn statement(
        &mut self,
        node: Node,
        prefix: &str,
        parent: Option<usize>,
        owner: Owner,
        depth: usize,
    ) {
        match node.kind() {
            "function_definition" => self.definition(node, node, prefix, parent, owner, depth),
            "class_definition" => self.definition(node, node, prefix, parent, owner, depth),
            "decorated_definition" => {
                if let Some(def) = node.child_by_field_name("definition") {
                    self.definition(def, node, prefix, parent, owner, depth);
                }
            }
            "import_statement" => self.import_statement(node),
            "import_from_statement" => self.import_from(node),
            "future_import_statement" => self.future_import(node),
            "type_alias_statement" => self.type_alias(node, prefix, parent),
            "expression_statement" => {
                if owner != Owner::Function {
                    self.assignment(node, prefix, parent);
                }
            }
            kind if is_transparent(kind) => self.block(node, prefix, parent, owner, depth),
            _ => {}
        }
    }

    /// A def or class. `outer` is the decorated wrapper when there is one, so
    /// the range and signature include the decorators.
    fn definition(
        &mut self,
        def: Node,
        outer: Node,
        prefix: &str,
        parent: Option<usize>,
        owner: Owner,
        depth: usize,
    ) {
        let is_class = def.kind() == "class_definition";
        let kind = match (is_class, owner) {
            (true, _) => "class",
            (false, Owner::Class) => "method",
            (false, _) => "function",
        };
        // The head ends at the colon, so a comment after it stays out.
        let head_end = def.child_by_field_name("body").map_or(def.end_byte(), |b| {
            def.children(&mut def.walk())
                .filter(|c| c.kind() == ":" && c.end_byte() <= b.start_byte())
                .last()
                .map_or(b.start_byte(), |c| c.end_byte())
        });
        let Some(index) = self.symbol(def, outer, def.start_byte(), kind, head_end, prefix, parent)
        else {
            return;
        };
        if let Some(body) = def.child_by_field_name("body") {
            let name = self.out.symbols[index].qualified_name.clone();
            let inner = if is_class {
                Owner::Class
            } else {
                Owner::Function
            };
            self.block(body, &name, Some(index), inner, depth + 1);
        }
    }

    /// `NAME = value` at module or class level, including tuple targets and
    /// chained assignments.
    fn assignment(&mut self, stmt: Node, prefix: &str, parent: Option<usize>) {
        let mut cursor = stmt.walk();
        let found = stmt
            .named_children(&mut cursor)
            .find(|n| n.kind() == "assignment");
        let mut next = found;
        let mut hops = 0;
        while let Some(assign) = next {
            hops += 1;
            if hops > MAX_DEPTH {
                return;
            }
            let right = assign.child_by_field_name("right");
            let is_lambda = right.is_some_and(|r| r.kind() == "lambda");
            let kind = if is_lambda { "function" } else { "variable" };
            if let Some(left) = assign.child_by_field_name("left") {
                let mut targets = Vec::new();
                collect_identifiers(left, &mut targets, 0);
                for target in targets {
                    self.symbol_at(
                        target,
                        stmt,
                        stmt.start_byte(),
                        kind,
                        stmt.end_byte(),
                        prefix,
                        parent,
                    );
                }
            }
            next = right.filter(|r| r.kind() == "assignment");
        }
    }

    /// `type Name = value`
    fn type_alias(&mut self, node: Node, prefix: &str, parent: Option<usize>) {
        let Some(left) = node.child_by_field_name("left") else {
            return;
        };
        let named = left.named_child(0).unwrap_or(left);
        let name = if named.kind() == "generic_type" {
            named.named_child(0).unwrap_or(named)
        } else {
            named
        };
        self.symbol_at(
            name,
            node,
            node.start_byte(),
            "type_alias",
            node.end_byte(),
            prefix,
            parent,
        );
    }

    #[allow(clippy::too_many_arguments)]
    fn symbol(
        &mut self,
        def: Node,
        outer: Node,
        sig_start: usize,
        kind: &'static str,
        head_end: usize,
        prefix: &str,
        parent: Option<usize>,
    ) -> Option<usize> {
        let name = def.child_by_field_name("name")?;
        self.symbol_at(name, outer, sig_start, kind, head_end, prefix, parent)
    }

    #[allow(clippy::too_many_arguments)]
    fn symbol_at(
        &mut self,
        name_node: Node,
        outer: Node,
        sig_start: usize,
        kind: &'static str,
        head_end: usize,
        prefix: &str,
        parent: Option<usize>,
    ) -> Option<usize> {
        let name = text(name_node, self.source).to_string();
        if name.is_empty() {
            return None;
        }
        let start = outer.start_byte();
        // Only the first characters are kept, so a huge statement is not decoded whole.
        let wanted = head_end.min(self.source.len());
        let end = wanted.min(sig_start.saturating_add(MAX_SIGNATURE_CHARS * 4 + 4));
        let raw = String::from_utf8_lossy(&self.source[sig_start.min(end)..end]);
        let (signature, mut signature_cut) =
            clean_signature(raw.trim_end_matches(|c: char| c == ':' || c.is_whitespace()));
        signature_cut |= end < wanted;
        let exported = !name.starts_with('_');
        self.out.symbols.push(Symbol {
            qualified_name: join_dotted(prefix, &name),
            name,
            kind,
            signature: Some(signature),
            signature_cut,
            start_byte: start,
            end_byte: outer.end_byte(),
            start_line: outer.start_position().row + 1,
            end_line: outer.end_position().row + 1,
            parent,
            exported,
        });
        Some(self.out.symbols.len() - 1)
    }

    fn import_statement(&mut self, node: Node) {
        let line = node.start_position().row + 1;
        let mut cursor = node.walk();
        let names: Vec<Node> = node.children_by_field_name("name", &mut cursor).collect();
        for name in names {
            let (module, alias) = match name.kind() {
                "aliased_import" => (
                    name.child_by_field_name("name"),
                    name.child_by_field_name("alias"),
                ),
                _ => (Some(name), None),
            };
            let Some(module) = module else {
                continue;
            };
            self.out.imports.push(Import {
                module: text(module, self.source).to_string(),
                names: alias.map(|a| format!("as {}", text(a, self.source))),
                line,
            });
        }
    }

    /// `from __future__ import annotations`
    fn future_import(&mut self, node: Node) {
        let mut cursor = node.walk();
        let names: Vec<String> = node
            .children_by_field_name("name", &mut cursor)
            .map(|n| {
                text(n, self.source)
                    .split_whitespace()
                    .collect::<Vec<_>>()
                    .join(" ")
            })
            .collect();
        self.out.imports.push(Import {
            module: "__future__".to_string(),
            names: Some(names.join(", ")),
            line: node.start_position().row + 1,
        });
    }

    fn import_from(&mut self, node: Node) {
        let line = node.start_position().row + 1;
        let module = node
            .child_by_field_name("module_name")
            .map(|m| text(m, self.source).to_string())
            .unwrap_or_default();
        let mut cursor = node.walk();
        let mut parts: Vec<String> = node
            .children_by_field_name("name", &mut cursor)
            .map(|n| {
                let raw = text(n, self.source);
                raw.split_whitespace().collect::<Vec<_>>().join(" ")
            })
            .collect();
        let mut inner = node.walk();
        if node
            .named_children(&mut inner)
            .any(|n| n.kind() == "wildcard_import")
        {
            parts.push("*".to_string());
        }
        self.out.imports.push(Import {
            module,
            names: if parts.is_empty() {
                None
            } else {
                Some(parts.join(", "))
            },
            line,
        });
    }
}

#[cfg(test)]
mod tests {
    use super::super::{extract as extract_any, find, parse, Import, Lang, Symbol};

    fn run(src: &str) -> (Vec<Symbol>, Vec<Import>) {
        let parsed = parse(Lang::Python, src.as_bytes()).expect("parse");
        let out = extract_any(Lang::Python, &parsed, src.as_bytes());
        (out.symbols, out.imports)
    }

    #[test]
    fn every_declaration_kind_is_extracted_with_its_range_and_export_flag() {
        // Arrange
        let src = "\
LIMIT = 3
_cache: dict = {}
square = lambda x: x * x

def top(a, b=1):
    def inner():
        pass
    return a

class Point(Base):
    scale = 2

    def area(self):
        return 0

    async def load(self):
        pass

    @staticmethod
    def make():
        pass

@decorate
def wrapped():
    pass
";

        // Act
        let (symbols, _) = run(src);

        // Assert
        let kinds: Vec<(&str, &str, bool)> = symbols
            .iter()
            .map(|s| (s.qualified_name.as_str(), s.kind, s.exported))
            .collect();
        assert_eq!(
            kinds,
            vec![
                ("LIMIT", "variable", true),
                ("_cache", "variable", false),
                ("square", "function", true),
                ("top", "function", true),
                ("top.inner", "function", true),
                ("Point", "class", true),
                ("Point.scale", "variable", true),
                ("Point.area", "method", true),
                ("Point.load", "method", true),
                ("Point.make", "method", true),
                ("wrapped", "function", true),
            ]
        );
        let top = find(&symbols, "top");
        assert_eq!((top.start_line, top.end_line), (5, 8));
        assert_eq!(top.signature.as_deref(), Some("def top(a, b=1)"));
        assert_eq!(
            find(&symbols, "Point").signature.as_deref(),
            Some("class Point(Base)")
        );
        assert_eq!(
            find(&symbols, "top.inner").parent,
            symbols.iter().position(|s| s.name == "top")
        );
    }

    #[test]
    fn decorators_are_part_of_the_range_and_signature() {
        // Arrange
        let src = "x = 1\n\n@decorate\n@other(1)\ndef wrapped():\n    pass\n";

        // Act
        let (symbols, _) = run(src);

        // Assert
        let w = find(&symbols, "wrapped");
        assert_eq!((w.start_line, w.end_line), (3, 6));
        assert_eq!(w.signature.as_deref(), Some("def wrapped()"));
        assert!(src[w.start_byte..w.end_byte].starts_with("@decorate"));
    }

    #[test]
    fn definitions_inside_if_and_try_blocks_stay_in_the_enclosing_scope() {
        // Arrange
        let src = "\
if True:
    def a():
        pass
else:
    def b():
        pass
try:
    import fast
except ImportError:
    def c():
        pass
";

        // Act
        let (symbols, imports) = run(src);

        // Assert
        for want in ["a", "b", "c"] {
            assert_eq!(find(&symbols, want).kind, "function", "{want}");
        }
        assert_eq!(imports.len(), 1);
    }

    #[test]
    fn newer_syntax_and_assignment_targets_are_extracted() {
        // Arrange
        let src = "\
from __future__ import annotations
type Vector = list[float]
a, b = 1, 2
X = Y = 0
class Foo(Base):  # noqa: E501
    pass
match command:
    case 1:
        def in_case():
            pass
";

        // Act
        let (symbols, imports) = run(src);

        // Assert
        assert_eq!(find(&symbols, "Vector").kind, "type_alias");
        for want in ["a", "b", "X", "Y"] {
            assert_eq!(find(&symbols, want).kind, "variable", "{want}");
        }
        assert_eq!(
            find(&symbols, "Foo").signature.as_deref(),
            Some("class Foo(Base)")
        );
        assert_eq!(find(&symbols, "in_case").kind, "function");
        assert_eq!(imports[0].module, "__future__");
        assert_eq!(imports[0].names.as_deref(), Some("annotations"));
    }

    #[test]
    fn a_long_aligned_signature_is_flagged_cut_even_when_it_collapses_short() {
        // Arrange: each parameter sits behind 100 spaces, so the head passes 804 bytes.
        let params: Vec<String> = (0..12)
            .map(|i| format!("{}p{i}", " ".repeat(100)))
            .collect();
        let src = format!("def f(\n{}):\n    pass\n", params.join(",\n"));

        // Act
        let (symbols, _) = run(&src);

        // Assert
        assert!(symbols[0].signature_cut);
    }

    #[test]
    fn assignments_inside_functions_are_not_symbols() {
        // Arrange
        let src = "def f():\n    local = 1\n    return local\n";

        // Act
        let (symbols, _) = run(src);

        // Assert
        let names: Vec<&str> = symbols.iter().map(|s| s.name.as_str()).collect();
        assert_eq!(names, vec!["f"]);
    }

    #[test]
    fn imports_record_module_names_and_line_for_every_form() {
        // Arrange
        let src = "\
import os
import numpy as np, sys
from collections import OrderedDict, defaultdict as dd
from . import sibling
from ..pkg import (a, b)
from mod import *
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
                ("os", None, 1),
                ("numpy", Some("as np"), 2),
                ("sys", None, 2),
                ("collections", Some("OrderedDict, defaultdict as dd"), 3),
                (".", Some("sibling"), 4),
                ("..pkg", Some("a, b"), 5),
                ("mod", Some("*"), 6),
            ]
        );
    }

    #[test]
    fn ranges_are_right_after_multibyte_text() {
        // Arrange
        let src = "# caf\u{e9}\ndef \u{fc}n\u{ef}():\n    pass\n";

        // Act
        let (symbols, _) = run(src);

        // Assert
        let f = find(&symbols, "\u{fc}n\u{ef}");
        assert_eq!(f.start_line, 2);
        assert_eq!(
            &src[f.start_byte..f.end_byte],
            "def \u{fc}n\u{ef}():\n    pass"
        );
    }

    #[test]
    fn signatures_of_199_200_and_201_characters_come_back_cut_only_at_the_cap() {
        for (len, want_len, cut) in [
            (199usize, 199usize, false),
            (200, 200, false),
            (201, 200, true),
        ] {
            // Arrange: the signature is `def <name>()`, 6 characters past the name.
            let name = "a".repeat(len - 6);
            let src = format!("def {name}():\n    pass\n");

            // Act
            let (symbols, _) = run(&src);

            // Assert
            let sig = symbols[0].signature.clone().expect("signature");
            assert_eq!(sig.chars().count(), want_len, "{len}");
            assert_eq!(symbols[0].signature_cut, cut, "{len}");
        }
    }

    #[test]
    fn the_shared_edge_cases_hold_for_python() {
        super::super::edge_cases(
            Lang::Python,
            "def thing():\n    pass",
            "thing",
            "def broken(:",
            "#",
        );
    }
}
