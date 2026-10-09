// SPDX-FileCopyrightText: 2026 Igor Santos
// SPDX-License-Identifier: Apache-2.0

//! `install.sh` only installs the Claude Code plugin when the `claude` CLI is
//! at least `CLAUDE_MIN_VERSION`. A stub `claude` records which `plugin`
//! subcommands the installer calls.

#[path = "support/shell_fixture.rs"]
mod fx;

use fx::{bin_dir_with_playbook, repo_root, stderr, stdout, write_exec, Work};
use std::fs;
use std::path::PathBuf;
use std::process::Output;

fn min_version() -> String {
    let text = fs::read_to_string(repo_root().join("install.sh")).unwrap();
    text.lines()
        .find_map(|l| {
            l.strip_prefix("CLAUDE_MIN_VERSION=\"")
                .and_then(|r| r.strip_suffix('"'))
        })
        .expect("CLAUDE_MIN_VERSION in install.sh")
        .to_string()
}

struct Run {
    out: Output,
    calls: String,
    home: PathBuf,
    _work: Work,
}

impl Run {
    fn log(&self) -> String {
        format!("{}{}", stdout(&self.out), stderr(&self.out))
    }
}

/// `version_script` is the shell for `claude --version`.
fn run(tag: &str, version_script: &str) -> Run {
    let work = Work::new(tag);
    let src = work.dir("src");
    let home = work.dir("home");
    let claude_dir = work.dir("claude-bin");
    let bin = bin_dir_with_playbook(&work);
    let call_log = work.path("claude-calls.log");
    fs::write(&call_log, "").unwrap();
    write_exec(
        &claude_dir.join("claude"),
        &format!(
            "case \"${{1:-}}\" in\n  --version) {version_script} ;;\n  plugin) printf '%s\\n' \"$*\" >> \"{}\" ;;\nesac\nexit 0\n",
            call_log.display()
        ),
    );
    let path = format!(
        "{}:{}",
        claude_dir.display(),
        std::env::var("PATH").unwrap_or_default()
    );
    let mut cmd = std::process::Command::new("bash");
    cmd.arg(repo_root().join("install.sh"))
        .arg("--yes")
        .env_remove("SHELL")
        .env("PLAYBOOK_SRC", &src)
        .env("PLAYBOOK_BIN_DIR", &bin)
        .env("CLAUDE_HOME", home.join(".claude"))
        .env("HOME", &home)
        .env("PATH", path);
    let out = cmd.output().expect("bash spawns");
    let calls = fs::read_to_string(&call_log).unwrap_or_default();
    Run {
        out,
        calls,
        home,
        _work: work,
    }
}

fn version(v: &str) -> String {
    format!("printf '%s (Claude Code)\\n' \"{v}\"")
}

#[test]
fn claude_below_the_minimum_skips_the_plugin_with_a_clear_warning() {
    let r = run("gate-below", &version("2.0.0"));
    assert!(r.out.status.success(), "{}", r.log());
    assert!(r.log().contains("2.0.0"), "{}", r.log());
    assert!(r.log().contains(&min_version()), "{}", r.log());
    assert!(r.calls.is_empty(), "plugin called: {}", r.calls);
}

#[test]
fn claude_at_exactly_the_minimum_proceeds() {
    let r = run("gate-atmin", &version(&min_version()));
    assert!(r.out.status.success(), "{}", r.log());
    assert!(r.calls.contains("marketplace add"), "{}", r.calls);
    assert!(r.calls.contains("install"), "{}", r.calls);
}

#[test]
fn claude_above_the_minimum_proceeds() {
    let r = run("gate-above", &version("2.1.269"));
    assert!(r.out.status.success(), "{}", r.log());
    assert!(r.calls.contains("marketplace add"), "{}", r.calls);
}

#[test]
fn an_unparseable_version_degrades_to_the_same_skip_as_too_old() {
    let r = run("gate-unparse", &version("dev-build"));
    assert!(r.out.status.success(), "{}", r.log());
    assert!(r.log().contains("dev-build"), "{}", r.log());
    assert!(r.calls.is_empty(), "plugin called: {}", r.calls);
}

#[test]
fn a_version_probe_that_exits_nonzero_does_not_abort_the_installer() {
    let r = run("gate-probe", "exit 1");
    assert!(r.out.status.success(), "{}", r.log());
    assert!(
        r.home.join(".claude/settings.json").exists(),
        "installer died before finishing: {}",
        r.log()
    );
    assert!(
        r.log()
            .contains("could not determine the installed claude CLI's version"),
        "{}",
        r.log()
    );
    assert!(r.calls.is_empty(), "plugin called: {}", r.calls);
}
