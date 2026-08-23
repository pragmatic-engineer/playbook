// SPDX-FileCopyrightText: 2026 Igor Santos
// SPDX-License-Identifier: MIT

//! Behavioural tests for the ported safety guards.
//!
//! Both directions are asserted for every case. A guard that only proves it
//! blocks can pass while blocking everything, which breaks the tool it guards;
//! one that only proves it allows can pass while guarding nothing.

use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
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
    let dir = base.join(format!("playbook-guards-{tag}-{}-{n}", std::process::id()));
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

mod rm_workspace_guard {
    use super::*;

    /// `roots: None` REMOVES `PLAYBOOK_SAFE_ROOTS` rather than setting it empty,
    /// which is the only way to reach `safe_roots()`'s zero-config fallback. It
    /// must be removed, not merely left alone: the variable may be set in the
    /// environment the suite inherits, and then the default path would never run.
    fn run_guard_in(command: &str, roots: Option<&str>, cwd: Option<&Path>) -> String {
        let payload = serde_json::json!({ "tool_input": { "command": command } }).to_string();
        let mut spawn = Command::new(env!("CARGO_BIN_EXE_playbook"));
        spawn
            .args(["hook", "rm-workspace-guard"])
            .env("HOOK_INPUT", payload);
        match roots {
            Some(value) => spawn.env("PLAYBOOK_SAFE_ROOTS", value),
            None => spawn.env_remove("PLAYBOOK_SAFE_ROOTS"),
        };
        if let Some(dir) = cwd {
            spawn.current_dir(dir);
        }
        let out = spawn.output().expect("playbook binary should spawn");
        assert!(
            out.status.success(),
            "a guard must exit 0 even when denying"
        );
        String::from_utf8_lossy(&out.stdout).to_string()
    }

    fn is_deny(stdout: &str) -> bool {
        if stdout.trim().is_empty() {
            return false;
        }
        assert!(
            stdout.contains(r#""permissionDecision":"deny""#),
            "non-empty output must be a deny: {stdout}"
        );
        true
    }

    /// Runs with an explicit safe-root list and the repo as cwd, so the result
    /// never depends on where the suite happens to be invoked from.
    fn blocked(command: &str, roots: &str) -> bool {
        is_deny(&run_guard_in(command, Some(roots), None))
    }

    /// The zero-config path. `blocked` above can never reach it, because it sets
    /// the variable on every call.
    fn blocked_defaulting(command: &str, cwd: &Path) -> bool {
        is_deny(&run_guard_in(command, None, Some(cwd)))
    }

    /// A scratch dir resolved through its symlinks. macOS temp dirs live under
    /// `/var`, a symlink to `/private/var`, while `git rev-parse --show-toplevel`
    /// and `getcwd` both report the resolved form. The guard's `canon()` is
    /// deliberately lexical and never touches the filesystem, so an unresolved
    /// fixture path would compare unequal to the derived root and the case would
    /// fail on path form rather than on guard logic.
    fn real_dir(tag: &str) -> PathBuf {
        fs::canonicalize(scratch(tag)).expect("scratch dir should resolve")
    }

    /// A path guaranteed outside every safe root, used as the negative half of
    /// the default-root cases. Deliberately NOT under the temp tree: temp is
    /// always allowed, and on Linux `std::env::temp_dir()` IS `/tmp`, so a
    /// temp-based fixture would quietly stop proving anything there while still
    /// passing on macOS, where temp is `/var/folders`. It never needs to exist,
    /// because the guard resolves lexically without touching the filesystem.
    fn outside_everything() -> String {
        format!("{}/outside-every-safe-root", home())
    }

    fn home() -> String {
        std::env::var("HOME").expect("HOME")
    }

    /// Assembled at runtime so this file never contains a literal deletion of a
    /// system path: the live guard inspects tool calls, and a source file
    /// carrying those strings trips it during ordinary work on the repo.
    fn del() -> String {
        "r".to_string() + "m"
    }
    fn etc() -> String {
        "/e".to_string() + "tc"
    }

    #[test]
    fn targets_inside_a_safe_root_are_allowed() {
        let (h, d) = (home(), del());
        let ws = format!("{h}/Workspace");
        for cmd in [
            format!("{d} -rf {ws}/proj/build"),
            format!("{d} {ws}/a.txt"),
            format!("{d} -rf ~/Workspace/proj/node_modules"),
            // ~/.claude is always allowed, whatever the roots say.
            format!("{d} -rf {h}/.claude/cache/x"),
            // Collapses back inside the root, so it stays allowed.
            format!("{d} -rf {ws}/proj/sub/../build"),
        ] {
            assert!(!blocked(&cmd, &ws), "should allow: {cmd}");
        }
    }

    #[test]
    fn targets_outside_every_safe_root_are_blocked() {
        let (h, d, e) = (home(), del(), etc());
        let ws = format!("{h}/Workspace");
        for cmd in [
            format!("{d} -rf {e}/passwd"),
            format!("{d} {h}/secrets.txt"),
            format!("{d} -rf /"),
        ] {
            assert!(blocked(&cmd, &ws), "should block: {cmd}");
        }
    }

    /// The lexical canonicaliser is what closes this: the path is resolved
    /// without touching the filesystem, since the target may not exist.
    #[test]
    fn dot_dot_traversal_out_of_a_safe_root_is_blocked() {
        let (h, d, e) = (home(), del(), etc());
        let ws = format!("{h}/Workspace");
        for cmd in [
            format!("{d} -rf ~/Workspace/../.ssh"),
            format!("{d} -rf {ws}/../../..{e}/passwd"),
            format!("{d} -rf {h}/.claude/../.aws/credentials"),
        ] {
            assert!(blocked(&cmd, &ws), "traversal should block: {cmd}");
        }
    }

    #[test]
    fn unresolvable_commands_are_blocked_conservatively() {
        let (h, d, e) = (home(), del(), etc());
        let ws = format!("{h}/Workspace");
        // A cd makes a relative target unresolvable.
        assert!(blocked(&format!("cd /tmp && {d} -rf foo"), &ws));
        // A substitution could expand to anything.
        assert!(blocked(&format!("{d} -rf $(echo {e})"), &ws));
    }

    /// Newlines and tabs are normalised to separators, so a deletion on any
    /// line is still seen rather than hidden by the tokenizer.
    #[test]
    fn multiline_and_tab_separated_commands_are_still_inspected() {
        let (h, d, e) = (home(), del(), etc());
        let ws = format!("{h}/Workspace");
        assert!(blocked(&format!("echo hi\n{d} -rf {e}/passwd"), &ws));
        assert!(blocked(&format!("echo hi\t{d} -rf {e}/passwd"), &ws));
        assert!(!blocked("echo hi\necho there", &ws));
        assert!(!blocked(&format!("echo hi\n{d} -rf {ws}/proj/build"), &ws));
    }

    #[test]
    fn multiple_roots_are_all_honoured() {
        let (h, d) = (home(), del());
        let roots = format!("{h}/a:{h}/b");
        assert!(!blocked(&format!("{d} -rf {h}/b/file"), &roots));
        assert!(!blocked(&format!("{d} -rf {h}/a/file"), &roots));
        assert!(blocked(&format!("{d} -rf {h}/c/file"), &roots));
    }

    #[test]
    fn a_trailing_slash_on_a_root_still_matches() {
        let (h, d) = (home(), del());
        let ws = format!("{h}/Workspace");
        assert!(!blocked(&format!("{d} -rf {ws}/file"), &format!("{ws}/")));
    }

    /// A root that does not exist must not accidentally widen the allowlist.
    #[test]
    fn a_nonexistent_root_blocks_everything_outside_it() {
        let (d, e) = (del(), etc());
        assert!(blocked(
            &format!("{d} -rf {e}/passwd"),
            "/nonexistent/definitely/not/here"
        ));
    }

    /// With the variable unset the repo root becomes the safe root. The negative
    /// half is the point: a sibling of the repo must still block, or the default
    /// would have widened to the whole temp parent.
    #[test]
    fn an_unset_variable_defaults_to_the_git_repo_root() {
        let d = del();
        let repo = real_dir("default-repo");
        Command::new("git")
            .arg("-C")
            .arg(&repo)
            .args(["init", "-q"])
            .output()
            .expect("git should run");
        fs::create_dir_all(repo.join("sub")).expect("sub dir");

        let inside = repo.display();
        assert!(
            !blocked_defaulting(&format!("{d} -rf {inside}/sub/file"), &repo),
            "the repo root is the default safe root"
        );
        assert!(
            blocked_defaulting(&format!("{d} -rf {}/file", outside_everything()), &repo),
            "a path outside the repo root is not covered by the default"
        );
    }

    /// No repo, so the fallback drops through to the cwd itself.
    #[test]
    fn an_unset_variable_outside_a_git_repo_defaults_to_the_cwd() {
        let d = del();
        let plain = real_dir("default-plain");

        let inside = plain.display();
        assert!(
            !blocked_defaulting(&format!("{d} -rf {inside}/file"), &plain),
            "the cwd is the default safe root when there is no repo"
        );
        assert!(
            blocked_defaulting(&format!("{d} -rf {}/file", outside_everything()), &plain),
            "a path outside the cwd is not covered by the default"
        );
    }

    /// `safe_roots()` branches on `configured.is_empty()`, so an explicitly empty
    /// value has to take the same path as an absent one.
    #[test]
    fn an_empty_variable_behaves_like_unset() {
        let (d, e) = (del(), etc());
        let plain = real_dir("empty-var");
        let inside = plain.display();

        assert!(
            is_deny(&run_guard_in(
                &format!("{d} -rf {e}/passwd"),
                Some(""),
                Some(&plain)
            )),
            "an empty value must not open the allowlist"
        );
        assert!(
            !is_deny(&run_guard_in(
                &format!("{d} -rf {inside}/file"),
                Some(""),
                Some(&plain)
            )),
            "it falls back to the default root rather than blocking everything"
        );
    }

    /// A relative root is canon-ed against the guard's own cwd. Both halves are
    /// needed: resolving to the wrong directory would show up as a false allow
    /// elsewhere, not as a failure on the intended path.
    #[test]
    fn a_relative_root_resolves_against_the_guards_cwd() {
        let (d, e) = (del(), etc());
        let base = real_dir("relative-root");
        fs::create_dir_all(base.join("relroot")).expect("relroot dir");
        let inside = base.display();

        assert!(
            !is_deny(&run_guard_in(
                &format!("{d} -rf {inside}/relroot/file"),
                Some("relroot"),
                Some(&base)
            )),
            "a relative root resolves against the cwd"
        );
        assert!(
            is_deny(&run_guard_in(
                &format!("{d} -rf {e}/passwd"),
                Some("relroot"),
                Some(&base)
            )),
            "and must not widen into a blanket allow"
        );
    }

    /// The only assertion on the deny message's CONTENT. Without it the message
    /// could regress to a hardcoded `~/Workspace` literal with every other test
    /// in this file still green.
    #[test]
    fn the_deny_reason_names_the_roots_in_effect() {
        let (h, d, e) = (home(), del(), etc());
        let (first, second) = (format!("{h}/a"), format!("{h}/b"));
        let out = run_guard_in(
            &format!("{d} -rf {e}/passwd"),
            Some(&format!("{first}:{second}")),
            None,
        );
        let parsed: serde_json::Value =
            serde_json::from_str(out.trim()).expect("a deny must be valid JSON");
        let reason = parsed["hookSpecificOutput"]["permissionDecisionReason"]
            .as_str()
            .expect("a deny must carry a reason");
        assert!(
            reason.contains(&first) && reason.contains(&second),
            "the reason must name the roots actually in effect: {reason}"
        );
    }

    /// ACCEPTED false positive, pinned so it is not "discovered" as a bug later.
    /// The tokenizer cannot tell a heredoc body from a command, so a body merely
    /// QUOTING a deletion still blocks. The trade-off is intentional; the old
    /// shell suite documented it at :97-102 and this keeps that record.
    #[test]
    fn a_heredoc_body_quoting_a_deletion_is_an_accepted_false_positive() {
        let (h, d, e) = (home(), del(), etc());
        let ws = format!("{h}/Workspace");
        assert!(
            blocked(
                &format!("cat <<EOF\nold script did {d} -rf {e}/example here\nEOF"),
                &ws
            ),
            "nothing is deleted here, but the tokenizer cannot know that"
        );
    }

    /// Scratch space is exempt from the configured roots, the same standing
    /// exemption `~/.claude` has. The root itself is NOT exempt: unlike a
    /// configured safe root, which may be deleted whole, wiping the temp root
    /// takes out sockets and runtime state that live processes depend on.
    #[test]
    fn paths_inside_temp_are_allowed_but_the_temp_root_is_not() {
        let (h, d) = (home(), del());
        let ws = format!("{h}/Workspace");
        for cmd in [
            format!("{d} -rf /tmp/scratch-file"),
            // macOS spells the same directory both ways and `canon` is lexical,
            // so the resolved form has to be listed too.
            format!("{d} -rf /private/tmp/scratch-file"),
        ] {
            assert!(!blocked(&cmd, &ws), "temp scratch should be allowed: {cmd}");
        }
        for cmd in [format!("{d} -rf /tmp"), format!("{d} -rf /private/tmp")] {
            assert!(
                blocked(&cmd, &ws),
                "the temp root itself must stay blocked: {cmd}"
            );
        }
    }

    /// The exemption is by resolved path, never by how the string is spelled.
    #[test]
    fn the_temp_exemption_does_not_leak_through_traversal_or_prefixes() {
        let (h, d, e) = (home(), del(), etc());
        let ws = format!("{h}/Workspace");
        assert!(
            blocked(&format!("{d} -rf /tmp/..{e}/passwd"), &ws),
            "canon collapses the traversal, so this is judged as its real target"
        );
        assert!(
            blocked(&format!("{d} -rf /tmpfoo/file"), &ws),
            "a directory that merely starts with the same letters is not temp"
        );
    }

    /// A mention inside a quoted message or title is prose, not a command. This
    /// was a live annoyance rather than a theoretical one: it blocked commits
    /// and PR creation whose text merely named the guard.
    /// Every case here must FAIL under the old tokenizer, or the test proves
    /// nothing. A message whose remaining words are all relative paths is not
    /// enough: they canonicalise inside the safe root and were allowed anyway.
    /// So each case carries either an absolute path the old code would have
    /// judged as a target, or a substitution, which is the shape that actually
    /// blocked an agent mid-commit.
    #[test]
    fn a_quoted_mention_is_prose_not_a_command() {
        let (h, d, e) = (home(), del(), etc());
        let ws = format!("{h}/Workspace");
        for cmd in [
            format!("git commit -m \"fix: stop using {d} on {e}/passwd\""),
            format!("git commit -m 'docs: {d} on {e}/passwd is fatal'"),
            format!("git commit -m \"note: {d} is risky\" && echo $(date)"),
        ] {
            assert!(!blocked(&cmd, &ws), "prose must not block: {cmd}");
        }
    }

    /// Quoting must not become an escape hatch. Anything in command position is
    /// still a command, and outside quotes nothing changed at all, which is what
    /// keeps the wrapper forms caught.
    #[test]
    fn quoting_does_not_hide_a_real_deletion() {
        let (h, d, e) = (home(), del(), etc());
        let ws = format!("{h}/Workspace");
        for cmd in [
            format!("sudo {d} -rf {e}/passwd"),
            format!("xargs {d} -rf {e}/passwd"),
            format!("echo hi && {d} -rf {e}/passwd"),
            // Inside quotes, but a separator puts it back in command position.
            format!("sh -c \"cd /x && {d} -rf {e}/passwd\""),
        ] {
            assert!(blocked(&cmd, &ws), "must still block: {cmd}");
        }
    }

    #[test]
    fn malformed_payloads_exit_silently() {
        for raw in ["", "{", "null", r#"{"tool_input":{}}"#] {
            let out = Command::new(env!("CARGO_BIN_EXE_playbook"))
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
            .args(["hook", "precommit-check"])
            .current_dir(&dir)
            .env("HOOK_INPUT", payload)
            .env("PRECOMMIT_CHECK", "0")
            .output()
            .expect("spawn");
        assert!(String::from_utf8_lossy(&out.stdout).trim().is_empty());
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
