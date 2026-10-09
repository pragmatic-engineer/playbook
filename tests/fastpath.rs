// SPDX-FileCopyrightText: 2026 Igor Santos
// SPDX-License-Identifier: Apache-2.0

//! The pre-clap fast path must agree with clap on every form it handles, and
//! hand everything else to clap unchanged.

use clap::{CommandFactory, Parser, ValueEnum};
use playbook::fastpath::{classify, version_text, Fast};
use playbook::{Cli, Command, HookName};
use std::ffi::OsString;
use std::process::Command as Proc;

fn argv(parts: &[&str]) -> Vec<OsString> {
    parts.iter().map(OsString::from).collect()
}

fn name_of(v: HookName) -> String {
    v.to_possible_value().unwrap().get_name().to_string()
}

#[test]
fn every_hook_name_resolves_to_the_same_hook_as_clap() {
    for v in HookName::value_variants() {
        let name = name_of(*v);
        let fast = classify(&argv(&["playbook", "hook", &name]));
        let Some(Fast::Hook(fast)) = fast else {
            panic!("{name} was not recognised");
        };
        let cli = Cli::try_parse_from(["playbook", "hook", &name]).unwrap();
        let Command::Hook { name: clap_name } = cli.command else {
            panic!("clap parsed {name} as another command");
        };
        assert_eq!(name_of(fast), name_of(clap_name), "{name}");
    }
}

#[test]
fn a_form_the_fast_path_skips_is_still_parsed_or_rejected_by_clap() {
    for parts in [
        &["playbook", "hook", "nope"][..],
        &["playbook", "hook", "session-init", "extra"],
        &["playbook", "statusline", "--bogus"],
    ] {
        assert!(classify(&argv(parts)).is_none(), "{parts:?}");
        assert!(Cli::try_parse_from(parts).is_err(), "{parts:?}");
    }
}

#[test]
fn version_text_equals_what_clap_renders() {
    assert_eq!(version_text(), Cli::command().render_version());
}

fn run(args: &[&str]) -> (i32, String, String) {
    let out = Proc::new(env!("CARGO_BIN_EXE_playbook"))
        .args(args)
        .output()
        .unwrap();
    (
        out.status.code().unwrap_or(-1),
        String::from_utf8_lossy(&out.stdout).into_owned(),
        String::from_utf8_lossy(&out.stderr).into_owned(),
    )
}

#[test]
fn the_binary_prints_the_same_for_the_flag_and_the_clap_subcommand() {
    let fast = run(&["--version"]);
    let short = run(&["-V"]);
    let clap = run(&["version"]);
    assert_eq!(fast, clap);
    assert_eq!(short, clap);
}

#[test]
fn help_and_error_output_still_come_from_clap() {
    let (code, out, _) = run(&["hook", "--help"]);
    assert_eq!(code, 0);
    assert!(out.contains("Usage"), "{out}");
    let (code, _, err) = run(&["hook", "nope"]);
    assert_ne!(code, 0);
    assert!(err.contains("invalid value"), "{err}");
}
