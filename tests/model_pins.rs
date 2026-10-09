// SPDX-FileCopyrightText: 2026 Igor Santos
// SPDX-License-Identifier: Apache-2.0

//! No plugin file or workflow pins a model id outside the table in
//! `src/models`. Plugin files name a tier by alias, and Claude Code resolves it.

use playbook::models::{unknown_pins, TIERS};
use std::fs;
use std::path::{Path, PathBuf};

fn files(dir: &Path, out: &mut Vec<PathBuf>) {
    let Ok(entries) = fs::read_dir(dir) else {
        return;
    };
    for e in entries.flatten() {
        let p = e.path();
        if p.is_dir() {
            files(&p, out);
        } else if matches!(
            p.extension().and_then(|x| x.to_str()),
            Some("md" | "json" | "yml" | "yaml")
        ) {
            out.push(p);
        }
    }
}

#[test]
fn no_shipped_file_pins_a_model_outside_the_table() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"));
    let mut all = Vec::new();
    for d in [
        "agents",
        "commands",
        "skills",
        "prompts",
        "output-styles",
        ".github/workflows",
    ] {
        files(&root.join(d), &mut all);
    }
    all.push(root.join("settings.shared.json"));
    let mut bad = Vec::new();
    for f in all {
        let Ok(text) = fs::read_to_string(&f) else {
            continue;
        };
        for id in unknown_pins(&text) {
            bad.push(format!("{}: {id}", f.strip_prefix(root).unwrap().display()));
        }
    }
    assert!(
        bad.is_empty(),
        "model ids outside the table:\n{}",
        bad.join("\n")
    );
}

#[test]
fn every_agent_uses_a_tier_alias() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("agents");
    let aliases: Vec<&str> = TIERS.iter().map(|t| t.alias).collect();
    for e in fs::read_dir(root).unwrap().flatten() {
        let text = fs::read_to_string(e.path()).unwrap();
        let model = text
            .lines()
            .find_map(|l| l.strip_prefix("model: "))
            .unwrap_or("");
        assert!(
            aliases.contains(&model.trim()),
            "{}: {model}",
            e.path().display()
        );
    }
}
