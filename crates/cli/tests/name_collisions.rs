// SPDX-FileCopyrightText: 2026 Igor Santos
// SPDX-License-Identifier: Apache-2.0

//! Claude Code exposes each skill as `/playbook:<name>` as well, so a skill
//! and a command with the same name collide. Fail on any shared name.

use std::collections::BTreeMap;
use std::fs;
use std::path::Path;

fn root(sub: &str) -> std::path::PathBuf {
    Path::new(env!("PLAYBOOK_REPO_ROOT")).join(sub)
}

/// Skill names: directory names and the `name:` in each SKILL.md.
fn skill_names() -> Vec<(String, String)> {
    let mut out = Vec::new();
    for e in fs::read_dir(root("skills")).unwrap().filter_map(Result::ok) {
        let md = e.path().join("SKILL.md");
        if let Ok(text) = fs::read_to_string(md) {
            let dir = e.file_name().to_string_lossy().into_owned();
            out.push((dir.clone(), dir.clone()));
            if let Some(n) = text
                .lines()
                .find_map(|l| l.strip_prefix("name:"))
                .map(|n| n.trim().to_string())
            {
                out.push((n, format!("{dir}/SKILL.md name")));
            }
        }
    }
    out
}

fn command_names() -> Vec<String> {
    fs::read_dir(root("commands"))
        .unwrap()
        .filter_map(Result::ok)
        .filter_map(|e| {
            let p = e.path();
            (p.extension().is_some_and(|x| x == "md"))
                .then(|| p.file_stem().unwrap().to_string_lossy().into_owned())
        })
        .collect()
}

#[test]
fn no_skill_shares_a_name_with_a_command() {
    assert!(
        !skill_names().is_empty() && !command_names().is_empty(),
        "scan found nothing"
    );
    let commands = command_names();
    let mut clashes = Vec::new();
    for (name, origin) in skill_names() {
        if commands.contains(&name) {
            clashes.push(format!(
                "skills/{origin} and commands/{name}.md share the name `{name}`"
            ));
        }
    }
    assert!(
        clashes.is_empty(),
        "name collision:\n  {}",
        clashes.join("\n  ")
    );
}

#[test]
fn no_two_skills_share_a_name() {
    let mut seen: BTreeMap<String, String> = BTreeMap::new();
    let mut clashes = Vec::new();
    for (name, origin) in skill_names() {
        let dir = origin.split('/').next().unwrap_or("").to_string();
        match seen.get(&name) {
            Some(prev) if *prev != dir => {
                clashes.push(format!("skills/{prev} and skills/{dir} share `{name}`"));
            }
            _ => {
                seen.insert(name, dir);
            }
        }
    }
    assert!(
        clashes.is_empty(),
        "name collision:\n  {}",
        clashes.join("\n  ")
    );
}
