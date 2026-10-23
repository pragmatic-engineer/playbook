// SPDX-FileCopyrightText: 2026 Igor Santos
// SPDX-License-Identifier: Apache-2.0

//! Proves that the flag attributes listed in `golden/flags.txt` did not change.
//!
//! No test here writes the golden. It changes only through
//! `cargo run -p playbook --example regen_flags`.

use clap::{Arg, ArgAction, Command, CommandFactory};
use playbook::flags_snapshot::{render_flags_snapshot, write_flags_snapshot};
use playbook::Cli;
use std::path::PathBuf;

const GOLDEN: &str = include_str!("golden/flags.txt");

fn sample(default: &'static str, alias: Option<&'static str>, global: bool) -> Command {
    let mut flag = Arg::new("limit")
        .long("limit")
        .default_value(default)
        .global(global);
    if let Some(name) = alias {
        flag = flag.alias(name);
    }
    Command::new("x").arg(flag)
}

fn temp_path(name: &str) -> PathBuf {
    std::env::temp_dir().join(format!("flags-snapshot-{}-{name}", std::process::id()))
}

#[test]
fn the_flags_match_the_snapshot() {
    let live = render_flags_snapshot(&Cli::command());
    assert!(
        live == GOLDEN,
        "flags.txt is out of date. If the change is deliberate, run \
         `cargo run -p playbook --example regen_flags` and label the PR `flags-change`."
    );
}

#[test]
fn the_snapshot_catches_a_changed_flag_name() {
    let before = render_flags_snapshot(&sample("20", None, false));
    let after = render_flags_snapshot(
        &Command::new("x").arg(Arg::new("limit").long("max").default_value("20")),
    );
    assert_ne!(before, after);
}

#[test]
fn the_snapshot_catches_a_changed_default() {
    assert_ne!(
        render_flags_snapshot(&sample("20", None, false)),
        render_flags_snapshot(&sample("30", None, false))
    );
}

#[test]
fn the_snapshot_catches_an_added_alias() {
    assert_ne!(
        render_flags_snapshot(&sample("20", None, false)),
        render_flags_snapshot(&sample("20", Some("cap"), false))
    );
}

#[test]
fn the_snapshot_catches_a_changed_global_setting() {
    assert_ne!(
        render_flags_snapshot(&sample("20", None, false)),
        render_flags_snapshot(&sample("20", None, true))
    );
}

#[test]
fn the_snapshot_catches_a_changed_action() {
    let set = Command::new("x").arg(Arg::new("f").long("f").action(ArgAction::SetTrue));
    let count = Command::new("x").arg(Arg::new("f").long("f").action(ArgAction::Count));
    assert_ne!(render_flags_snapshot(&set), render_flags_snapshot(&count));
}

#[test]
fn the_snapshot_ignores_help_text() {
    let one = Command::new("x").arg(Arg::new("f").long("f").help("One"));
    let two = Command::new("x").arg(Arg::new("f").long("f").help("Two"));
    assert_eq!(render_flags_snapshot(&one), render_flags_snapshot(&two));
}

#[test]
fn the_writer_writes_the_render_and_is_repeatable() {
    let path = temp_path("write.txt");
    let cmd = sample("20", None, false);
    write_flags_snapshot(&cmd, &path).unwrap();
    let first = std::fs::read(&path).unwrap();
    write_flags_snapshot(&cmd, &path).unwrap();
    let second = std::fs::read(&path).unwrap();
    std::fs::remove_file(&path).unwrap();
    assert_eq!(first, render_flags_snapshot(&cmd).into_bytes());
    assert_eq!(first, second);
}
