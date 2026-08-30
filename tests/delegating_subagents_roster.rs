// SPDX-FileCopyrightText: 2026 Igor Santos
// SPDX-License-Identifier: MIT

//! Pins the roster table in `skills/delegating-subagents/SKILL.md` against
//! `agents/*.md`. That table went stale once, listing 7 of the 12 real
//! agents (missing `auditor`, `cheap-checker`, `patch-applier`, and
//! `review-triage` entirely) with no automated check to catch the drift,
//! only a manual re-read. This test is that check: it fails CI the next
//! time an agent is added or removed without the table being updated to
//! match, rather than relying on someone remembering the table exists.

use std::collections::BTreeSet;
use std::fs;
use std::path::PathBuf;

fn repo_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
}

/// The real agent roster: every `name:` value in `agents/*.md`, excluding
/// a `_TEMPLATE.md` if one exists (not a real agent, per the convention
/// `tests/agents_check.rs` already follows for the same directory).
fn real_agent_names() -> BTreeSet<String> {
    let dir = repo_root().join("agents");
    let mut names = BTreeSet::new();
    for entry in fs::read_dir(&dir).expect("agents/ should exist").flatten() {
        let path = entry.path();
        if path.extension().is_none_or(|e| e != "md") {
            continue;
        }
        if path.file_name().and_then(|f| f.to_str()) == Some("_TEMPLATE.md") {
            continue;
        }
        let text = fs::read_to_string(&path).expect("agent file should be readable");
        let name = text
            .lines()
            .find_map(|l| l.strip_prefix("name: "))
            .unwrap_or_else(|| panic!("{}: no top-level 'name:' frontmatter key", path.display()))
            .trim()
            .to_string();
        names.insert(name);
    }
    names
}

/// Agent names listed in the roster table's first column (backtick-quoted,
/// one per row) in `skills/delegating-subagents/SKILL.md`. The table has a
/// `subagent_type` column too, checked separately below.
fn table_rows() -> Vec<(String, String)> {
    let path = repo_root().join("skills/delegating-subagents/SKILL.md");
    let text = fs::read_to_string(&path).expect("SKILL.md should be readable");

    let mut rows = Vec::new();
    let mut in_table = false;
    for line in text.lines() {
        if line.starts_with("| Agent |") {
            in_table = true;
            continue;
        }
        if !in_table {
            continue;
        }
        if line.starts_with("|---") {
            continue; // header separator row
        }
        if !line.starts_with('|') {
            break; // table ended
        }
        let cells: Vec<&str> = line.split('|').map(str::trim).collect();
        // cells[0] is empty (before the leading `|`); name is cells[1], subagent_type is cells[2].
        let name = cells
            .get(1)
            .unwrap_or(&"")
            .trim_matches('`')
            .trim()
            .to_string();
        let subagent_type = cells
            .get(2)
            .unwrap_or(&"")
            .trim_matches('`')
            .trim()
            .to_string();
        if name.is_empty() {
            continue;
        }
        rows.push((name, subagent_type));
    }
    rows
}

#[test]
fn roster_table_lists_every_real_agent_and_no_others() {
    let real = real_agent_names();
    assert!(
        real.len() >= 10,
        "scanned {} agent files, expected at least 10; the agents/ directory walk is broken",
        real.len()
    );

    let rows = table_rows();
    assert!(
        !rows.is_empty(),
        "found no rows in the delegating-subagents roster table; the table's header \
         line ('| Agent |...') may have changed shape and this parser needs updating"
    );

    let tabled: BTreeSet<String> = rows.iter().map(|(name, _)| name.clone()).collect();

    let missing_from_table: Vec<&String> = real.difference(&tabled).collect();
    let stale_in_table: Vec<&String> = tabled.difference(&real).collect();

    assert!(
        missing_from_table.is_empty() && stale_in_table.is_empty(),
        "skills/delegating-subagents/SKILL.md's roster table is out of sync with agents/*.md.\n\
         Missing from the table (real agents, not listed): {missing_from_table:?}\n\
         Stale in the table (listed, but no such agent file exists): {stale_in_table:?}"
    );
}

#[test]
fn every_table_row_names_its_playbook_prefixed_subagent_type() {
    let rows = table_rows();
    assert!(!rows.is_empty(), "found no rows in the roster table");

    let mut failures = Vec::new();
    for (name, subagent_type) in &rows {
        let expected = format!("playbook:{name}");
        if subagent_type != &expected {
            failures.push(format!(
                "'{name}' row has subagent_type '{subagent_type}', expected '{expected}'"
            ));
        }
    }
    assert!(
        failures.is_empty(),
        "roster table rows with a wrong or missing playbook: prefix on subagent_type \
         (a bare, unprefixed value resolves to the wrong or no agent, the class of bug \
         fixed repo-wide in #284):\n{}",
        failures.join("\n")
    );
}
