// SPDX-FileCopyrightText: 2026 Igor Santos
// SPDX-License-Identifier: Apache-2.0

//! `playbook init` is the deprecated name for `playbook setup` (ADR-0040).
//! Each test runs both names in their own scratch `$HOME` and checks they
//! write the same files, print the same output and exit with the same code.
//! Without a terminal, `init` prints no deprecation line. The line itself is
//! covered by the unit tests in `lib.rs`, since it needs a terminal.

use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::atomic::{AtomicU64, Ordering};

static COUNTER: AtomicU64 = AtomicU64::new(0);

struct Run {
    code: Option<i32>,
    stdout: String,
    stderr: String,
    files: BTreeMap<String, String>,
}

/// Runs `playbook <name> <args>` in a fresh scratch home, with a cleared
/// environment so nothing reaches the real home through `XDG_*` or
/// `PLAYBOOK_*` variables.
fn run(name: &str, args: &[&str]) -> Run {
    let n = COUNTER.fetch_add(1, Ordering::Relaxed);
    let home = std::env::temp_dir().join(format!(
        "playbook-setup-alias-{name}-{}-{n}",
        playbook::testing::run_id()
    ));
    assert!(home.starts_with(std::env::temp_dir()) && home != std::env::temp_dir());
    let _ = fs::remove_dir_all(&home);
    fs::create_dir_all(home.join(".claude")).expect("scratch home");
    let out = Command::new(env!("CARGO_BIN_EXE_playbook"))
        .arg(name)
        .args(args)
        .env_clear()
        .env("HOME", &home)
        .env("PATH", "/usr/bin:/bin")
        .env("SHELL", "/bin/zsh")
        .env("TMPDIR", &home)
        .env("CLAUDE_PLUGIN_ROOT", env!("PLAYBOOK_REPO_ROOT"))
        .current_dir(&home)
        .output()
        .expect("playbook should spawn");
    let scrub = |text: &str| scrub_tokens(&text.replace(&home.display().to_string(), "{HOME}"));
    let mut files = BTreeMap::new();
    collect(&home, &home, &mut files);
    let files = files
        .into_iter()
        .map(|(path, body)| (path, scrub(&body)))
        .collect();
    let result = Run {
        code: out.status.code(),
        stdout: scrub(&String::from_utf8_lossy(&out.stdout)),
        stderr: scrub(&String::from_utf8_lossy(&out.stderr)),
        files,
    };
    let _ = fs::remove_dir_all(&home);
    result
}

/// Replaces each run of exactly 32 hex digits, such as the random telemetry
/// token, with `{TOKEN}`.
fn scrub_tokens(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut run = String::new();
    let flush = |run: &mut String, out: &mut String| {
        if run.len() == 32 {
            out.push_str("{TOKEN}");
        } else {
            out.push_str(run);
        }
        run.clear();
    };
    for c in text.chars() {
        if c.is_ascii_hexdigit() {
            run.push(c);
        } else {
            flush(&mut run, &mut out);
            out.push(c);
        }
    }
    flush(&mut run, &mut out);
    out
}

/// The rows of every table in a SQLite file, one line each, with times
/// replaced by `{TIME}`, since the raw pages differ from run to run.
fn dump(path: &Path) -> String {
    use rusqlite::types::ValueRef;
    let db = rusqlite::Connection::open(path).expect("open db");
    let mut tables = db
        .prepare("SELECT name FROM sqlite_master WHERE type = 'table' ORDER BY name")
        .expect("list tables");
    let names: Vec<String> = tables
        .query_map([], |r| r.get(0))
        .expect("table names")
        .flatten()
        .collect();
    let mut out = String::new();
    for name in names {
        let mut rows = db
            .prepare(&format!("SELECT * FROM \"{name}\" ORDER BY 1"))
            .expect("select rows");
        let width = rows.column_count();
        let mut it = rows.query([]).expect("rows");
        while let Some(row) = it.next().expect("row") {
            let cells: Vec<String> = (0..width)
                .map(|i| match row.get_ref(i).expect("cell") {
                    ValueRef::Integer(n) if n > 1_000_000_000 => "{TIME}".into(),
                    ValueRef::Text(t) if t.starts_with(b"20") && t.contains(&b':') => {
                        "{TIME}".into()
                    }
                    other => format!("{other:?}"),
                })
                .collect();
            out.push_str(&format!("{name}: {}\n", cells.join(" | ")));
        }
    }
    out
}

/// A backup carries the time in its name, as in `settings.json.bak.1792073671`.
/// Replaces each run of nine or more digits with `{TIME}`.
fn stamp_free(name: &str) -> String {
    let mut out = String::new();
    let mut digits = String::new();
    for c in name.chars().chain(std::iter::once('\0')) {
        if c.is_ascii_digit() {
            digits.push(c);
            continue;
        }
        out.push_str(if digits.len() >= 9 { "{TIME}" } else { &digits });
        digits.clear();
        if c != '\0' {
            out.push(c);
        }
    }
    out
}

/// Every regular file under `dir`, keyed by its path relative to `root`.
fn collect(root: &Path, dir: &Path, out: &mut BTreeMap<String, String>) {
    let Ok(entries) = fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let path: PathBuf = entry.path();
        if path.is_dir() {
            collect(root, &path, out);
            continue;
        }
        let rel = stamp_free(&path.strip_prefix(root).unwrap().display().to_string());
        let body = if rel.ends_with(".db") {
            dump(&path)
        } else {
            fs::read(&path)
                .map(|b| String::from_utf8_lossy(&b).into_owned())
                .unwrap_or_default()
        };
        out.insert(rel, body);
    }
}

fn assert_same(args: &[&str]) -> Run {
    let setup = run("setup", args);
    let init = run("init", args);
    assert_eq!(setup.code, init.code, "exit codes differ for {args:?}");
    assert_eq!(setup.stdout, init.stdout, "stdout differs for {args:?}");
    assert_eq!(setup.stderr, init.stderr, "stderr differs for {args:?}");
    assert_eq!(
        setup.files.keys().collect::<Vec<_>>(),
        init.files.keys().collect::<Vec<_>>(),
        "different files for {args:?}"
    );
    for (path, body) in &setup.files {
        assert_eq!(
            Some(body),
            init.files.get(path),
            "{path} differs for {args:?}"
        );
    }
    assert!(
        !init.stderr.contains("deprecated") && !init.stdout.contains("deprecated"),
        "init printed a deprecation line without a terminal: {}{}",
        init.stdout,
        init.stderr
    );
    setup
}

#[test]
fn a_full_run_writes_the_same_files_under_both_names() {
    let setup = assert_same(&["--yes", "--no-telemetry"]);
    assert!(setup.code == Some(0) && setup.files.contains_key(".claude/settings.json"));
}

#[test]
fn a_run_with_part_flags_writes_the_same_files_under_both_names() {
    assert_same(&["--no-hooks", "--statusline", "--security", "--no-telemetry"]);
}

#[test]
fn a_run_without_a_terminal_or_yes_matches_under_both_names() {
    assert_same(&[]);
}

#[test]
fn a_dry_run_and_explain_match_under_both_names() {
    assert_same(&["--dry-run"]);
    assert_same(&["--explain"]);
    assert_same(&["--explain", "hooks"]);
}

#[test]
fn only_telemetry_and_short_name_match_under_both_names() {
    assert_same(&["--only", "telemetry"]);
    assert_same(&["--only", "short-name", "--short-name"]);
}

#[test]
fn errors_exit_with_the_same_code_under_both_names() {
    assert_eq!(assert_same(&["--only", "no-such-part"]).code, Some(2));
    assert_eq!(assert_same(&["--explain", "no-such-part"]).code, Some(2));
}

#[test]
fn a_flag_conflict_exits_two_under_both_names() {
    // clap names the command in its usage line, so only the code is compared.
    let setup = run("setup", &["--dry-run", "--explain"]);
    let init = run("init", &["--dry-run", "--explain"]);
    assert_eq!(setup.code, Some(2));
    assert_eq!(init.code, Some(2));
}
