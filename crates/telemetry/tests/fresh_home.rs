// SPDX-FileCopyrightText: 2026 Igor Santos
// SPDX-License-Identifier: Apache-2.0

//! A home with no files at all: every command starts clean, creates the data
//! file owner-only, and prints nothing on stderr.

use std::path::PathBuf;
use std::process::{Command, Output};

fn fresh_home(tag: &str) -> PathBuf {
    let home = std::env::temp_dir().join(format!(
        "playbook-telemetry-fresh-{}-{tag}",
        std::process::id()
    ));
    let _ = std::fs::remove_dir_all(&home);
    std::fs::create_dir_all(&home).expect("create home");
    home
}

fn run(home: &PathBuf, args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_playbook-telemetry"))
        .env_remove("PLAYBOOK_TELEMETRY_TZ")
        .args(args)
        .env("HOME", home)
        .env_remove("CLAUDE_CONFIG_DIR")
        .env_remove("XDG_STATE_HOME")
        .env_remove("XDG_CONFIG_HOME")
        .output()
        .expect("run the binary")
}

#[test]
fn every_command_creates_the_data_file_and_stays_quiet() {
    for (tag, args) in [
        ("summary", vec!["--summary"]),
        ("json", vec!["--json"]),
        ("status", vec!["status"]),
        ("prune", vec!["prune", "--dry-run"]),
    ] {
        let home = fresh_home(tag);
        let out = run(&home, &args);
        assert!(out.status.success(), "{tag}: {out:?}");
        assert!(
            out.stderr.is_empty(),
            "{tag}: stderr was {}",
            String::from_utf8_lossy(&out.stderr)
        );
        let db = home.join(".config/playbook/data/usage.db");
        assert!(db.is_file(), "{tag}: no data file");
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode =
                |p: &std::path::Path| std::fs::metadata(p).unwrap().permissions().mode() & 0o777;
            assert_eq!(mode(&db), 0o600, "{tag}");
            assert_eq!(mode(db.parent().unwrap()), 0o700, "{tag}");
        }
    }
}

#[test]
fn the_summary_of_a_fresh_home_says_nothing_is_recorded() {
    let home = fresh_home("empty-summary");
    let out = run(&home, &["--summary"]);
    let text = String::from_utf8_lossy(&out.stdout).to_lowercase();
    assert!(text.contains("no usage recorded yet"), "{text}");
}

#[test]
fn an_old_database_is_moved_once_and_announced_once() {
    let home = fresh_home("move");
    let old = home.join(".config/playbook/usage.db");
    std::fs::create_dir_all(old.parent().unwrap()).unwrap();
    // A database from an older release: a table the current schema grows from.
    let conn = rusqlite_free_db(&old);
    drop(conn);
    let first = run(&home, &["--summary"]);
    assert!(first.status.success(), "{first:?}");
    assert!(!old.exists());
    assert!(home.join(".config/playbook/data/usage.db").is_file());
    let second = run(&home, &["--summary"]);
    assert!(
        second.stderr.is_empty(),
        "{}",
        String::from_utf8_lossy(&second.stderr)
    );
}

/// Creates an empty file: the opener turns it into a database.
fn rusqlite_free_db(path: &PathBuf) -> std::fs::File {
    std::fs::File::create(path).expect("create old db")
}
