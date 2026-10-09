// SPDX-FileCopyrightText: 2026 Igor Santos
// SPDX-License-Identifier: Apache-2.0

//! Renders the repo-scoped markdown slice of the graph-first memory store
//! that the retired shell original (now `playbook memory context`) built with
//! a `jq` filter. Ported
//! function-for-function against real `jq` output (see
//! `tests/json_memorycontext.rs`), including the empty-repo edge case
//! where jq's `"" | split("/")[0]` is `null` rather than `""`.

use serde::de::{self, Deserializer, IgnoredAny, MapAccess, SeqAccess, Visitor};
use serde::Deserialize;
use std::borrow::Cow;
use std::collections::{BTreeMap, HashSet};
use std::fmt;

/// A string field that tolerates any other JSON type by reading it as absent,
/// matching the old `Value` walk (`get(..).and_then(as_str)`). Borrows from
/// the input when the string has no escapes.
#[derive(Debug, Default, Clone)]
pub struct Str<'a>(Option<Cow<'a, str>>);

impl Str<'_> {
    pub fn as_deref(&self) -> Option<&str> {
        self.0.as_deref()
    }

    /// The string, or `""` when absent or not a string.
    pub fn or_empty(&self) -> &str {
        self.as_deref().unwrap_or("")
    }
}

struct StrVisitor<'a>(std::marker::PhantomData<&'a ()>);

impl<'de: 'a, 'a> Visitor<'de> for StrVisitor<'a> {
    type Value = Str<'a>;

    fn expecting(&self, f: &mut fmt::Formatter) -> fmt::Result {
        f.write_str("any JSON value")
    }
    fn visit_borrowed_str<E>(self, v: &'de str) -> Result<Self::Value, E> {
        Ok(Str(Some(Cow::Borrowed(v))))
    }
    fn visit_str<E>(self, v: &str) -> Result<Self::Value, E> {
        Ok(Str(Some(Cow::Owned(v.to_string()))))
    }
    fn visit_string<E>(self, v: String) -> Result<Self::Value, E> {
        Ok(Str(Some(Cow::Owned(v))))
    }
    fn visit_bool<E>(self, _: bool) -> Result<Self::Value, E> {
        Ok(Str(None))
    }
    fn visit_i64<E>(self, _: i64) -> Result<Self::Value, E> {
        Ok(Str(None))
    }
    fn visit_u64<E>(self, _: u64) -> Result<Self::Value, E> {
        Ok(Str(None))
    }
    fn visit_f64<E>(self, _: f64) -> Result<Self::Value, E> {
        Ok(Str(None))
    }
    fn visit_unit<E>(self) -> Result<Self::Value, E> {
        Ok(Str(None))
    }
    fn visit_none<E>(self) -> Result<Self::Value, E> {
        Ok(Str(None))
    }
    fn visit_seq<A: SeqAccess<'de>>(self, mut seq: A) -> Result<Self::Value, A::Error> {
        while seq.next_element::<IgnoredAny>()?.is_some() {}
        Ok(Str(None))
    }
    fn visit_map<A: MapAccess<'de>>(self, mut map: A) -> Result<Self::Value, A::Error> {
        while map.next_entry::<IgnoredAny, IgnoredAny>()?.is_some() {}
        Ok(Str(None))
    }
}

impl<'de: 'a, 'a> Deserialize<'de> for Str<'a> {
    fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        d.deserialize_any(StrVisitor(std::marker::PhantomData))
    }
}

/// A boolean field that reads anything but `true`/`false` as absent.
#[derive(Debug, Default, Clone, Copy)]
pub struct Flag(Option<bool>);

impl<'de> Deserialize<'de> for Flag {
    fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        struct V;
        impl<'de> Visitor<'de> for V {
            type Value = Flag;
            fn expecting(&self, f: &mut fmt::Formatter) -> fmt::Result {
                f.write_str("any JSON value")
            }
            fn visit_bool<E>(self, v: bool) -> Result<Flag, E> {
                Ok(Flag(Some(v)))
            }
            fn visit_str<E>(self, _: &str) -> Result<Flag, E> {
                Ok(Flag(None))
            }
            fn visit_i64<E>(self, _: i64) -> Result<Flag, E> {
                Ok(Flag(None))
            }
            fn visit_u64<E>(self, _: u64) -> Result<Flag, E> {
                Ok(Flag(None))
            }
            fn visit_f64<E>(self, _: f64) -> Result<Flag, E> {
                Ok(Flag(None))
            }
            fn visit_unit<E>(self) -> Result<Flag, E> {
                Ok(Flag(None))
            }
            fn visit_none<E>(self) -> Result<Flag, E> {
                Ok(Flag(None))
            }
            fn visit_seq<A: SeqAccess<'de>>(self, mut seq: A) -> Result<Flag, A::Error> {
                while seq.next_element::<IgnoredAny>()?.is_some() {}
                Ok(Flag(None))
            }
            fn visit_map<A: MapAccess<'de>>(self, mut map: A) -> Result<Flag, A::Error> {
                while map.next_entry::<IgnoredAny, IgnoredAny>()?.is_some() {}
                Ok(Flag(None))
            }
        }
        d.deserialize_any(V)
    }
}

impl Flag {
    pub fn is_true(self) -> bool {
        self.0 == Some(true)
    }
}

/// One memory node, only the fields session-start reads. A node that is not
/// an object deserializes to the default (no fields), as the `Value` walk
/// treated it.
#[derive(Debug, Default, Deserialize)]
#[serde(default)]
pub struct Node<'a> {
    #[serde(borrow)]
    pub id: Str<'a>,
    #[serde(borrow)]
    pub name: Str<'a>,
    #[serde(borrow)]
    pub description: Str<'a>,
    #[serde(borrow)]
    pub scope: Str<'a>,
    #[serde(borrow)]
    pub project: Str<'a>,
    #[serde(borrow)]
    pub file: Str<'a>,
    pub pinned: Flag,
}

/// One edge, only the fields the render reads.
#[derive(Debug, Default, Deserialize)]
#[serde(default)]
pub struct Edge<'a> {
    #[serde(borrow)]
    pub from: Str<'a>,
    #[serde(borrow)]
    pub to: Str<'a>,
    #[serde(borrow)]
    pub relation: Str<'a>,
}

/// An element that reads as `T::default()` when it is not a JSON object.
struct OrDefault<T>(T);

impl<'de, T: Deserialize<'de> + Default> Deserialize<'de> for OrDefault<T> {
    fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        struct V<T>(std::marker::PhantomData<T>);
        impl<'de, T: Deserialize<'de> + Default> Visitor<'de> for V<T> {
            type Value = OrDefault<T>;
            fn expecting(&self, f: &mut fmt::Formatter) -> fmt::Result {
                f.write_str("any JSON value")
            }
            fn visit_map<A: MapAccess<'de>>(self, map: A) -> Result<Self::Value, A::Error> {
                T::deserialize(de::value::MapAccessDeserializer::new(map)).map(OrDefault)
            }
            fn visit_seq<A: SeqAccess<'de>>(self, mut seq: A) -> Result<Self::Value, A::Error> {
                while seq.next_element::<IgnoredAny>()?.is_some() {}
                Ok(OrDefault(T::default()))
            }
            fn visit_bool<E>(self, _: bool) -> Result<Self::Value, E> {
                Ok(OrDefault(T::default()))
            }
            fn visit_i64<E>(self, _: i64) -> Result<Self::Value, E> {
                Ok(OrDefault(T::default()))
            }
            fn visit_u64<E>(self, _: u64) -> Result<Self::Value, E> {
                Ok(OrDefault(T::default()))
            }
            fn visit_f64<E>(self, _: f64) -> Result<Self::Value, E> {
                Ok(OrDefault(T::default()))
            }
            fn visit_str<E>(self, _: &str) -> Result<Self::Value, E> {
                Ok(OrDefault(T::default()))
            }
            fn visit_unit<E>(self) -> Result<Self::Value, E> {
                Ok(OrDefault(T::default()))
            }
        }
        d.deserialize_any(V(std::marker::PhantomData))
    }
}

/// A list that reads as empty when the value is not an array.
fn lenient_list<'de, D, T>(d: D) -> Result<Vec<T>, D::Error>
where
    D: Deserializer<'de>,
    T: Deserialize<'de> + Default,
{
    struct V<T>(std::marker::PhantomData<T>);
    impl<'de, T: Deserialize<'de> + Default> Visitor<'de> for V<T> {
        type Value = Vec<T>;
        fn expecting(&self, f: &mut fmt::Formatter) -> fmt::Result {
            f.write_str("any JSON value")
        }
        fn visit_seq<A: SeqAccess<'de>>(self, mut seq: A) -> Result<Vec<T>, A::Error> {
            let mut out = Vec::with_capacity(seq.size_hint().unwrap_or(0).min(65_536));
            while let Some(OrDefault(item)) = seq.next_element::<OrDefault<T>>()? {
                out.push(item);
            }
            Ok(out)
        }
        fn visit_map<A: MapAccess<'de>>(self, mut map: A) -> Result<Vec<T>, A::Error> {
            while map.next_entry::<IgnoredAny, IgnoredAny>()?.is_some() {}
            Ok(Vec::new())
        }
        fn visit_bool<E>(self, _: bool) -> Result<Vec<T>, E> {
            Ok(Vec::new())
        }
        fn visit_i64<E>(self, _: i64) -> Result<Vec<T>, E> {
            Ok(Vec::new())
        }
        fn visit_u64<E>(self, _: u64) -> Result<Vec<T>, E> {
            Ok(Vec::new())
        }
        fn visit_f64<E>(self, _: f64) -> Result<Vec<T>, E> {
            Ok(Vec::new())
        }
        fn visit_str<E>(self, _: &str) -> Result<Vec<T>, E> {
            Ok(Vec::new())
        }
        fn visit_unit<E>(self) -> Result<Vec<T>, E> {
            Ok(Vec::new())
        }
    }
    d.deserialize_any(V(std::marker::PhantomData))
}

/// The memory graph, parsed once into borrowed typed structs. `nodes` is
/// `None` when the top level has no `nodes` array, which renders as empty.
#[derive(Debug, Default, Deserialize)]
#[serde(default)]
pub struct Graph<'a> {
    #[serde(borrow, deserialize_with = "lenient_nodes")]
    pub nodes: Option<Vec<Node<'a>>>,
    #[serde(borrow, deserialize_with = "lenient_list")]
    pub edges: Vec<Edge<'a>>,
}

fn lenient_nodes<'de: 'a, 'a, D>(d: D) -> Result<Option<Vec<Node<'a>>>, D::Error>
where
    D: Deserializer<'de>,
{
    struct V<'a>(std::marker::PhantomData<&'a ()>);
    impl<'de: 'a, 'a> Visitor<'de> for V<'a> {
        type Value = Option<Vec<Node<'a>>>;
        fn expecting(&self, f: &mut fmt::Formatter) -> fmt::Result {
            f.write_str("any JSON value")
        }
        fn visit_seq<A: SeqAccess<'de>>(self, mut seq: A) -> Result<Self::Value, A::Error> {
            let mut out = Vec::with_capacity(seq.size_hint().unwrap_or(0).min(65_536));
            while let Some(OrDefault(item)) = seq.next_element::<OrDefault<Node<'a>>>()? {
                out.push(item);
            }
            Ok(Some(out))
        }
        fn visit_map<A: MapAccess<'de>>(self, mut map: A) -> Result<Self::Value, A::Error> {
            while map.next_entry::<IgnoredAny, IgnoredAny>()?.is_some() {}
            Ok(None)
        }
        fn visit_bool<E>(self, _: bool) -> Result<Self::Value, E> {
            Ok(None)
        }
        fn visit_i64<E>(self, _: i64) -> Result<Self::Value, E> {
            Ok(None)
        }
        fn visit_u64<E>(self, _: u64) -> Result<Self::Value, E> {
            Ok(None)
        }
        fn visit_f64<E>(self, _: f64) -> Result<Self::Value, E> {
            Ok(None)
        }
        fn visit_str<E>(self, _: &str) -> Result<Self::Value, E> {
            Ok(None)
        }
        fn visit_unit<E>(self) -> Result<Self::Value, E> {
            Ok(None)
        }
    }
    d.deserialize_any(V(std::marker::PhantomData))
}

/// Parses `graph_json` once. `None` on malformed JSON or a non-object top level.
pub fn parse_graph(graph_json: &str) -> Option<Graph<'_>> {
    serde_json::from_str(graph_json).ok()
}

/// Reads `graph` and renders it for `repo`; empty when the file is missing
/// or unreadable, so callers never break on absent memory.
pub fn render_for_graph_file(graph: &std::path::Path, repo: &str) -> String {
    match std::fs::read_to_string(graph) {
        Ok(json) => render_memory_context(&json, repo),
        Err(_) => String::new(),
    }
}

/// Renders `graph_json` (a `{"nodes": [...], "edges": [...]}` memory graph)
/// into the same `Facts:`/`Edges:`/`Anchors:` markdown text
/// the retired shell original's `jq` filter produced for `repo`, an
/// `owner/name` slug. Empty on malformed JSON or an unexpected shape rather
/// than panicking, so a caller that renders this into session context never
/// crashes on a corrupt graph file.
///
/// A node is in scope when its `scope` is `"global"`, or `"project"` with a
/// matching `project`, or `"org"` with a `project` matching `repo`'s owner
/// segment (the text before the first `/`). An empty `repo` never satisfies
/// the org branch, matching jq's `"" | split("/")[0]` evaluating to `null`
/// rather than `""`.
pub fn render_memory_context(graph_json: &str, repo: &str) -> String {
    match parse_graph(graph_json) {
        Some(graph) => render_graph(&graph, repo),
        None => String::new(),
    }
}

/// `render_memory_context` over an already parsed graph, so a caller that
/// needs the graph for more than one purpose parses it once.
pub fn render_graph(graph: &Graph<'_>, repo: &str) -> String {
    let Some(nodes) = graph.nodes.as_deref() else {
        return String::new();
    };
    let edges: &[Edge<'_>] = &graph.edges;

    // jq's `"" | split("/")` is `[]`, so indexing `[0]` is `null`: an empty
    // repo must never match the org branch, even against an equally empty
    // `project` field.
    let org = (!repo.is_empty()).then(|| repo.split('/').next().unwrap_or(repo));

    let by_id: BTreeMap<&str, &Node<'_>> = nodes
        .iter()
        .filter_map(|n| n.id.as_deref().map(|id| (id, n)))
        .collect();

    let in_scope = |node: &Node<'_>| -> bool {
        let project = node.project.as_deref();
        match node.scope.as_deref() {
            Some("global") => true,
            Some("project") => project == Some(repo),
            Some("org") => org.is_some() && project == org,
            _ => false,
        }
    };

    let mut facts: Vec<&Node<'_>> = nodes.iter().filter(|n| in_scope(n)).collect();
    facts.sort_by_key(|n| n.name.or_empty());

    let in_scope_ids: HashSet<&str> = facts.iter().filter_map(|n| n.id.as_deref()).collect();

    let facts_block = facts
        .iter()
        .map(|n| format!("{}: {}", n.name.or_empty(), n.description.or_empty()))
        .collect::<Vec<_>>()
        .join("\n");

    let mut relation_edges: Vec<(&str, &str, &str)> = edges
        .iter()
        .filter(|e| e.relation.or_empty() != "anchors")
        .filter_map(|e| {
            let from = e.from.as_deref()?;
            let to = e.to.as_deref()?;
            (in_scope_ids.contains(from) && in_scope_ids.contains(to)).then_some((
                e.relation.or_empty(),
                from,
                to,
            ))
        })
        .collect();
    relation_edges.sort();

    let edges_block = relation_edges
        .iter()
        .map(|(relation, from, to)| {
            let from_name = by_id.get(from).map_or("", |n| n.name.or_empty());
            let to_name = by_id.get(to).map_or("", |n| n.name.or_empty());
            format!("{from_name} {relation} {to_name}")
        })
        .collect::<Vec<_>>()
        .join("\n");

    // Group anchor edges by their target id first (an intermediate step, not
    // the final render order); each group's own path drives the eventual
    // sort below.
    let mut anchors_by_to: BTreeMap<&str, Vec<&str>> = BTreeMap::new();
    for e in edges {
        if e.relation.or_empty() != "anchors" {
            continue;
        }
        let Some(from) = e.from.as_deref() else {
            continue;
        };
        let Some(to) = e.to.as_deref() else {
            continue;
        };
        if !in_scope_ids.contains(from) {
            continue;
        }
        let from_name = by_id.get(from).map_or("", |n| n.name.or_empty());
        anchors_by_to.entry(to).or_default().push(from_name);
    }

    let mut anchor_rows: Vec<(&str, String)> = anchors_by_to
        .into_iter()
        .map(|(to, mut names)| {
            names.sort();
            let path = by_id.get(to).map_or("", |n| n.file.or_empty());
            (path, names.join(", "))
        })
        .collect();
    anchor_rows.sort_by_key(|(path, _)| *path);

    let anchors_block = anchor_rows
        .iter()
        .map(|(path, names)| format!("{path}: {names}"))
        .collect::<Vec<_>>()
        .join("\n");

    let mut blocks = Vec::new();
    if !facts_block.is_empty() {
        blocks.push(format!("Facts:\n{facts_block}"));
    }
    if !edges_block.is_empty() {
        blocks.push(format!("Edges:\n{edges_block}"));
    }
    if !anchors_block.is_empty() {
        blocks.push(format!("Anchors:\n{anchors_block}"));
    }
    blocks.join("\n\n")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn facts_in_scope_across_all_five_scope_combinations() {
        // Arrange: global always in; project matching/mismatching $repo;
        // org matching/mismatching $repo's owner segment.
        let repo = "acme/proj";
        let graph = r#"{
            "nodes": [
                {"id": "n-charlie", "scope": "org", "project": "acme", "type": "reference", "name": "charlie", "description": "org fact in scope"},
                {"id": "n-alpha", "scope": "global", "type": "reference", "name": "alpha", "description": "global fact"},
                {"id": "n-delta", "scope": "project", "project": "other/proj", "type": "reference", "name": "delta", "description": "different project"},
                {"id": "n-bravo", "scope": "project", "project": "acme/proj", "type": "reference", "name": "bravo", "description": "project fact in scope"},
                {"id": "n-echo", "scope": "org", "project": "other", "type": "reference", "name": "echo", "description": "different org"}
            ],
            "edges": []
        }"#;

        // Act
        let got = render_memory_context(graph, repo);

        // Assert: only the three in-scope facts render, sorted by name.
        assert_eq!(
            got,
            "Facts:\nalpha: global fact\nbravo: project fact in scope\ncharlie: org fact in scope"
        );
    }

    #[test]
    fn edges_block_orders_rows_by_node_id_not_name() {
        // Arrange: ids deliberately out of alphabetical sync with names.
        let repo = "acme/proj";
        let graph = r#"{
            "nodes": [
                {"id": "z1", "scope": "global", "type": "reference", "name": "alpha", "description": "d-alpha"},
                {"id": "a1", "scope": "global", "type": "reference", "name": "zulu", "description": "d-zulu"},
                {"id": "m1", "scope": "global", "type": "reference", "name": "mike", "description": "d-mike"}
            ],
            "edges": [
                {"from": "z1", "to": "a1", "relation": "relates_to"},
                {"from": "a1", "to": "m1", "relation": "relates_to"}
            ]
        }"#;

        // Act
        let got = render_memory_context(graph, repo);

        // Assert: row order follows id order (a1 before z1), row text uses names.
        assert_eq!(
            got,
            "Facts:\nalpha: d-alpha\nmike: d-mike\nzulu: d-zulu\n\nEdges:\nzulu relates_to mike\nalpha relates_to zulu"
        );
    }

    #[test]
    fn anchors_block_groups_multiple_facts_anchored_to_the_same_file() {
        // Arrange: two facts anchor to the same code node.
        let repo = "acme/proj";
        let graph = r#"{
            "nodes": [
                {"id": "f1", "scope": "global", "type": "reference", "name": "factB", "description": "descB"},
                {"id": "f2", "scope": "global", "type": "reference", "name": "factA", "description": "descA"},
                {"id": "code1", "file": "src/example.rs", "scope": "code", "type": "code"}
            ],
            "edges": [
                {"from": "f1", "to": "code1", "relation": "anchors"},
                {"from": "f2", "to": "code1", "relation": "anchors"}
            ]
        }"#;

        // Act
        let got = render_memory_context(graph, repo);

        // Assert: both fact names appear for the one path, sorted, comma-joined.
        assert_eq!(
            got,
            "Facts:\nfactA: descA\nfactB: descB\n\nAnchors:\nsrc/example.rs: factA, factB"
        );
    }

    #[test]
    fn anchors_block_resolves_target_file_even_when_code_node_is_out_of_scope() {
        // Arrange: the anchored code node's own scope/project don't match
        // the queried repo/org at all; only the fact's scope gates anchors.
        let repo = "acme/proj";
        let graph = r#"{
            "nodes": [
                {"id": "fact1", "scope": "global", "type": "reference", "name": "fact1", "description": "d1"},
                {"id": "code-out", "file": "lib/foo.rs", "scope": "code", "type": "code", "project": "someoneelse/repo"}
            ],
            "edges": [
                {"from": "fact1", "to": "code-out", "relation": "anchors"}
            ]
        }"#;

        // Act
        let got = render_memory_context(graph, repo);

        // Assert: the path still resolves via $byid built from all nodes.
        assert_eq!(got, "Facts:\nfact1: d1\n\nAnchors:\nlib/foo.rs: fact1");
    }

    #[test]
    fn facts_block_is_omitted_when_no_facts_are_in_scope() {
        // Arrange: the only node is out of scope; a self-edge referencing
        // it is present but can never qualify since its endpoint is not
        // in scope either.
        let repo = "acme/proj";
        let graph = r#"{
            "nodes": [
                {"id": "p1", "scope": "project", "project": "someone/else", "type": "reference", "name": "outproj", "description": "d"}
            ],
            "edges": [
                {"from": "p1", "to": "p1", "relation": "relates_to"}
            ]
        }"#;

        // Act
        let got = render_memory_context(graph, repo);

        // Assert: no "Facts:" header at all, not an empty one.
        assert_eq!(got, "");
        assert!(!got.contains("Facts:"));
    }

    #[test]
    fn edges_and_anchors_headers_are_omitted_when_nothing_qualifies() {
        // Arrange: one in-scope fact, plus edges present in the array but
        // neither qualifies (their "from" is out of scope).
        let repo = "acme/proj";
        let graph = r#"{
            "nodes": [
                {"id": "solo", "scope": "global", "type": "reference", "name": "solo", "description": "only fact"},
                {"id": "other", "scope": "project", "project": "different/repo", "type": "reference", "name": "other", "description": "out of scope"},
                {"id": "code1", "file": "a.rs", "scope": "code", "type": "code"}
            ],
            "edges": [
                {"from": "other", "to": "solo", "relation": "relates_to"},
                {"from": "other", "to": "code1", "relation": "anchors"}
            ]
        }"#;

        // Act
        let got = render_memory_context(graph, repo);

        // Assert: Facts renders alone, no blank-line separators trailing it.
        assert_eq!(got, "Facts:\nsolo: only fact");
    }

    #[test]
    fn org_scope_does_not_match_empty_project_against_an_empty_repo() {
        // Arrange: repo is unresolved (empty). An org-scoped node with an
        // empty project string must not match, since real jq's
        // `"" | split("/")[0]` is null, not "", so `.project == null` is
        // false for a `.project` of "".
        let repo = "";
        let graph = r#"{
            "nodes": [
                {"id": "n1", "scope": "org", "project": "", "type": "reference", "name": "orgempty", "description": "d"},
                {"id": "g1", "scope": "global", "type": "reference", "name": "globalone", "description": "gd"}
            ],
            "edges": []
        }"#;

        // Act
        let got = render_memory_context(graph, repo);

        // Assert: only the global fact renders, the org-empty fact is excluded.
        assert_eq!(got, "Facts:\nglobalone: gd");
    }

    #[test]
    fn output_is_empty_string_when_graph_has_no_nodes_or_edges() {
        // Arrange
        let repo = "acme/proj";
        let graph = r#"{"nodes": [], "edges": []}"#;

        // Act
        let got = render_memory_context(graph, repo);

        // Assert
        assert_eq!(got, "");
    }

    #[test]
    fn odd_shapes_read_as_absent_exactly_like_the_value_walk() {
        // Non-string fields, a non-object node, an escaped string and a
        // non-array edges value must not fail the whole render.
        let graph = r#"{
            "nodes": [
                7,
                {"id": "n1", "scope": "global", "name": "alpha", "description": "line \"one\"\nline two"},
                {"id": 5, "scope": "global", "name": "beta", "description": "numeric id"},
                {"id": "n3", "scope": "global", "name": 9, "description": null}
            ],
            "edges": {"not": "a list"}
        }"#;
        let got = render_memory_context(graph, "acme/proj");
        assert_eq!(
            got,
            "Facts:\n: \nalpha: line \"one\"\nline two\nbeta: numeric id"
        );
    }

    #[test]
    fn a_missing_or_wrong_nodes_value_renders_empty() {
        assert_eq!(render_memory_context(r#"{"edges": []}"#, "a/b"), "");
        assert_eq!(render_memory_context(r#"{"nodes": "x"}"#, "a/b"), "");
        assert_eq!(render_memory_context("not json", "a/b"), "");
    }
}
