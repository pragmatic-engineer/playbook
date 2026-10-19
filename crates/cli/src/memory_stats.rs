// SPDX-FileCopyrightText: 2026 Igor Santos
// SPDX-License-Identifier: Apache-2.0

//! `playbook memory stats`: a read-only summary of the memory graph in
//! `~/.config/playbook/data/memory.db`, so nobody needs `sqlite3` to count
//! facts, nodes and edges. `playbook memory rebuild` prints the same summary.

use playbook_data::memory::{read_graph, touched_at, EdgeRow, NodeRow};
use serde_json::{json, Value};
use std::collections::{BTreeMap, HashSet};
use std::path::{Path, PathBuf};

/// The fact scopes, in the order they are printed.
const SCOPES: [&str; 3] = ["global", "org", "project"];

/// The scope of a code location node. Every other node is a fact.
const CODE_SCOPE: &str = "code";

/// What `playbook memory stats` reports.
#[derive(Debug, Default, PartialEq, Eq)]
pub struct Stats {
    /// The database file.
    pub db: PathBuf,
    /// Whether the database holds a graph.
    pub present: bool,
    /// Size of the database file in bytes.
    pub size_bytes: u64,
    /// When the graph was last rebuilt, in unix seconds.
    pub rebuilt_at: Option<i64>,
    /// Fact count per scope (`global`, `org`, `project`).
    pub facts: BTreeMap<String, usize>,
    /// Every node: facts and code locations.
    pub nodes: usize,
    /// Code location nodes (the targets of `anchors:`).
    pub anchors: usize,
    /// Edge count per relation.
    pub edges: BTreeMap<String, usize>,
    /// Edges whose target node is missing, as `(from, to, relation)`.
    pub dangling_edges: Vec<(String, String, String)>,
    /// The repo whose anchors were checked on disk, and its checkout.
    pub anchor_repo: Option<(String, PathBuf)>,
    /// Anchors of `anchor_repo` that were checked on disk.
    pub anchors_checked: usize,
    /// Anchors of `anchor_repo` whose path is missing on disk.
    pub dangling_anchors: Vec<String>,
}

/// Reads the graph of the store under `home` and summarizes it. `repo` is the
/// slug and checkout whose anchors are checked on disk; anchors of other repos
/// cannot be checked, because only this checkout's location is known.
pub fn collect(home: &Path, repo: Option<(String, PathBuf)>) -> Result<Stats, String> {
    let db = playbook_data::memory::db_in(&playbook_data::data_dir_from(home));
    let graph = read_graph(&db).map_err(|e| format!("read {}: {e}", db.display()))?;
    let mut stats = match graph {
        Some((nodes, edges)) => summarize(&nodes, &edges, repo),
        None => Stats::default(),
    };
    stats.size_bytes = std::fs::metadata(&db).map(|m| m.len()).unwrap_or(0);
    stats.rebuilt_at = touched_at(&db)
        .ok()
        .flatten()
        .and_then(crate::common::time::epoch_secs_of);
    stats.db = db;
    Ok(stats)
}

/// The slug and checkout of the repo holding the current directory, when it
/// has an `origin` remote.
pub fn current_repo() -> Option<(String, PathBuf)> {
    let cwd = std::env::current_dir().ok()?;
    let top = crate::common::gitfacts::toplevel(&cwd)?;
    let slug = crate::common::repo::repo_slug_in(&top);
    (!slug.is_empty()).then_some((slug, top))
}

/// Counts the graph. Pure apart from the anchor checks against `repo`.
pub fn summarize(nodes: &[NodeRow], edges: &[EdgeRow], repo: Option<(String, PathBuf)>) -> Stats {
    let mut facts: BTreeMap<String, usize> = SCOPES.iter().map(|s| (s.to_string(), 0)).collect();
    let mut anchors = 0;
    let mut anchors_checked = 0;
    let mut dangling_anchors = Vec::new();
    for node in nodes {
        if node.scope != CODE_SCOPE {
            *facts.entry(node.scope.clone()).or_default() += 1;
            continue;
        }
        anchors += 1;
        let Some((slug, top)) = &repo else { continue };
        if node.project.as_deref() != Some(slug.as_str()) {
            continue;
        }
        anchors_checked += 1;
        if !anchor_exists(top, &node.file) {
            dangling_anchors.push(node.file.clone());
        }
    }
    let ids: HashSet<&str> = nodes.iter().map(|n| n.id.as_str()).collect();
    let mut by_relation: BTreeMap<String, usize> = BTreeMap::new();
    let mut dangling_edges = Vec::new();
    for edge in edges {
        *by_relation.entry(edge.relation.clone()).or_default() += 1;
        if !ids.contains(edge.to.as_str()) {
            dangling_edges.push((edge.from.clone(), edge.to.clone(), edge.relation.clone()));
        }
    }
    dangling_anchors.sort();
    Stats {
        present: true,
        facts,
        nodes: nodes.len(),
        anchors,
        edges: by_relation,
        dangling_edges,
        anchor_repo: repo,
        anchors_checked,
        dangling_anchors,
        ..Stats::default()
    }
}

/// Whether the anchor `file` (`path` or `path#symbol`) names a path that
/// exists in the checkout at `top`. Only the path is checked.
fn anchor_exists(top: &Path, file: &str) -> bool {
    let path = file.split('#').next().unwrap_or_default();
    path.is_empty() || top.join(path).exists()
}

impl Stats {
    fn total_facts(&self) -> usize {
        self.facts.values().sum()
    }

    fn total_edges(&self) -> usize {
        self.edges.values().sum()
    }
}

/// The summary for a person.
pub fn render_text(stats: &Stats) -> String {
    let mut out = format!("database: {}\n", stats.db.display());
    if !stats.present {
        out.push_str("no memory graph yet; run `playbook memory rebuild` to build it\n");
        return out;
    }
    out.push_str(&format!("size: {}\n", human_size(stats.size_bytes)));
    let rebuilt = stats
        .rebuilt_at
        .map_or_else(|| "unknown".to_string(), crate::common::time::iso_utc);
    out.push_str(&format!("last rebuild: {rebuilt}\n"));
    let per_scope: Vec<String> = stats
        .facts
        .iter()
        .map(|(s, n)| format!("{s} {n}"))
        .collect();
    out.push_str(&format!(
        "facts: {} ({})\n",
        stats.total_facts(),
        per_scope.join(", ")
    ));
    out.push_str(&format!(
        "nodes: {} (facts {}, anchors {})\n",
        stats.nodes,
        stats.total_facts(),
        stats.anchors
    ));
    out.push_str(&format!("edges: {}\n", stats.total_edges()));
    for (relation, n) in &stats.edges {
        out.push_str(&format!("  {relation}: {n}\n"));
    }
    out.push_str(&format!("dangling edges: {}\n", stats.dangling_edges.len()));
    for (from, to, relation) in &stats.dangling_edges {
        out.push_str(&format!("  {from} -{relation}-> {to} (target missing)\n"));
    }
    match &stats.anchor_repo {
        Some((slug, _)) => {
            out.push_str(&format!(
                "dangling anchors: {} of {} checked in {slug}\n",
                stats.dangling_anchors.len(),
                stats.anchors_checked
            ));
            for path in &stats.dangling_anchors {
                out.push_str(&format!("  {path} (missing on disk)\n"));
            }
        }
        None => out.push_str(
            "dangling anchors: not checked (run inside a repo to check its anchors on disk)\n",
        ),
    }
    out
}

/// The summary as JSON.
pub fn render_json(stats: &Stats) -> Value {
    let dangling_edges: Vec<Value> = stats
        .dangling_edges
        .iter()
        .map(|(from, to, relation)| json!({"from": from, "to": to, "relation": relation}))
        .collect();
    json!({
        "database": {
            "path": stats.db,
            "present": stats.present,
            "size_bytes": stats.size_bytes,
            "rebuilt_at": stats.rebuilt_at.map(crate::common::time::iso_utc),
        },
        "facts": {
            "total": stats.total_facts(),
            "by_scope": stats.facts,
        },
        "nodes": {
            "total": stats.nodes,
            "facts": stats.total_facts(),
            "anchors": stats.anchors,
        },
        "edges": {
            "total": stats.total_edges(),
            "by_relation": stats.edges,
        },
        "dangling": {
            "edges": dangling_edges,
            "anchors": stats.dangling_anchors,
            "anchors_checked": stats.anchors_checked,
            "anchor_repo": stats.anchor_repo.as_ref().map(|(slug, _)| slug),
        },
    })
}

/// Bytes as B, KiB or MiB, one decimal above a KiB.
fn human_size(bytes: u64) -> String {
    const KIB: f64 = 1024.0;
    let b = bytes as f64;
    if b < KIB {
        format!("{bytes} B")
    } else if b < KIB * KIB {
        format!("{:.1} KiB", b / KIB)
    } else {
        format!("{:.1} MiB", b / (KIB * KIB))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use playbook_common::test_support::scratch_dir;

    fn fact(id: &str, scope: &str, project: Option<&str>) -> NodeRow {
        NodeRow {
            id: id.into(),
            file: format!("{id}.md"),
            scope: scope.into(),
            project: project.map(str::to_string),
            kind: "feedback".into(),
            name: None,
            description: None,
            pinned: false,
        }
    }

    fn code(project: &str, file: &str) -> NodeRow {
        NodeRow {
            id: format!("code:{project}/{file}"),
            file: file.into(),
            scope: CODE_SCOPE.into(),
            project: Some(project.into()),
            kind: "code".into(),
            name: None,
            description: None,
            pinned: false,
        }
    }

    fn edge(from: &str, to: &str, relation: &str) -> EdgeRow {
        EdgeRow {
            from: from.into(),
            to: to.into(),
            relation: relation.into(),
            signals: None,
        }
    }

    #[test]
    fn counts_facts_per_scope_and_edges_per_relation() {
        let nodes = vec![
            fact("global/a", "global", None),
            fact("acme/b", "org", Some("acme")),
            fact("acme/web/c", "project", Some("acme/web")),
            fact("acme/web/d", "project", Some("acme/web")),
            code("acme/web", "src/lib.rs"),
        ];
        let edges = vec![
            edge("acme/web/c", "acme/web/d", "relates_to"),
            edge("acme/web/c", "code:acme/web/src/lib.rs", "anchors"),
            edge("acme/web/d", "global/a", "depends_on"),
        ];

        let s = summarize(&nodes, &edges, None);

        assert_eq!(s.facts["global"], 1);
        assert_eq!(s.facts["org"], 1);
        assert_eq!(s.facts["project"], 2);
        assert_eq!((s.nodes, s.anchors), (5, 1));
        assert_eq!(s.edges["anchors"], 1);
        assert_eq!(s.edges["relates_to"], 1);
        assert_eq!(s.edges["depends_on"], 1);
        assert!(s.dangling_edges.is_empty());
        assert_eq!(s.anchors_checked, 0);
    }

    #[test]
    fn an_edge_to_a_missing_node_is_dangling() {
        let nodes = vec![fact("global/a", "global", None)];
        let edges = vec![edge("global/a", "global/gone", "supersedes")];

        let s = summarize(&nodes, &edges, None);

        assert_eq!(
            s.dangling_edges,
            [("global/a".into(), "global/gone".into(), "supersedes".into())]
        );
    }

    #[test]
    fn only_anchors_of_the_current_repo_are_checked_on_disk() {
        let top = scratch_dir("memory-stats-anchors");
        std::fs::create_dir_all(top.join("src")).unwrap();
        std::fs::write(top.join("src/lib.rs"), "").unwrap();
        let nodes = vec![
            code("acme/web", "src/lib.rs#run"),
            code("acme/web", "src/gone.rs"),
            code("acme/other", "src/nowhere.rs"),
        ];

        let s = summarize(&nodes, &[], Some(("acme/web".into(), top)));

        assert_eq!(s.anchors, 3);
        assert_eq!(s.anchors_checked, 2);
        assert_eq!(s.dangling_anchors, ["src/gone.rs"]);
    }

    #[test]
    fn a_missing_database_reads_as_no_graph() {
        let home = scratch_dir("memory-stats-missing");

        let s = collect(&home, None).unwrap();

        assert!(!s.present);
        assert!(render_text(&s).contains("playbook memory rebuild"));
        assert!(!s.db.exists(), "stats must not create the database");
    }

    #[test]
    fn sizes_read_in_the_nearest_unit() {
        assert_eq!(human_size(512), "512 B");
        assert_eq!(human_size(2048), "2.0 KiB");
        assert_eq!(human_size(3 * 1024 * 1024), "3.0 MiB");
    }
}
