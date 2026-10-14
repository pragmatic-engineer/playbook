// SPDX-FileCopyrightText: 2026 Igor Santos
// SPDX-License-Identifier: Apache-2.0

//! The hook reference and the "What init installs" guide must cover every hook
//! `playbook init` wires. The test wires a scratch settings file with the real
//! code and checks each entry against the page, so a new hook cannot ship
//! without a row.

use playbook::init::wire::{self, wired_hooks};
use serde_json::Value;
use std::collections::BTreeSet;
use std::fs;
use std::path::PathBuf;
use std::sync::atomic::{AtomicUsize, Ordering};

const REFERENCE: &str = "docs/reference/hooks.md";
const GUIDE: &str = "docs/guides/06-what-init-installs.md";

fn root() -> PathBuf {
    PathBuf::from(env!("PLAYBOOK_REPO_ROOT"))
}

fn read(rel: &str) -> String {
    fs::read_to_string(root().join(rel)).unwrap_or_else(|e| panic!("{rel}: {e}"))
}

/// Every (event, matcher, hook name) entry in a settings value.
fn entries(settings: &Value) -> Vec<(String, String, String)> {
    let mut out = Vec::new();
    let hooks = settings["hooks"].as_object().expect("hooks object");
    for (event, groups) in hooks {
        for group in groups.as_array().expect("event array") {
            let matcher = group["matcher"].as_str().unwrap_or("none").to_string();
            for hook in group["hooks"].as_array().expect("hooks array") {
                let command = hook["command"].as_str().expect("command");
                let name = command
                    .rsplit(" hook ")
                    .next()
                    .expect("hook name")
                    .to_string();
                out.push((event.clone(), matcher.clone(), name));
            }
        }
    }
    out
}

fn row_prefix((event, matcher, name): &(String, String, String)) -> String {
    let matcher = matcher.replace('|', "\\|");
    format!(
        "| `{event}` | {} | `playbook hook {name}` |",
        cell(&matcher)
    )
}

fn cell(matcher: &str) -> String {
    if matcher == "none" {
        "none".to_string()
    } else {
        format!("`{matcher}`")
    }
}

fn wired_entries() -> Vec<(String, String, String)> {
    static NEXT: AtomicUsize = AtomicUsize::new(0);
    let dir = std::env::temp_dir().join(format!(
        "pb-init-docs-{}-{}",
        std::process::id(),
        NEXT.fetch_add(1, Ordering::SeqCst)
    ));
    fs::create_dir_all(&dir).unwrap();
    let path = dir.join("settings.json");
    fs::write(&path, "{}\n").unwrap();
    wire::wire(&path).expect("wire a scratch settings file");
    let value: Value = serde_json::from_str(&fs::read_to_string(&path).unwrap()).unwrap();
    let _ = fs::remove_dir_all(&dir);
    entries(&value)
}

#[test]
fn every_hook_init_wires_has_a_row_in_the_hook_reference() {
    let page = read(REFERENCE);
    let wired = wired_entries();
    assert!(!wired.is_empty());

    for entry in &wired {
        let prefix = row_prefix(entry);
        assert!(
            page.lines().any(|l| l.starts_with(&prefix)),
            "{REFERENCE} has no table row starting with: {prefix}"
        );
    }
}

#[test]
fn every_hook_in_the_shipped_template_is_one_init_wires() {
    let template: Value = serde_json::from_str(&read("settings.shared.json")).unwrap();
    let wired: BTreeSet<_> = wired_entries().into_iter().collect();

    for entry in entries(&template) {
        assert!(
            wired.contains(&entry),
            "settings.shared.json carries a hook init does not wire: {entry:?}"
        );
    }
}

#[test]
fn the_reference_has_no_row_for_a_hook_nothing_ships() {
    let page = read(REFERENCE);
    let known: BTreeSet<String> = wired_entries().iter().map(row_prefix).collect();

    for line in page
        .lines()
        .filter(|l| l.starts_with('|') && l.contains("`playbook hook "))
    {
        assert!(
            known.iter().any(|k| line.starts_with(k)),
            "{REFERENCE} lists a hook that is not shipped: {line}"
        );
    }
}

#[test]
fn the_guide_states_the_real_hook_counts() {
    let guide = read(GUIDE);
    let wired = wired_entries();
    let names: BTreeSet<_> = wired.iter().map(|(_, _, n)| n.clone()).collect();

    let claim = format!(
        "Init adds {} hook entries to `~/.claude/settings.json`. They run {} different",
        wired.len(),
        names.len()
    );
    assert!(guide.contains(&claim), "{GUIDE} must say: {claim}");
}

#[test]
fn the_new_pages_are_linked_from_the_readme_the_index_and_the_install_guide() {
    for file in ["README.md", "docs/index.md", "docs/guides/00-install.md"] {
        let text = read(file);
        assert!(
            text.contains("06-what-init-installs.md"),
            "{file} does not link to the guide"
        );
        assert!(
            text.contains("reference/hooks.md"),
            "{file} does not link to the hook reference"
        );
    }
}

fn table_entries(
    specs: impl Iterator<Item = &'static wire::HookSpec>,
) -> BTreeSet<(String, String, String)> {
    specs
        .map(|h| {
            (
                h.event.to_string(),
                h.matcher.unwrap_or("none").to_string(),
                h.name.to_string(),
            )
        })
        .collect()
}

#[test]
fn the_hook_table_in_code_is_exactly_what_init_wires() {
    let wired: BTreeSet<_> = wired_entries().into_iter().collect();

    assert_eq!(table_entries(wired_hooks()), wired);
}

#[test]
fn the_reference_says_what_each_hook_does_in_the_words_the_table_uses() {
    let page = read(REFERENCE);

    for h in wired_hooks() {
        let matcher = h.matcher.unwrap_or("none").to_string();
        let prefix = row_prefix(&(h.event.to_string(), matcher, h.name.to_string()));
        let row = page
            .lines()
            .find(|l| l.starts_with(&prefix))
            .unwrap_or_else(|| panic!("no row for {}", h.name));
        assert!(
            row.contains(&format!("| {} |", h.purpose)),
            "the row for {} {} does not carry its purpose: {}",
            h.event,
            h.name,
            h.purpose
        );
    }
}

#[test]
fn every_hook_sits_under_the_heading_of_its_group() {
    let page = read(REFERENCE);

    for group in wire::Group::ALL {
        let heading = format!("## {}\n", group.label());
        let start = page
            .find(&heading)
            .unwrap_or_else(|| panic!("no heading for {}", group.label()));
        let end = page[start + heading.len()..]
            .find("\n## ")
            .map_or(page.len(), |n| start + heading.len() + n);
        let section = &page[start..end];
        for h in wired_hooks().filter(|h| h.group == group) {
            assert!(
                section.contains(&format!("`playbook hook {}`", h.name)),
                "{} is not under {}",
                h.name,
                group.label()
            );
        }
    }
}
