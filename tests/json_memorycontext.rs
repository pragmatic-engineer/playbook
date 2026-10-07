// SPDX-FileCopyrightText: 2026 Igor Santos
// SPDX-License-Identifier: MIT

//! Differential test for `playbook::json::memorycontext::render_memory_context`
//! against the real `jq` filter the former `shell/memory-context.sh` runs. Skipped when
//! `jq` is absent from PATH, matching `tests/doctor_field.rs`'s skip pattern.

use playbook::json::memorycontext::render_memory_context;
use std::fs;
use std::path::PathBuf;
use std::process::Command;
use std::sync::atomic::{AtomicU64, Ordering};

static COUNTER: AtomicU64 = AtomicU64::new(0);

/// The exact filter the former `shell/memory-context.sh` runs through `jq -r`.
const JQ_FILTER: &str = r#"
  def in_scope: .scope == "global"
    or (.scope == "project" and .project == $repo)
    or (.scope == "org" and .project == ($repo | split("/")[0]));

  (.nodes | map(select(in_scope))) as $facts
  | ($facts | map(.id)) as $ids
  | (reduce $ids[] as $i ({}; .[$i] = true)) as $inscope
  | (.nodes | map({(.id): .}) | add) as $byid
  | ($facts | sort_by(.name) | map("\(.name): \(.description)") | join("\n")) as $facts_block
  | (
      [.edges[] | select(.relation != "anchors")
                | select($inscope[.from] == true and $inscope[.to] == true)]
      | sort_by([.relation, .from, .to])
      | map("\($byid[.from].name) \(.relation) \($byid[.to].name)")
      | join("\n")
    ) as $edges_block
  | (
      [.edges[] | select(.relation == "anchors") | select($inscope[.from] == true)]
      | group_by(.to)
      | map({path: $byid[.[0].to].file, names: (map($byid[.from].name) | sort | join(", "))})
      | sort_by(.path)
      | map("\(.path): \(.names)")
      | join("\n")
    ) as $anchors_block
  | [
      (if $facts_block   != "" then "Facts:\n"   + $facts_block   else empty end),
      (if $edges_block   != "" then "Edges:\n"   + $edges_block   else empty end),
      (if $anchors_block != "" then "Anchors:\n" + $anchors_block else empty end)
    ]
  | join("\n\n")
"#;

/// A scratch graph JSON file, cleaned up on drop.
struct Fixture {
    path: PathBuf,
}

impl Fixture {
    fn new(tag: &str, contents: &str) -> Self {
        let n = COUNTER.fetch_add(1, Ordering::Relaxed);
        let path = std::env::temp_dir().join(format!(
            "playbook-json-memorycontext-{tag}-{}-{n}.json",
            std::process::id()
        ));
        fs::write(&path, contents).expect("fixture file should be writable");
        Self { path }
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = fs::remove_file(&self.path);
    }
}

/// Runs the real `JQ_FILTER` against `fixture` for `repo`, stripping the
/// trailing newline `jq -r` always emits, the same way the former `shell/memory-context.sh`'s
/// `output="$(...)"` capture strips it.
fn run_jq(fixture: &Fixture, repo: &str) -> String {
    let out = Command::new("jq")
        .args(["-r", "--arg", "repo", repo, JQ_FILTER])
        .arg(&fixture.path)
        .output()
        .expect("jq should run");
    String::from_utf8_lossy(&out.stdout)
        .trim_end_matches('\n')
        .to_string()
}

fn assert_matches_jq(tag: &str, contents: &str, repo: &str) {
    let fixture = Fixture::new(tag, contents);
    let jq_output = run_jq(&fixture, repo);
    let rust_output = render_memory_context(contents, repo);
    assert_eq!(rust_output, jq_output, "mismatch for case {tag}");
}

#[test]
fn matches_jq_across_scope_sort_order_and_empty_repo_fixtures() {
    if Command::new("jq").arg("--version").output().is_err() {
        eprintln!("SKIP: jq not installed");
        return;
    }

    // Scope fixture: global, project-match, project-mismatch, org-match,
    // org-mismatch, reused from the RED report's verified scenario.
    assert_matches_jq(
        "scope",
        r#"{
            "nodes": [
                {"id": "n-charlie", "scope": "org", "project": "acme", "type": "reference", "name": "charlie", "description": "org fact in scope"},
                {"id": "n-alpha", "scope": "global", "type": "reference", "name": "alpha", "description": "global fact"},
                {"id": "n-delta", "scope": "project", "project": "other/proj", "type": "reference", "name": "delta", "description": "different project"},
                {"id": "n-bravo", "scope": "project", "project": "acme/proj", "type": "reference", "name": "bravo", "description": "project fact in scope"},
                {"id": "n-echo", "scope": "org", "project": "other", "type": "reference", "name": "echo", "description": "different org"}
            ],
            "edges": []
        }"#,
        "acme/proj",
    );

    // Sort-order fixture: edge row order follows node id, not name, but
    // renders using names.
    assert_matches_jq(
        "sort-order",
        r#"{
            "nodes": [
                {"id": "z1", "scope": "global", "type": "reference", "name": "alpha", "description": "d-alpha"},
                {"id": "a1", "scope": "global", "type": "reference", "name": "zulu", "description": "d-zulu"},
                {"id": "m1", "scope": "global", "type": "reference", "name": "mike", "description": "d-mike"}
            ],
            "edges": [
                {"from": "z1", "to": "a1", "relation": "relates_to"},
                {"from": "a1", "to": "m1", "relation": "relates_to"}
            ]
        }"#,
        "acme/proj",
    );

    // Empty-repo-slug fixture: an org-scoped node with an empty project
    // string must not match an unresolved (empty) repo.
    assert_matches_jq(
        "empty-repo",
        r#"{
            "nodes": [
                {"id": "n1", "scope": "org", "project": "", "type": "reference", "name": "orgempty", "description": "d"},
                {"id": "g1", "scope": "global", "type": "reference", "name": "globalone", "description": "gd"}
            ],
            "edges": []
        }"#,
        "",
    );
}
