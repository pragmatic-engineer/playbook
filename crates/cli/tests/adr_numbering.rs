// SPDX-FileCopyrightText: 2026 Igor Santos
// SPDX-License-Identifier: Apache-2.0

//! The ADR numbers in `docs/adr` run 1, 2, 3 and so on with no gap, and
//! every link to an ADR file in shipped text names a file that exists. A
//! number may repeat only for the blueprint, quality and test mapping
//! companions of one ADR, which share its number and start with its slug.

#[path = "support/shipped.rs"]
mod shipped;

use std::collections::BTreeSet;
use std::fs;
use std::path::PathBuf;

fn root() -> PathBuf {
    PathBuf::from(env!("PLAYBOOK_REPO_ROOT"))
}

/// The file names under `docs/adr`.
fn adr_files() -> Vec<String> {
    let mut names: Vec<String> = fs::read_dir(root().join("docs/adr"))
        .expect("docs/adr exists")
        .flatten()
        .map(|e| e.file_name().to_string_lossy().into_owned())
        .filter(|n| n.ends_with(".md"))
        .collect();
    names.sort();
    names
}

/// Whether `name` starts with four digits and a dash.
fn has_number(name: &str) -> bool {
    name.len() > 5 && name.as_bytes()[4] == b'-' && name[..4].bytes().all(|b| b.is_ascii_digit())
}

/// The four digit number a file name starts with.
fn number(name: &str) -> u32 {
    assert!(has_number(name), "{name} does not start with NNNN-");
    name[..4].parse().expect("digits")
}

/// Whether `name` is a blueprint, quality or test mapping companion.
fn is_companion(name: &str) -> bool {
    let stem = name.trim_end_matches(".md");
    stem.ends_with("-blueprint") || stem.ends_with("-quality") || stem == "0007-test-mapping"
}

#[test]
fn adr_numbers_are_contiguous_from_one() {
    let numbers: BTreeSet<u32> = adr_files().iter().map(|n| number(n)).collect();
    let last = *numbers.last().expect("at least one ADR");
    let missing: Vec<u32> = (1..=last).filter(|n| !numbers.contains(n)).collect();
    assert!(
        missing.is_empty(),
        "ADR numbers missing from 1..={last}: {missing:?}"
    );
}

#[test]
fn a_number_is_shared_only_with_companions_of_one_adr() {
    let files = adr_files();
    let mut seen = BTreeSet::new();
    for name in files.iter().filter(|n| !is_companion(n)) {
        assert!(
            seen.insert(number(name)),
            "two ADRs share the number in {name}"
        );
    }
    for name in files.iter().filter(|n| is_companion(n)) {
        assert!(
            seen.contains(&number(name)),
            "{name} is a companion of an ADR that does not exist"
        );
    }
}

#[test]
fn every_adr_link_resolves_to_a_file() {
    let root = root();
    let files = adr_files();
    let mut docs = Vec::new();
    for rel in ["docs", "README.md", "CONTRIBUTING.md"] {
        shipped::files_under(&root.join(rel), &["md"], &mut docs);
    }
    let mut broken = Vec::new();
    for path in docs {
        let in_adr = path.starts_with(root.join("docs/adr"));
        let text = fs::read_to_string(&path).unwrap_or_default();
        for (at, _) in text.match_indices(".md") {
            let head = &text[..at];
            let start = head
                .rfind(|c: char| !(c.is_ascii_alphanumeric() || c == '-'))
                .map_or(0, |i| i + 1);
            let name = format!("{}.md", &head[start..]);
            let before = &head[..start];
            let linked = before.ends_with("adr/") || (in_adr && before.ends_with("]("));
            if linked && has_number(&name) && !files.contains(&name) {
                let rel = path.strip_prefix(&root).unwrap_or(&path);
                broken.push(format!("{}: {name}", rel.display()));
            }
        }
    }
    assert!(broken.is_empty(), "ADR links with no file: {broken:#?}");
}
