// SPDX-FileCopyrightText: 2026 Igor Santos
// SPDX-License-Identifier: MIT

//! Behavioural tests for the ported safety guards.
//!
//! Both directions are asserted for every case. A guard that only proves it
//! blocks can pass while blocking everything, which breaks the tool it guards;
//! one that only proves it allows can pass while guarding nothing.

use std::fs;
use std::path::PathBuf;
use std::process::Command;
use std::sync::atomic::{AtomicU64, Ordering};

static COUNTER: AtomicU64 = AtomicU64::new(0);

fn scratch(tag: &str) -> PathBuf {
    let n = COUNTER.fetch_add(1, Ordering::Relaxed);
    let dir =
        std::env::temp_dir().join(format!("playbook-guards-{tag}-{}-{n}", std::process::id()));
    fs::create_dir_all(&dir).expect("scratch dir");
    dir
}

fn run_guard(name: &str, command: &str) -> String {
    let payload = serde_json::json!({ "tool_input": { "command": command } }).to_string();
    let out = Command::new(env!("CARGO_BIN_EXE_playbook"))
        .args(["hook", name])
        .env("HOOK_INPUT", payload)
        .output()
        .expect("playbook binary should spawn");
    assert!(
        out.status.success(),
        "a guard must exit 0 even when denying, or it breaks the hook contract"
    );
    String::from_utf8_lossy(&out.stdout).to_string()
}

fn blocks(command: &str) -> bool {
    let out = run_guard("no-dash-guard", command);
    if out.trim().is_empty() {
        return false;
    }
    assert!(
        out.contains(r#""permissionDecision":"deny""#),
        "non-empty output must be a deny decision: {out}"
    );
    true
}

mod bg_await_guard {
    use super::*;

    /// Returns true when the guard emitted its nudge. It must never deny:
    /// backgrounding a long job and awaiting its exit is legitimate, so a block
    /// here would break a valid workflow.
    fn warns(command: &str, background: bool) -> bool {
        let payload = serde_json::json!({
            "tool_input": { "command": command, "run_in_background": background }
        })
        .to_string();
        let out = Command::new(env!("CARGO_BIN_EXE_playbook"))
            .args(["hook", "bg-await-guard"])
            .env("HOOK_INPUT", payload)
            .output()
            .expect("playbook binary should spawn");
        assert!(out.status.success(), "a guard must exit 0");
        let stdout = String::from_utf8_lossy(&out.stdout);
        if stdout.trim().is_empty() {
            return false;
        }
        assert!(
            !stdout.contains(r#""permissionDecision""#),
            "this guard warns, it must never deny: {stdout}"
        );
        true
    }

    #[test]
    fn backgrounded_await_sensitive_commands_warn() {
        for cmd in [
            "npm install",
            "rm -rf node_modules package-lock.json && npm install",
            "pnpm install --frozen-lockfile",
            "yarn install",
            "bun install",
            "npm ci",
            "npm run build",
            "tsc && tsc-alias",
            "make build",
            "cargo build --release",
            "pip install -r requirements.txt",
            "rm -rf node_modules",
        ] {
            assert!(warns(cmd, true), "should warn: {cmd}");
        }
    }

    /// Backgrounding is the trigger, not the command. Without this the guard
    /// could pass while warning on every install anywhere.
    #[test]
    fn the_same_commands_in_the_foreground_stay_quiet() {
        for cmd in [
            "npm install",
            "npm run build",
            "rm -rf node_modules && npm install",
        ] {
            assert!(!warns(cmd, false), "foreground must be quiet: {cmd}");
        }
    }

    #[test]
    fn long_running_watches_are_left_alone() {
        for cmd in [
            "npm run dev",
            "vite --host",
            "tail -f /var/log/app.log",
            "node server.js",
        ] {
            assert!(
                !warns(cmd, true),
                "backgrounding a watch is the correct use: {cmd}"
            );
        }
    }

    /// `tsc` and `make` only count at a command start, so an argument that
    /// happens to spell one does not trip the guard.
    #[test]
    fn build_tools_only_match_at_a_command_start() {
        assert!(!warns("echo tsc", true));
        assert!(!warns("grep make Makefile", true));
        assert!(warns("a; tsc", true), "after a separator it is a command");
    }

    #[test]
    fn the_env_switch_disables_the_guard() {
        let payload = serde_json::json!({
            "tool_input": { "command": "npm install", "run_in_background": true }
        })
        .to_string();
        let out = Command::new(env!("CARGO_BIN_EXE_playbook"))
            .args(["hook", "bg-await-guard"])
            .env("HOOK_INPUT", payload)
            .env("BG_AWAIT_GUARD", "0")
            .output()
            .expect("spawn");
        assert!(String::from_utf8_lossy(&out.stdout).trim().is_empty());
    }

    #[test]
    fn malformed_payloads_exit_silently() {
        for raw in [
            "",
            "{",
            "null",
            r#"{"tool_input":{}}"#,
            r#"{"tool_input":{"command":"npm install"}}"#,
        ] {
            let out = Command::new(env!("CARGO_BIN_EXE_playbook"))
                .args(["hook", "bg-await-guard"])
                .env("HOOK_INPUT", raw)
                .output()
                .expect("spawn");
            assert!(out.status.success(), "must exit 0 on: {raw:?}");
            assert!(
                String::from_utf8_lossy(&out.stdout).trim().is_empty(),
                "must stay silent on: {raw:?}"
            );
        }
    }
}

mod no_dash_guard {
    use super::*;

    /// U+2012 figure, U+2013 en, U+2014 em, U+2015 horizontal bar. The shell
    /// suite only ever exercised en and em, so the outer two were unproven.
    const DASH_FAMILY: [char; 4] = ['\u{2012}', '\u{2013}', '\u{2014}', '\u{2015}'];

    #[test]
    fn every_dash_in_the_family_blocks_a_posting_command() {
        for dash in DASH_FAMILY {
            let cmd = format!(r#"git commit -m "fix: a {dash} b""#);
            assert!(
                blocks(&cmd),
                "U+{:04X} must block, it is in the banned range",
                dash as u32
            );
        }
    }

    #[test]
    fn posting_commands_with_an_inline_dash_block() {
        for cmd in [
            "gh pr edit 42 --title \"feat: do a thing \u{2014} really\"",
            "gh pr create --title \"fix: a \u{2013} b\" --body \"x\"",
            "git commit -m \"fix: stop stale reads \u{2014} after invalidation\"",
            "git tag -a v1.0 -m \"release \u{2013} first\"",
            "gh pr comment 42 --body \"nice work \u{2014} ship it\"",
        ] {
            assert!(blocks(cmd), "should block: {cmd}");
        }
    }

    #[test]
    fn a_dash_inside_a_referenced_body_file_blocks() {
        let dir = scratch("bodyfile");
        let dashed = dir.join("dash.md");
        let clean = dir.join("clean.md");
        fs::write(&dashed, "Summary line with an em dash \u{2014} here.\n").expect("write");
        fs::write(&clean, "Summary line, clean, no dashes here.\n").expect("write");
        let (d, c) = (dashed.display(), clean.display());

        for cmd in [
            format!("gh pr create --body-file {d} --title \"clean title\""),
            format!("gh pr edit 42 --body-file={d}"),
            format!("git commit -F {d}"),
            format!("gh api -X POST /repos/o/r/pulls/1/reviews --input {d}"),
        ] {
            assert!(blocks(&cmd), "should block: {cmd}");
        }

        // The same commands with a clean file must pass, otherwise the rule
        // above would be satisfied by blocking whenever a file flag appears.
        for cmd in [
            format!("gh pr create --body-file {c} --title \"clean title\""),
            format!("git commit -F {c}"),
        ] {
            assert!(!blocks(&cmd), "should allow: {cmd}");
        }
    }

    #[test]
    fn clean_posting_commands_pass() {
        for cmd in [
            "gh pr edit 42 --title \"feat: do a thing - really\"",
            "git commit -m \"fix: stop stale reads after invalidation\"",
        ] {
            assert!(!blocks(cmd), "an ascii hyphen is fine: {cmd}");
        }
    }

    #[test]
    fn non_posting_commands_are_never_guarded() {
        for cmd in [
            "echo \"a \u{2014} b\"",
            "grep -n \"\u{2013}\" notes.md",
            "gh pr view 42",
            "gh pr diff 42",
        ] {
            assert!(
                !blocks(cmd),
                "the guard covers posting only, not every command carrying a dash: {cmd}"
            );
        }
    }

    #[test]
    fn a_missing_body_file_does_not_block() {
        // The guard catches prose it can read. A path it cannot open is the
        // shell's error to report, and blocking here would be a false deny.
        assert!(!blocks("gh pr create --body-file /nonexistent/nope.md"));
    }

    #[test]
    fn the_env_switch_disables_the_guard() {
        let payload =
            serde_json::json!({ "tool_input": { "command": "git commit -m \"a \u{2014} b\"" } })
                .to_string();
        let out = Command::new(env!("CARGO_BIN_EXE_playbook"))
            .args(["hook", "no-dash-guard"])
            .env("HOOK_INPUT", payload)
            .env("NO_DASH_GUARD", "0")
            .output()
            .expect("spawn");
        assert!(String::from_utf8_lossy(&out.stdout).trim().is_empty());
    }

    #[test]
    fn malformed_and_empty_payloads_exit_silently() {
        // These run on the PreToolUse hot path, so a panic breaks a live
        // session. Anything unparseable must pass rather than fail.
        for raw in [
            "",
            "{",
            "null",
            r#"{"tool_input":{}}"#,
            r#"{"tool_input":null}"#,
        ] {
            let out = Command::new(env!("CARGO_BIN_EXE_playbook"))
                .args(["hook", "no-dash-guard"])
                .env("HOOK_INPUT", raw)
                .output()
                .expect("spawn");
            assert!(out.status.success(), "must exit 0 on: {raw:?}");
            assert!(
                String::from_utf8_lossy(&out.stdout).trim().is_empty(),
                "must stay silent on: {raw:?}"
            );
        }
    }
}
