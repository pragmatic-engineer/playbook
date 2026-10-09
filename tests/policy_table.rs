// SPDX-FileCopyrightText: 2026 Igor Santos
// SPDX-License-Identifier: Apache-2.0

//! The policy table in docs/internals/02-model-routing-and-memory.md lists the
//! model and effort of every agent, command and skill. This test keeps it equal
//! to the frontmatter of the real files.

use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};

const DOC: &str = "docs/internals/02-model-routing-and-memory.md";

fn root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
}

/// `(model, effort)` from the frontmatter, `-` when a key is absent.
fn frontmatter(path: &Path) -> (String, String) {
    let text = fs::read_to_string(path).unwrap();
    let body = text.strip_prefix("---\n").expect("frontmatter");
    let block = &body[..body.find("\n---").expect("closing ---")];
    let get = |key: &str| {
        block
            .lines()
            .find_map(|l| l.strip_prefix(&format!("{key}:")))
            .map_or_else(
                || "-".to_string(),
                |v| v.trim().trim_matches('"').to_string(),
            )
    };
    (get("model"), get("effort"))
}

/// Real components: `(kind, name) -> (model, effort)`.
fn components() -> BTreeMap<(String, String), (String, String)> {
    let mut out = BTreeMap::new();
    for (kind, dir) in [("agent", "agents"), ("command", "commands")] {
        for entry in fs::read_dir(root().join(dir)).unwrap() {
            let path = entry.unwrap().path();
            if path.extension().is_some_and(|e| e == "md") {
                let name = path.file_stem().unwrap().to_string_lossy().to_string();
                out.insert((kind.to_string(), name), frontmatter(&path));
            }
        }
    }
    for entry in fs::read_dir(root().join("skills")).unwrap() {
        let path = entry.unwrap().path().join("SKILL.md");
        if path.is_file() {
            let name = path
                .parent()
                .unwrap()
                .file_name()
                .unwrap()
                .to_string_lossy()
                .to_string();
            out.insert(("skill".to_string(), name), frontmatter(&path));
        }
    }
    out
}

fn cell(raw: &str) -> String {
    raw.trim().trim_matches('`').to_string()
}

/// The rows of the marked table: `(kind, name) -> (model, effort)`.
fn table() -> BTreeMap<(String, String), (String, String)> {
    let doc = fs::read_to_string(root().join(DOC)).unwrap();
    let start = doc
        .find("<!-- policy-table:start -->")
        .expect("start marker");
    let end = doc.find("<!-- policy-table:end -->").expect("end marker");
    let mut out = BTreeMap::new();
    for line in doc[start..end].lines().filter(|l| l.starts_with("| ")) {
        let cols: Vec<&str> = line.trim_matches('|').split('|').collect();
        if cols.len() < 5 || cell(cols[0]) == "Kind" || cell(cols[0]).starts_with(':') {
            continue;
        }
        out.insert(
            (cell(cols[0]), cell(cols[1])),
            (cell(cols[2]), cell(cols[3])),
        );
    }
    out
}

#[test]
fn the_policy_table_matches_every_component_file() {
    let real = components();
    let doc = table();
    let mut problems = Vec::new();
    for (key, want) in &real {
        match doc.get(key) {
            None => problems.push(format!("missing row for {} `{}`", key.0, key.1)),
            Some(got) if got != want => problems.push(format!(
                "{} `{}`: the file says model {} effort {}, the table says model {} effort {}",
                key.0, key.1, want.0, want.1, got.0, got.1
            )),
            Some(_) => {}
        }
    }
    for key in doc.keys() {
        if !real.contains_key(key) {
            problems.push(format!("row for {} `{}` has no file", key.0, key.1));
        }
    }
    assert!(
        problems.is_empty(),
        "the policy table in {DOC} is out of date:\n  {}",
        problems.join("\n  ")
    );
}

#[test]
fn skills_never_set_a_model_or_effort() {
    for ((kind, name), (model, effort)) in components() {
        if kind == "skill" {
            assert_eq!(
                (model.as_str(), effort.as_str()),
                ("-", "-"),
                "skill `{name}` sets a model or effort, which would override the caller"
            );
        }
    }
}
