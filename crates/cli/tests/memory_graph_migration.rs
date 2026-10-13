// SPDX-FileCopyrightText: 2026 Igor Santos
// SPDX-License-Identifier: Apache-2.0

//! Moving a `memory.graph.json` from an older playbook into
//! `~/.config/playbook/data/memory.db`: only an old file, no graph, both, a
//! corrupt old file, a corrupt database, a busy lock and concurrent readers.
//! Each test gets its own scratch `HOME` and runs the compiled binary.

use serde_json::{json, Value};
use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Output, Stdio};
use std::sync::atomic::{AtomicU64, Ordering};

static COUNTER: AtomicU64 = AtomicU64::new(0);

const PINNED_FACT: &str = "---\nname: a-fact\ndescription: d\ntype: feedback\npinned: true\nanchors:\n  - code:~/.zshrc\n---\nbody\n";

fn scratch_home(tag: &str) -> PathBuf {
    let n = COUNTER.fetch_add(1, Ordering::Relaxed);
    let home = std::env::temp_dir().join(format!(
        "playbook-memmig-{}-{tag}-{n}",
        playbook::testing::run_id()
    ));
    fs::create_dir_all(home.join(".config").join("playbook").join("memory")).unwrap();
    home
}

fn write_fact(home: &Path, relpath: &str, content: &str) {
    let full = home
        .join(".config")
        .join("playbook")
        .join("memory")
        .join(relpath);
    fs::create_dir_all(full.parent().unwrap()).unwrap();
    fs::write(full, content).unwrap();
}

fn legacy_json_path(home: &Path) -> PathBuf {
    home.join(".config")
        .join("playbook")
        .join("memory")
        .join("memory.graph.json")
}

fn memory_db(home: &Path) -> PathBuf {
    home.join(".config")
        .join("playbook")
        .join("data")
        .join("memory.db")
}

fn run_playbook(home: &Path, args: &[&str], hook_input: &str) -> Output {
    Command::new(env!("CARGO_BIN_EXE_playbook"))
        .env_remove("CI")
        .env_remove("PLAYBOOK_HEADLESS")
        .args(args)
        .current_dir(env!("PLAYBOOK_REPO_ROOT"))
        .env("HOME", home)
        .env("HOOK_INPUT", hook_input)
        .stdin(Stdio::null())
        .output()
        .expect("playbook binary should run")
}

fn run_rebuild_for(home: &Path, relpath: &str) -> Output {
    let file_path = home
        .join(".config")
        .join("playbook")
        .join("memory")
        .join(relpath);
    let hook_input = json!({"tool_input": {"file_path": file_path.to_string_lossy()}}).to_string();
    run_playbook(home, &["hook", "rebuild-memory-graph"], &hook_input)
}

fn read_graph(home: &Path) -> Value {
    let root = home.join(".config").join("playbook");
    let content = playbook::hooks::rebuild_memory_graph::export_json(&root)
        .expect("memory.db should be readable")
        .expect("memory.db should exist");
    serde_json::from_str(&content).expect("the exported graph should be valid JSON")
}

fn nodes(graph: &Value) -> &Vec<Value> {
    graph["nodes"].as_array().expect("nodes should be an array")
}

fn has_node(graph: &Value, id: &str) -> bool {
    nodes(graph).iter().any(|n| n["id"] == id)
}

fn export(home: &Path) -> Output {
    run_playbook(home, &["memory", "export", "--json"], "")
}

const OLD_JSON: &str = r#"{"nodes":[{"id":"global/stale","file":"stale.md","scope":"global","type":"reference"}],"edges":[],"version":1}"#;

fn write_legacy_json(home: &Path) {
    fs::write(legacy_json_path(home), OLD_JSON).unwrap();
}

fn migrated_leftovers(home: &Path) -> Vec<String> {
    fs::read_dir(legacy_json_path(home).parent().unwrap())
        .unwrap()
        .flatten()
        .map(|e| e.file_name().to_string_lossy().into_owned())
        .filter(|n| n.starts_with("memory.graph.json.migrated"))
        .collect()
}

#[test]
fn an_old_graph_file_is_rebuilt_from_the_facts_into_memory_db_and_set_aside() {
    let home = scratch_home("memdb-old");
    write_fact(&home, "global/a-fact.md", PINNED_FACT);
    write_legacy_json(&home);

    let out = export(&home);

    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    let doc: Value = serde_json::from_slice(&out.stdout).unwrap();
    assert!(has_node(&doc, "global/a-fact"), "built from the facts");
    assert!(!has_node(&doc, "global/stale"), "not copied from the file");
    assert!(!legacy_json_path(&home).exists());
    assert_eq!(migrated_leftovers(&home), ["memory.graph.json.migrated"]);
    assert_eq!(
        fs::read_to_string(legacy_json_path(&home).with_extension("json.migrated")).unwrap(),
        OLD_JSON,
        "the old file is kept as it was"
    );
}

#[test]
fn no_graph_at_all_exports_an_empty_graph() {
    let home = scratch_home("memdb-none");

    let out = export(&home);

    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    let doc: Value = serde_json::from_slice(&out.stdout).unwrap();
    assert!(nodes(&doc).is_empty());
    assert!(migrated_leftovers(&home).is_empty());
}

#[test]
fn a_graph_file_that_an_older_binary_wrote_beside_a_database_is_folded_in_again() {
    let home = scratch_home("memdb-both");
    write_fact(&home, "global/a-fact.md", PINNED_FACT);
    run_rebuild_for(&home, "global/a-fact.md");
    write_fact(
        &home,
        "global/b-fact.md",
        PINNED_FACT.replace("a-fact", "b-fact").as_str(),
    );
    write_legacy_json(&home); // what the older binary's hook would have written

    let out = export(&home);

    let doc: Value = serde_json::from_slice(&out.stdout).unwrap();
    assert!(has_node(&doc, "global/a-fact"));
    assert!(has_node(&doc, "global/b-fact"), "the new fact is in");
    assert!(!legacy_json_path(&home).exists());
}

#[test]
fn a_second_old_graph_file_replaces_the_first_kept_copy() {
    let home = scratch_home("memdb-twice-old");
    write_fact(&home, "global/a-fact.md", PINNED_FACT);
    write_legacy_json(&home);
    export(&home);
    fs::write(legacy_json_path(&home), r#"{"nodes":[],"edges":[]}"#).unwrap();

    export(&home);

    assert_eq!(
        migrated_leftovers(&home),
        ["memory.graph.json.migrated"],
        "one kept copy, so an older binary cannot fill the folder"
    );
    assert_eq!(
        fs::read_to_string(legacy_json_path(&home).with_extension("json.migrated")).unwrap(),
        r#"{"nodes":[],"edges":[]}"#
    );
}

#[test]
fn a_graph_file_older_than_the_database_is_set_aside_without_a_rebuild() {
    let home = scratch_home("memdb-json-older");
    write_fact(&home, "global/a-fact.md", PINNED_FACT);
    write_legacy_json(&home);
    std::thread::sleep(std::time::Duration::from_millis(50));
    run_rebuild_for(&home, "global/a-fact.md");
    write_fact(
        &home,
        "global/b-fact.md",
        PINNED_FACT.replace("a-fact", "b-fact").as_str(),
    );

    let out = export(&home);

    let doc: Value = serde_json::from_slice(&out.stdout).unwrap();
    assert!(!legacy_json_path(&home).exists());
    assert!(has_node(&doc, "global/a-fact"));
    assert!(
        !has_node(&doc, "global/b-fact"),
        "the database was already newer than the file, so nothing was rebuilt"
    );
}

const LEGACY_FACT_JSON: &str = r#"{"nodes":[{"id":"global/legacy-fact","file":"legacy-fact.md","scope":"global","type":"reference","name":"legacy-fact","description":"kept in the old file","pinned":true}],"edges":[]}"#;

fn context(home: &Path) -> String {
    let out = run_playbook(home, &["memory", "context", "--repo", "acme/widget"], "");
    String::from_utf8_lossy(&out.stdout).into_owned()
}

#[test]
fn a_busy_rebuild_lock_leaves_the_old_graph_file_alone_and_readable() {
    let home = scratch_home("memdb-lock-busy");
    fs::write(legacy_json_path(&home), LEGACY_FACT_JSON).unwrap();
    fs::create_dir(
        legacy_json_path(&home)
            .parent()
            .unwrap()
            .join("memory.graph.json.lock"),
    )
    .unwrap();

    let text = context(&home);

    assert!(text.contains("legacy-fact"), "read as it is: {text}");
    assert!(
        legacy_json_path(&home).is_file(),
        "not moved without the lock"
    );
    assert!(migrated_leftovers(&home).is_empty());
}

#[test]
fn a_failed_rebuild_leaves_the_old_graph_file_alone_and_readable() {
    let home = scratch_home("memdb-rebuild-fails");
    fs::write(legacy_json_path(&home), LEGACY_FACT_JSON).unwrap();
    fs::create_dir_all(memory_db(&home)).unwrap(); // a directory where the database should be

    let text = context(&home);

    assert!(text.contains("legacy-fact"), "read as it is: {text}");
    assert!(legacy_json_path(&home).is_file());
}

#[test]
fn a_corrupt_old_graph_file_does_not_stop_the_move() {
    let home = scratch_home("memdb-corrupt-old");
    write_fact(&home, "global/a-fact.md", PINNED_FACT);
    fs::write(legacy_json_path(&home), "{not json").unwrap();

    let out = export(&home);

    let doc: Value = serde_json::from_slice(&out.stdout).unwrap();
    assert!(has_node(&doc, "global/a-fact"));
    assert_eq!(migrated_leftovers(&home).len(), 1);
}

#[test]
fn a_database_that_is_not_a_database_is_set_aside_and_rebuilt() {
    let home = scratch_home("memdb-corrupt-db");
    write_fact(&home, "global/a-fact.md", PINNED_FACT);
    fs::create_dir_all(memory_db(&home).parent().unwrap()).unwrap();
    fs::write(
        memory_db(&home),
        b"this is not a sqlite file, only text that is long enough to read as a header",
    )
    .unwrap();

    let out = run_rebuild_for(&home, "global/a-fact.md");

    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    assert!(has_node(&read_graph(&home), "global/a-fact"));
    let kept: Vec<_> = fs::read_dir(memory_db(&home).parent().unwrap())
        .unwrap()
        .flatten()
        .filter(|e| e.file_name().to_string_lossy().contains(".migrated"))
        .collect();
    assert_eq!(kept.len(), 1, "the unreadable file is kept, not deleted");
}

#[test]
fn two_readers_moving_an_old_graph_file_at_once_both_succeed() {
    let home = scratch_home("memdb-race");
    write_fact(&home, "global/a-fact.md", PINNED_FACT);
    write_legacy_json(&home);

    let handles: Vec<_> = (0..4)
        .map(|_| {
            let home = home.clone();
            std::thread::spawn(move || export(&home))
        })
        .collect();

    for handle in handles {
        let out = handle.join().unwrap();
        assert!(
            out.status.success(),
            "{}",
            String::from_utf8_lossy(&out.stderr)
        );
        let doc: Value = serde_json::from_slice(&out.stdout).unwrap();
        assert!(has_node(&doc, "global/a-fact"));
    }
    assert!(!legacy_json_path(&home).exists());
    assert_eq!(migrated_leftovers(&home).len(), 1);
}
