// SPDX-FileCopyrightText: 2026 Igor Santos
// SPDX-License-Identifier: Apache-2.0

//! Behavioural tests for the ported safety guards.
//!
//! Both directions are asserted for every case. A guard that only proves it
//! blocks can pass while blocking everything, which breaks the tool it guards;
//! one that only proves it allows can pass while guarding nothing.

use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicU64, Ordering};

static COUNTER: AtomicU64 = AtomicU64::new(0);

/// Scratch space for fixtures, deliberately NOT `std::env::temp_dir()`.
///
/// The guard always allows paths inside `/tmp`, and on Linux `temp_dir()` IS
/// `/tmp`. A fixture placed there would be allowed no matter what the root
/// derivation did, so every "this is allowed" assertion built on one would pass
/// even with root handling completely broken, while still looking meaningful on
/// macOS where `temp_dir()` is `/var/folders`.
fn scratch(tag: &str) -> PathBuf {
    let n = COUNTER.fetch_add(1, Ordering::Relaxed);
    let base = PathBuf::from(std::env::var("HOME").expect("HOME")).join(".cache/playbook-tests");
    let dir = base.join(format!(
        "playbook-guards-{tag}-{}-{n}",
        playbook::testing::run_id()
    ));
    fs::create_dir_all(&dir).expect("scratch dir");
    dir
}

fn run_guard(name: &str, command: &str) -> String {
    let payload = serde_json::json!({ "tool_input": { "command": command } }).to_string();
    let out = Command::new(env!("CARGO_BIN_EXE_playbook"))
        .env_remove("CI")
        .env_remove("PLAYBOOK_HEADLESS")
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

/// Pipes the payload over stdin instead of setting `HOOK_INPUT`, the contract
/// the installed hooks use. `HOOK_INPUT` is explicitly removed so a leftover in the test
/// process's own environment cannot mask a stdin regression.
fn run_guard_via_stdin(name: &str, command: &str) -> String {
    let payload = serde_json::json!({ "tool_input": { "command": command } }).to_string();
    let mut child = Command::new(env!("CARGO_BIN_EXE_playbook"))
        .env_remove("CI")
        .env_remove("PLAYBOOK_HEADLESS")
        .args(["hook", name])
        .env_remove("HOOK_INPUT")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .spawn()
        .expect("playbook binary should spawn");
    child
        .stdin
        .take()
        .expect("stdin should be piped")
        .write_all(payload.as_bytes())
        .expect("payload should write to stdin");
    let out = child
        .wait_with_output()
        .expect("playbook binary should exit");
    assert!(
        out.status.success(),
        "a guard must exit 0 even when denying, or it breaks the hook contract"
    );
    String::from_utf8_lossy(&out.stdout).to_string()
}

fn blocks(command: &str) -> bool {
    let out = run_guard("no-slop-guard", command);
    if out.trim().is_empty() {
        return false;
    }
    assert!(
        out.contains(r#""permissionDecision":"deny""#),
        "non-empty output must be a deny decision: {out}"
    );
    true
}

mod rm_workspace_guard {
    use super::*;

    /// Runs the hook with `HOME` set to a scratch folder, so a command that
    /// names the home folder names the scratch one. Nothing is executed.
    fn run_in_home(command: &str, home: &Path) -> String {
        let payload = serde_json::json!({ "tool_input": { "command": command } }).to_string();
        let out = Command::new(env!("CARGO_BIN_EXE_playbook"))
            .env_remove("CI")
            .env_remove("PLAYBOOK_HEADLESS")
            .env("HOME", home)
            .args(["hook", "rm-workspace-guard"])
            .env("HOOK_INPUT", payload)
            .output()
            .expect("playbook binary should spawn");
        assert!(
            out.status.success(),
            "a guard must exit 0 even when denying"
        );
        String::from_utf8_lossy(&out.stdout).to_string()
    }

    fn denied(command: &str, home: &Path) -> bool {
        let out = run_in_home(command, home);
        if out.trim().is_empty() {
            return false;
        }
        assert!(
            out.contains(r#""permissionDecision":"deny""#),
            "non-empty output must be a deny decision: {out}"
        );
        true
    }

    #[test]
    fn root_and_home_deletions_are_denied() {
        let home = scratch("rm-deny");
        let h = home.display();
        for command in [
            "rm -rf /".to_string(),
            "rm -rf /*".to_string(),
            "rm -rf ~".to_string(),
            "rm -rf $HOME/".to_string(),
            format!("rm -rf {h}"),
            format!("sudo rm -rf --no-preserve-root {h}/*"),
            "echo ok && rm -rf ~/".to_string(),
        ] {
            assert!(denied(&command, &home), "should deny: {command}");
        }
    }

    #[test]
    fn everything_else_is_allowed() {
        let home = scratch("rm-allow");
        let h = home.display();
        for command in [
            "rm -rf ./target".to_string(),
            "rm -rf ~/projects/x/target".to_string(),
            format!("rm -rf {h}/projects/x"),
            "rm file".to_string(),
            "rm -rf /tmp/x".to_string(),
            "rm -rf /Users/other".to_string(),
            "ls /".to_string(),
            "echo rm -rf /".to_string(),
        ] {
            assert!(!denied(&command, &home), "should allow: {command}");
        }
    }

    #[test]
    fn the_denial_is_the_fixed_message() {
        let home = scratch("rm-message");
        let out = run_in_home("rm -rf ~", &home);
        assert!(out.contains("Blocked by the rm guard"), "{out}");
        assert!(out.contains("run the command yourself"), "{out}");
    }

    #[test]
    fn stdin_piped_input_is_read_when_hook_input_is_unset() {
        assert!(!run_guard_via_stdin("rm-workspace-guard", "ls").contains("deny"));
        assert!(run_guard_via_stdin("rm-workspace-guard", "rm -rf /").contains("deny"));
    }

    #[test]
    fn malformed_payloads_exit_silently() {
        for raw in ["", "{", "null", r#"{"tool_input":{}}"#] {
            let out = Command::new(env!("CARGO_BIN_EXE_playbook"))
                .env_remove("CI")
                .env_remove("PLAYBOOK_HEADLESS")
                .args(["hook", "rm-workspace-guard"])
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

mod precommit_check {
    use super::*;

    /// A throwaway repo, so the staged diff under test is fully controlled and
    /// the guard never reads the real working tree.
    fn repo(tag: &str) -> PathBuf {
        let dir = scratch(tag);
        for args in [
            vec!["init", "-q"],
            vec!["config", "user.email", "t@t"],
            vec!["config", "user.name", "T"],
        ] {
            Command::new("git")
                .arg("-C")
                .arg(&dir)
                .args(&args)
                .output()
                .expect("git should run");
        }
        dir
    }

    fn git_in(dir: &PathBuf, args: &[&str]) {
        Command::new("git")
            .arg("-C")
            .arg(dir)
            .args(args)
            .output()
            .expect("git should run");
    }

    /// Runs with the scratch repo as cwd, which is how the guard finds the diff.
    fn warns_in(dir: &PathBuf, command: &str) -> bool {
        let payload = serde_json::json!({ "tool_input": { "command": command } }).to_string();
        let out = Command::new(env!("CARGO_BIN_EXE_playbook"))
            .env_remove("CI")
            .env_remove("PLAYBOOK_HEADLESS")
            .args(["hook", "precommit-check"])
            .current_dir(dir)
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
    fn only_a_real_commit_is_inspected() {
        let dir = repo("not-commit");
        fs::write(dir.join("a.txt"), "hello\n").expect("write");
        git_in(&dir, &["add", "a.txt"]);
        for cmd in ["git log --oneline -5", "git status", "git commit --help"] {
            assert!(!warns_in(&dir, cmd), "not a commit: {cmd}");
        }
        assert!(
            !warns_in(&dir, "git commit -m 'feat: add a'"),
            "a clean small diff has nothing to report"
        );
    }

    #[test]
    fn nothing_staged_is_quiet() {
        let dir = repo("empty");
        fs::write(dir.join("b.txt"), "untracked\n").expect("write");
        assert!(!warns_in(&dir, "git commit -m 'feat: nothing'"));
    }

    #[test]
    fn secret_shaped_filenames_warn() {
        for (tag, name, add_flag) in [
            ("env", ".env", true),
            ("pem", "server.pem", false),
            ("key", "id_rsa", false),
        ] {
            let dir = repo(tag);
            fs::write(dir.join(name), "secret\n").expect("write");
            if add_flag {
                git_in(&dir, &["add", "-f", name]);
            } else {
                git_in(&dir, &["add", name]);
            }
            assert!(warns_in(&dir, "git commit -m x"), "{name} should warn");
        }
    }

    #[test]
    fn debug_leftovers_in_added_lines_warn() {
        for (tag, file, body) in [
            (
                "js",
                "app.js",
                "function f() {\n  console.log(\"dbg\");\n}\n",
            ),
            ("py", "app.py", "def f():\n    breakpoint()\n"),
        ] {
            let dir = repo(tag);
            fs::write(dir.join(file), body).expect("write");
            git_in(&dir, &["add", file]);
            assert!(warns_in(&dir, "git commit -m x"), "{file} should warn");
        }
    }

    #[test]
    fn oversized_commits_warn_by_lines_and_by_files() {
        let dir = repo("big-lines");
        let body: String = (1..=700).map(|i| format!("line {i}\n")).collect();
        fs::write(dir.join("big.txt"), body).expect("write");
        git_in(&dir, &["add", "big.txt"]);
        assert!(warns_in(&dir, "git commit -m x"), "over 600 changed lines");

        let dir = repo("big-files");
        for i in 1..=25 {
            fs::write(dir.join(format!("f{i}.txt")), "x\n").expect("write");
        }
        git_in(&dir, &["add", "."]);
        assert!(warns_in(&dir, "git commit -m x"), "over 20 files");
    }

    #[test]
    fn the_env_switch_disables_the_guard() {
        let dir = repo("disabled");
        fs::write(dir.join(".env"), "TOKEN=abc\n").expect("write");
        git_in(&dir, &["add", "-f", ".env"]);
        let payload =
            serde_json::json!({ "tool_input": { "command": "git commit -m x" } }).to_string();
        let out = Command::new(env!("CARGO_BIN_EXE_playbook"))
            .env_remove("CI")
            .env_remove("PLAYBOOK_HEADLESS")
            .args(["hook", "precommit-check"])
            .current_dir(&dir)
            .env("HOOK_INPUT", payload)
            .env("PRECOMMIT_CHECK", "0")
            .output()
            .expect("spawn");
        assert!(String::from_utf8_lossy(&out.stdout).trim().is_empty());
    }

    /// A shim that logs each git call, then runs the real git.
    #[cfg(unix)]
    #[test]
    fn the_repo_probe_is_not_a_separate_git_call() {
        use std::os::unix::fs::PermissionsExt;
        let dir = repo("spawn-count");
        fs::write(dir.join("a.txt"), "hello\n").expect("write");
        git_in(&dir, &["add", "a.txt"]);
        let real = String::from_utf8_lossy(
            &Command::new("sh")
                .args(["-c", "command -v git"])
                .output()
                .expect("sh")
                .stdout,
        )
        .trim()
        .to_string();
        let bin = scratch("spawn-count-bin");
        let log = bin.join("calls.log");
        let shim = bin.join("git");
        fs::write(
            &shim,
            format!(
                "#!/bin/sh\necho \"$@\" >> '{}'\nexec '{real}' \"$@\"\n",
                log.display()
            ),
        )
        .expect("shim");
        fs::set_permissions(&shim, fs::Permissions::from_mode(0o755)).expect("chmod");
        // The first run of a new executable can be slow on a scanning machine,
        // longer than the hook waits for git. Pay that cost here, then clear the log.
        let _ = Command::new(&shim)
            .arg("--version")
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .status();
        let _ = fs::remove_file(&log);
        let payload =
            serde_json::json!({ "tool_input": { "command": "git commit -m x" } }).to_string();

        let out = Command::new(env!("CARGO_BIN_EXE_playbook"))
            .env_remove("CI")
            .env_remove("PLAYBOOK_HEADLESS")
            .env(
                "PATH",
                format!(
                    "{}:{}",
                    bin.display(),
                    std::env::var("PATH").unwrap_or_default()
                ),
            )
            .args(["hook", "precommit-check"])
            .current_dir(&dir)
            .env("HOOK_INPUT", payload)
            .output()
            .expect("spawn");

        assert!(out.status.success());
        let calls = fs::read_to_string(&log).unwrap_or_default();
        assert!(!calls.contains("rev-parse"), "calls: {calls}");
        assert!(
            calls.contains("diff --cached --name-only"),
            "calls: {calls}"
        );
    }

    /// Outside a repo the guard has no diff to read, and it must stay quiet
    /// rather than surfacing git's error.
    #[test]
    fn outside_a_git_repo_it_is_quiet() {
        let dir = scratch("no-repo");
        assert!(!warns_in(&dir, "git commit -m x"));
    }
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
            .env_remove("CI")
            .env_remove("PLAYBOOK_HEADLESS")
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
    fn long_foreground_commands_without_a_timeout_get_the_background_nudge() {
        for cmd in [
            "gh run watch 123 --exit-status",
            "gh pr checks 5 --watch",
            "cargo test --all",
            "cargo build --release",
        ] {
            assert!(warns(cmd, false), "foreground should nudge: {cmd}");
        }
    }

    #[test]
    fn the_nudge_names_background_and_timeout_and_stays_quiet_with_a_timeout() {
        let run = |input: serde_json::Value| {
            let out = Command::new(env!("CARGO_BIN_EXE_playbook"))
                .env_remove("CI")
                .env_remove("PLAYBOOK_HEADLESS")
                .args(["hook", "bg-await-guard"])
                .env("HOOK_INPUT", input.to_string())
                .output()
                .expect("playbook binary should spawn");
            String::from_utf8_lossy(&out.stdout).into_owned()
        };
        let nudged = run(serde_json::json!({"tool_input": {"command": "cargo test"}}));
        assert!(nudged.contains("run_in_background") && nudged.contains("600000"));
        let with_timeout =
            run(serde_json::json!({"tool_input": {"command": "cargo test", "timeout": 600000}}));
        assert!(
            with_timeout.trim().is_empty(),
            "explicit timeout: {with_timeout}"
        );
        let short = run(serde_json::json!({"tool_input": {"command": "cargo fmt --check"}}));
        assert!(short.trim().is_empty());
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
            .env_remove("CI")
            .env_remove("PLAYBOOK_HEADLESS")
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
                .env_remove("CI")
                .env_remove("PLAYBOOK_HEADLESS")
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

mod no_slop_guard {
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
    fn a_read_only_gh_api_call_is_not_a_posting() {
        assert!(!blocks(
            "gh api /repos/o/r/pulls/1/comments --jq '.[] | select(.body | contains(\"\u{2014}\"))'"
        ));
        assert!(blocks(
            "gh api -X POST /repos/o/r/issues/1/comments -f body=\"a \u{2014} b\""
        ));
        assert!(blocks(
            "gh api /repos/o/r/issues/1/comments -f body=\"a \u{2014} b\""
        ));
    }

    #[test]
    fn gh_api_flag_forms_decide_whether_it_posts() {
        let dash = "\u{2014}";
        assert!(!blocks(&format!(
            "gh api -X GET /repos/o/r/pulls/1/comments -f per_page=100 --jq '{dash}'"
        )));
        assert!(blocks(&format!(
            "gh api /repos/o/r/issues/1/comments -fbody='a {dash} b'"
        )));
        assert!(blocks(&format!(
            "gh api /repos/o/r/issues/1/comments -XPOST -f body='a {dash} b'"
        )));
    }

    #[test]
    fn a_missing_body_file_does_not_block() {
        // The guard catches prose it can read. A path it cannot open is the
        // shell's error to report, and blocking here would be a false deny.
        assert!(!blocks("gh pr create --body-file /nonexistent/nope.md"));
    }

    #[test]
    fn the_env_switch_disables_the_dash_check() {
        let payload =
            serde_json::json!({ "tool_input": { "command": "git commit -m \"a \u{2014} b\"" } })
                .to_string();
        let out = Command::new(env!("CARGO_BIN_EXE_playbook"))
            .env_remove("CI")
            .env_remove("PLAYBOOK_HEADLESS")
            .args(["hook", "no-slop-guard"])
            .env("HOOK_INPUT", payload)
            .env("NO_SLOP_GUARD", "0")
            .output()
            .expect("spawn");
        assert!(String::from_utf8_lossy(&out.stdout).trim().is_empty());
    }

    /// Stdin is the real hook contract; see also the comment-slop stdin test below.
    #[test]
    fn stdin_piped_input_is_read_when_hook_input_is_unset() {
        let deny = run_guard_via_stdin("no-slop-guard", "git commit -m \"fix: a \u{2014} b\"");
        assert!(
            deny.contains(r#""permissionDecision":"deny""#),
            "stdin-piped input must still reach the guard and produce a deny: {deny}"
        );

        let allow = run_guard_via_stdin("no-slop-guard", "git commit -m \"fix: a clean message\"");
        assert!(
            allow.trim().is_empty(),
            "stdin-piped input for a clean commit must stay silent: {allow}"
        );
    }

    #[test]
    fn malformed_and_empty_payloads_exit_silently() {
        // Anything unparseable must pass rather than fail on the hot path.
        for raw in [
            "",
            "{",
            "null",
            r#"{"tool_input":{}}"#,
            r#"{"tool_input":null}"#,
        ] {
            let out = Command::new(env!("CARGO_BIN_EXE_playbook"))
                .env_remove("CI")
                .env_remove("PLAYBOOK_HEADLESS")
                .args(["hook", "no-slop-guard"])
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

    // --- comment-slop side: PreToolUse(Edit|Write) --------------------------

    fn run_edit(file_path: &str, new_string: &str) -> String {
        let payload = serde_json::json!({
            "tool_input": { "file_path": file_path, "new_string": new_string }
        })
        .to_string();
        let out = Command::new(env!("CARGO_BIN_EXE_playbook"))
            .env_remove("CI")
            .env_remove("PLAYBOOK_HEADLESS")
            .args(["hook", "no-slop-guard"])
            .env("HOOK_INPUT", payload)
            .output()
            .expect("playbook binary should spawn");
        assert!(
            out.status.success(),
            "a guard must exit 0 even when denying"
        );
        String::from_utf8_lossy(&out.stdout).to_string()
    }

    fn run_write(file_path: &str, content: &str) -> String {
        let payload = serde_json::json!({
            "tool_input": { "file_path": file_path, "content": content }
        })
        .to_string();
        let out = Command::new(env!("CARGO_BIN_EXE_playbook"))
            .env_remove("CI")
            .env_remove("PLAYBOOK_HEADLESS")
            .args(["hook", "no-slop-guard"])
            .env("HOOK_INPUT", payload)
            .output()
            .expect("playbook binary should spawn");
        assert!(
            out.status.success(),
            "a guard must exit 0 even when denying"
        );
        String::from_utf8_lossy(&out.stdout).to_string()
    }

    fn is_denied(out: &str) -> bool {
        if out.trim().is_empty() {
            return false;
        }
        assert!(
            out.contains(r#""permissionDecision":"deny""#),
            "non-empty output must be a deny decision: {out}"
        );
        true
    }

    #[test]
    fn an_unwrapped_wide_comment_line_in_an_edit_blocks() {
        let new_string = format!("//! {}\nfn f() {{}}\n", "x".repeat(150));
        assert!(is_denied(&run_edit("src/lib.rs", &new_string)));
    }

    #[test]
    fn a_long_run_of_short_wrapped_lines_in_an_edit_is_allowed() {
        let new_string = "// line one\n// line two\n// line three\n// line four\nfn f() {}\n";
        assert!(!is_denied(&run_edit("src/lib.rs", new_string)));
    }

    #[test]
    fn orchestration_vocabulary_in_a_comment_blocks() {
        for new_string in [
            "// this Work Unit's file plan covers only this module\n",
            "// ported alongside WU-13's other guard bodies\n",
            "// see the brief for the full rationale\n",
            "// not done-when the tests pass, done when reviewed\n",
        ] {
            assert!(
                is_denied(&run_edit("src/hooks/example.rs", new_string)),
                "should block: {new_string}"
            );
        }
    }

    #[test]
    fn a_new_file_written_with_an_unwrapped_wide_module_doc_comment_blocks() {
        let content = format!(
            "// SPDX-FileCopyrightText: 2026 Igor Santos\n// SPDX-License-Identifier: Apache-2.0\n\n//! {}\n\nfn f() {{}}\n",
            "x".repeat(150)
        );
        assert!(is_denied(&run_write("src/hooks/example.rs", &content)));
    }

    #[test]
    fn a_normal_two_line_spdx_header_write_is_allowed() {
        let content =
            "// SPDX-FileCopyrightText: 2026 Igor Santos\n// SPDX-License-Identifier: Apache-2.0\n\nfn f() {}\n";
        assert!(!is_denied(&run_write("src/hooks/example.rs", content)));
    }

    #[test]
    fn shell_and_python_files_use_hash_comments() {
        let wide = format!("# {}\n", "x".repeat(150));
        assert!(is_denied(&run_edit("shell/example.sh", &wide)));
        assert!(is_denied(&run_edit("shell/example.py", &wide)));
    }

    #[test]
    fn extensions_without_a_known_comment_marker_are_never_blocked() {
        assert!(!is_denied(&run_edit(
            "README.md",
            "line one\nline two\nline three\nline four\n"
        )));
        assert!(!is_denied(&run_edit(
            "settings.shared.json",
            "// line one\n// line two\n// line three\n"
        )));
    }

    #[test]
    fn the_env_switch_disables_the_comment_check() {
        let payload = serde_json::json!({
            "tool_input": {
                "file_path": "src/lib.rs",
                "new_string": format!("//! {}\n", "x".repeat(150))
            }
        })
        .to_string();
        let out = Command::new(env!("CARGO_BIN_EXE_playbook"))
            .env_remove("CI")
            .env_remove("PLAYBOOK_HEADLESS")
            .args(["hook", "no-slop-guard"])
            .env("HOOK_INPUT", payload)
            .env("NO_SLOP_GUARD", "0")
            .output()
            .expect("spawn");
        assert!(String::from_utf8_lossy(&out.stdout).trim().is_empty());
    }

    #[test]
    fn edit_payload_malformed_and_empty_exit_silently() {
        for raw in [
            "",
            "{",
            "null",
            r#"{"tool_input":{}}"#,
            r#"{"tool_input":{"file_path":"src/lib.rs"}}"#,
        ] {
            let out = Command::new(env!("CARGO_BIN_EXE_playbook"))
                .env_remove("CI")
                .env_remove("PLAYBOOK_HEADLESS")
                .args(["hook", "no-slop-guard"])
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

    #[test]
    fn edit_payload_stdin_piped_input_is_read_when_hook_input_is_unset() {
        let payload = serde_json::json!({
            "tool_input": {
                "file_path": "src/lib.rs",
                "new_string": format!("//! {}\n", "x".repeat(150))
            }
        })
        .to_string();
        let mut child = Command::new(env!("CARGO_BIN_EXE_playbook"))
            .env_remove("CI")
            .env_remove("PLAYBOOK_HEADLESS")
            .args(["hook", "no-slop-guard"])
            .env_remove("HOOK_INPUT")
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .spawn()
            .expect("playbook binary should spawn");
        child
            .stdin
            .take()
            .expect("stdin should be piped")
            .write_all(payload.as_bytes())
            .expect("payload should write to stdin");
        let out = child
            .wait_with_output()
            .expect("playbook binary should exit");
        assert!(out.status.success());
        assert!(is_denied(&String::from_utf8_lossy(&out.stdout)));
    }

    #[test]
    fn a_write_over_an_existing_file_only_checks_what_changed() {
        let dir = scratch("legacy-write");
        let file = dir.join("legacy.rs");
        let old_line = format!("// {}", "legacy words ".repeat(12));
        let old = format!("{old_line}\nfn a() {{}}\n");
        fs::write(&file, &old).expect("write");
        let new = format!("{old_line}\nfn a() {{}}\n\nfn b() {{}}\n");
        assert!(!is_denied(&run_write(file.to_str().expect("utf8"), &new)));
        let wide = format!("{old}// {}\n", "new words ".repeat(15));
        assert!(is_denied(&run_write(file.to_str().expect("utf8"), &wide)));
        let fresh = dir.join("fresh.rs");
        assert!(is_denied(&run_write(fresh.to_str().expect("utf8"), &old)));
    }

    #[test]
    fn a_comment_line_that_is_one_long_url_is_allowed() {
        let url = format!("// https://example.com/{}\n", "a".repeat(120));
        assert!(!is_denied(&run_edit("src/lib.rs", &url)));
        let prose = format!(
            "// see https://example.com/{} for details\n",
            "a".repeat(120)
        );
        assert!(is_denied(&run_edit("src/lib.rs", &prose)));
    }
}
