// SPDX-FileCopyrightText: 2026 Igor Santos
// SPDX-License-Identifier: Apache-2.0

//! Renders the repo-scoped markdown slice of the graph-first memory store
//! that the retired shell original (now `playbook memory context`) built with
//! a `jq` filter. Ported
//! function-for-function against real `jq` output (see
//! `tests/json_memorycontext.rs`), including the empty-repo edge case
//! where jq's `"" | split("/")[0]` is `null` rather than `""`.

use serde_json::Value;
use std::collections::{BTreeMap, HashSet};

/// Reads `graph` and renders it for `repo`; empty when the file is missing
/// or unreadable, so callers never break on absent memory.
pub fn render_for_graph_file(graph: &std::path::Path, repo: &str) -> String {
    match std::fs::read_to_string(graph) {
        Ok(json) => render_memory_context(&json, repo),
        Err(_) => String::new(),
    }
}

/// A node's string field, or `""` when absent or not a string, matching this
/// port's established swallow-the-mismatch behavior (`src/doctor/field.rs`).
fn str_field<'a>(node: &'a Value, key: &str) -> &'a str {
    node.get(key).and_then(Value::as_str).unwrap_or("")
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
    let Ok(value) = serde_json::from_str::<Value>(graph_json) else {
        return String::new();
    };
    let Some(nodes) = value.get("nodes").and_then(Value::as_array) else {
        return String::new();
    };
    let edges: &[Value] = value
        .get("edges")
        .and_then(Value::as_array)
        .map(Vec::as_slice)
        .unwrap_or(&[]);

    // jq's `"" | split("/")` is `[]`, so indexing `[0]` is `null`: an empty
    // repo must never match the org branch, even against an equally empty
    // `project` field.
    let org = (!repo.is_empty()).then(|| repo.split('/').next().unwrap_or(repo));

    let by_id: BTreeMap<&str, &Value> = nodes
        .iter()
        .filter_map(|n| n.get("id").and_then(Value::as_str).map(|id| (id, n)))
        .collect();

    let in_scope = |node: &Value| -> bool {
        let project = node.get("project").and_then(Value::as_str);
        match node.get("scope").and_then(Value::as_str) {
            Some("global") => true,
            Some("project") => project == Some(repo),
            Some("org") => org.is_some() && project == org,
            _ => false,
        }
    };

    let mut facts: Vec<&Value> = nodes.iter().filter(|n| in_scope(n)).collect();
    facts.sort_by_key(|n| str_field(n, "name"));

    let in_scope_ids: HashSet<&str> = facts
        .iter()
        .filter_map(|n| n.get("id").and_then(Value::as_str))
        .collect();

    let facts_block = facts
        .iter()
        .map(|n| format!("{}: {}", str_field(n, "name"), str_field(n, "description")))
        .collect::<Vec<_>>()
        .join("\n");

    let mut relation_edges: Vec<(&str, &str, &str)> = edges
        .iter()
        .filter(|e| str_field(e, "relation") != "anchors")
        .filter_map(|e| {
            let from = e.get("from").and_then(Value::as_str)?;
            let to = e.get("to").and_then(Value::as_str)?;
            (in_scope_ids.contains(from) && in_scope_ids.contains(to)).then_some((
                str_field(e, "relation"),
                from,
                to,
            ))
        })
        .collect();
    relation_edges.sort();

    let edges_block = relation_edges
        .iter()
        .map(|(relation, from, to)| {
            let from_name = by_id.get(from).map_or("", |n| str_field(n, "name"));
            let to_name = by_id.get(to).map_or("", |n| str_field(n, "name"));
            format!("{from_name} {relation} {to_name}")
        })
        .collect::<Vec<_>>()
        .join("\n");

    // Group anchor edges by their target id first (an intermediate step, not
    // the final render order); each group's own path drives the eventual
    // sort below.
    let mut anchors_by_to: BTreeMap<&str, Vec<&str>> = BTreeMap::new();
    for e in edges {
        if str_field(e, "relation") != "anchors" {
            continue;
        }
        let Some(from) = e.get("from").and_then(Value::as_str) else {
            continue;
        };
        let Some(to) = e.get("to").and_then(Value::as_str) else {
            continue;
        };
        if !in_scope_ids.contains(from) {
            continue;
        }
        let from_name = by_id.get(from).map_or("", |n| str_field(n, "name"));
        anchors_by_to.entry(to).or_default().push(from_name);
    }

    let mut anchor_rows: Vec<(&str, String)> = anchors_by_to
        .into_iter()
        .map(|(to, mut names)| {
            names.sort();
            let path = by_id.get(to).map_or("", |n| str_field(n, "file"));
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
}
