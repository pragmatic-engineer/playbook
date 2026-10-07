// SPDX-FileCopyrightText: 2026 Igor Santos
// SPDX-License-Identifier: MIT

//! `commit-message-sanitizer` as seen from the real binary. Each command shape
//! is run through the hook, and the command it hands back is executed in a
//! scratch repository, so the assertions are on the message git actually
//! stores. Both directions are asserted: a hook that only proves it cleans
//! can pass while rewriting every commit.

#[path = "support/auto_env.rs"]
mod auto_env;

use auto_env::{hook_command, scratch, Scratch};
use serde_json::{json, Value};
use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

const HOOK: &str = "commit-message-sanitizer";
const COAUTHOR: &str = "Co-Authored-By: Claude <noreply@anthropic.com>";
const SESSION: &str = "Claude-Session: https://claude.ai/code/session_01AbCdEfGhIjKlMnOpQr";

struct Lab {
    scratch: Scratch,
    repo: PathBuf,
    bin: PathBuf,
}

impl Lab {
    fn new(tag: &str) -> Lab {
        let scratch = scratch(tag);
        let repo = scratch.cwd.join("repo");
        let bin = scratch.cwd.join("bin");
        fs::create_dir_all(&repo).expect("repo dir");
        fs::create_dir_all(&bin).expect("bin dir");
        let lab = Lab { scratch, repo, bin };
        lab.write_stub("rtk", "#!/bin/sh\nexec \"$@\"\n");
        lab.git(&["init", "-q", "-b", "main"]);
        lab.git(&["config", "user.name", "Test"]);
        lab.git(&["config", "user.email", "test@example.com"]);
        lab.git(&["config", "commit.gpgsign", "false"]);
        lab.git(&["config", "core.hooksPath", "/dev/null"]);
        lab.git(&["commit", "-q", "--allow-empty", "-m", "base"]);
        lab
    }

    fn write_stub(&self, name: &str, body: &str) {
        let path = self.bin.join(name);
        fs::write(&path, body).expect("stub");
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            fs::set_permissions(&path, fs::Permissions::from_mode(0o755)).expect("chmod");
        }
    }

    fn base(&self, program: &str) -> Command {
        let mut command = Command::new(program);
        let path = format!(
            "{}:{}",
            self.bin.display(),
            std::env::var("PATH").unwrap_or_default()
        );
        command
            .current_dir(&self.repo)
            .env("HOME", &self.scratch.home)
            .env("PATH", path)
            .env("GIT_CONFIG_NOSYSTEM", "1")
            .env_remove("GIT_DIR")
            .env_remove("GIT_EDITOR")
            .env("GIT_EDITOR", "true");
        command
    }

    fn git(&self, args: &[&str]) -> String {
        let out = self.base("git").args(args).output().expect("git runs");
        assert!(
            out.status.success(),
            "git {args:?}: {}",
            String::from_utf8_lossy(&out.stderr)
        );
        String::from_utf8_lossy(&out.stdout).into_owned()
    }

    fn head(&self) -> String {
        self.git(&["log", "-1", "--format=%B"])
    }

    fn payload(&self, command: &str) -> String {
        json!({
            "hook_event_name": "PreToolUse",
            "tool_name": "Bash",
            "cwd": self.repo,
            "tool_input": { "command": command, "description": "d", "timeout": 5000 },
        })
        .to_string()
    }

    fn hook(&self, command: &str) -> String {
        let mut child = hook_command(&self.scratch, HOOK)
            .current_dir(&self.repo)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()
            .expect("playbook spawns");
        child
            .stdin
            .take()
            .expect("stdin is piped")
            .write_all(self.payload(command).as_bytes())
            .expect("stdin accepts the payload");
        let out = child.wait_with_output().expect("playbook exits");
        assert_eq!(out.status.code(), Some(0), "{command}");
        String::from_utf8_lossy(&out.stdout).into_owned()
    }

    /// The command to run: the rewrite when the hook returns one, else the
    /// original. Asserts the output is a well-formed rewrite when present.
    fn rewritten(&self, command: &str) -> String {
        let out = self.hook(command);
        if out.is_empty() {
            return command.to_string();
        }
        updated_command(&out).unwrap_or_else(|| command.to_string())
    }

    fn run(&self, command: &str) -> (bool, String) {
        let out = self
            .base("bash")
            .args(["-c", command])
            .output()
            .expect("bash runs");
        let text = format!(
            "{}{}",
            String::from_utf8_lossy(&out.stdout),
            String::from_utf8_lossy(&out.stderr)
        );
        (out.status.success(), text)
    }

    /// Hook, then run what it returns. The run must succeed.
    fn commit_through_hook(&self, command: &str) -> String {
        let command = self.rewritten(command);
        let (ok, text) = self.run(&command);
        assert!(ok, "{command}\n{text}");
        self.head()
    }
}

fn updated_command(out: &str) -> Option<String> {
    let value: Value =
        serde_json::from_str(out.trim()).unwrap_or_else(|e| panic!("not JSON ({e}): {out:?}"));
    let inner = &value["hookSpecificOutput"];
    assert_eq!(inner["hookEventName"], "PreToolUse", "{out}");
    assert!(
        inner.get("permissionDecision").is_none(),
        "a rewrite must not decide permission: {out}"
    );
    let input = inner.get("updatedInput")?;
    assert_eq!(
        input["description"], "d",
        "other tool input fields survive: {out}"
    );
    assert_eq!(
        input["timeout"], 5000,
        "other tool input fields survive: {out}"
    );
    input["command"].as_str().map(str::to_string)
}

fn assert_no_attribution(message: &str) {
    for needle in ["Claude", "claude.ai", "anthropic", "Generated with"] {
        assert!(
            !message.contains(needle),
            "{needle} survived in: {message:?}"
        );
    }
}

fn write(dir: &Path, name: &str, text: &str) -> PathBuf {
    let path = dir.join(name);
    fs::write(&path, text).expect("file");
    path
}

#[test]
fn every_commit_message_form_is_stored_clean() {
    let dirty = format!("feat: x\n\nbody line\n\nRefs: 1\n{COAUTHOR}\n{SESSION}");
    let cases: Vec<(&str, String)> = vec![
        ("-m", format!("git commit --allow-empty -m {}", sh(&dirty))),
        ("joined -m", format!("git commit --allow-empty -m{}", sh(&dirty))),
        ("--message=", format!("git commit --allow-empty --message={}", sh(&dirty))),
        ("--m abbreviation", format!("git commit --allow-empty --m {}", sh(&dirty))),
        ("--mes= abbreviation", format!("git commit --allow-empty --mes={}", sh(&dirty))),
        (
            "several -m",
            format!(
                "git commit --allow-empty -m 'feat: x' -m 'body line' -m 'Refs: 1\n{COAUTHOR}\n{SESSION}'"
            ),
        ),
        (
            "heredoc to -F -",
            format!("git commit --allow-empty -F - <<'EOF'\n{dirty}\nEOF"),
        ),
        (
            "heredoc glued to -F -",
            format!("git commit --allow-empty -F -<<'EOF'\n{dirty}\nEOF"),
        ),
        (
            "command substitution heredoc with quotes and parentheses",
            format!(
                "git commit --allow-empty -m \"$(cat <<'EOF'\nfeat: x\n\nbody line say \"two words\" (twice)\n\nRefs: 1\n{COAUTHOR}\nEOF\n)\""
            ),
        ),
        (
            "heredoc to --file=-",
            format!("git commit --allow-empty --file=- <<'EOF'\n{dirty}\nEOF"),
        ),
        (
            "heredoc to -F /dev/stdin",
            format!("git commit --allow-empty -F /dev/stdin <<'EOF'\n{dirty}\nEOF"),
        ),
        (
            "heredoc to -F /dev/fd/0",
            format!("git commit --allow-empty -F /dev/fd/0 <<'EOF'\n{dirty}\nEOF"),
        ),
        (
            "unquoted heredoc",
            format!("git commit --allow-empty -F - <<EOF\n{dirty}\nEOF"),
        ),
        (
            "command substitution heredoc",
            format!("git commit --allow-empty -m \"$(cat <<'EOF'\n{dirty}\nEOF\n)\""),
        ),
        (
            "ANSI-C quoting",
            format!("git commit --allow-empty -m $'feat: x\\n\\nbody line\\n\\nRefs: 1\\n{COAUTHOR}\\n{SESSION}'"),
        ),
        (
            "heredoc producer",
            format!("cat <<'EOF' | git commit --allow-empty -F -\n{dirty}\nEOF"),
        ),
        (
            "bash -c",
            format!("bash -c {}", sh(&format!("git commit --allow-empty -m {}", sh(&dirty)))),
        ),
        (
            "sh -c",
            format!("sh -c {}", sh(&format!("git commit --allow-empty -m {}", sh(&dirty)))),
        ),
        (
            "env prefix",
            format!("env FOO=1 git commit --allow-empty -m {}", sh(&dirty)),
        ),
        (
            "rtk prefix",
            format!("rtk git commit --allow-empty -m {}", sh(&dirty)),
        ),
        (
            "nohup",
            format!("nohup git commit --allow-empty -m {}", sh(&dirty)),
        ),
        (
            "xargs",
            format!("echo go | xargs -I{{}} git commit --allow-empty -m {}", sh(&dirty)),
        ),
        (
            "find -exec",
            format!("find . -maxdepth 0 -exec git commit --allow-empty -m {} \\;", sh(&dirty)),
        ),
        (
            "quoted program name",
            format!("g''it commit --allow-empty -m {}", sh(&dirty)),
        ),
        (
            "shell fed a heredoc script",
            format!(
                "bash <<'SCRIPT'\ngit commit --allow-empty -m {}\nSCRIPT",
                sh(&dirty)
            ),
        ),
        (
            "chained after cd",
            format!("cd . && git commit --allow-empty -m {} && true", sh(&dirty)),
        ),
    ];
    for (name, command) in cases {
        let lab = Lab::new("commit-forms");
        let head = lab.commit_through_hook(&command);
        assert_no_attribution(&head);
        assert!(head.starts_with("feat: x"), "{name}: header lost: {head:?}");
        assert!(
            head.contains("Refs: 1"),
            "{name}: human trailer lost: {head:?}"
        );
        assert!(head.contains("body line"), "{name}: body lost: {head:?}");
    }
}

#[test]
fn proc_self_fd_zero_is_read_as_standard_input() {
    // Not run: /proc does not exist on every platform the tests run on.
    let lab = Lab::new("proc-fd");
    let command =
        format!("git commit --allow-empty -F /proc/self/fd/0 <<'EOF'\nfeat: x\n\n{COAUTHOR}\nEOF");

    let rewritten = lab.rewritten(&command);

    assert_ne!(rewritten, command);
    assert_no_attribution(&rewritten);
}

#[test]
fn git_dash_c_directory_is_followed() {
    let lab = Lab::new("dash-c");
    let command = format!(
        "cd / && git -C {} commit --allow-empty -m {}",
        lab.repo.display(),
        sh(&format!("feat: x\n\n{COAUTHOR}"))
    );

    let head = lab.commit_through_hook(&command);

    assert_eq!(head.trim(), "feat: x");
}

#[test]
fn a_message_file_is_left_alone_and_the_command_reads_the_cleaned_text_instead() {
    let lab = Lab::new("file");
    let text = format!("feat: x\n\nRefs: 1\n{COAUTHOR}\n");
    let file = write(&lab.repo, "msg.txt", &text);
    let command = format!("git commit --allow-empty -F {}", file.display());

    let out = lab.hook(&command);

    assert_eq!(
        updated_command(&out).as_deref(),
        Some(
            "git commit --allow-empty -F - <<'PLAYBOOK_MESSAGE_END'\n\
             feat: x\n\nRefs: 1\nPLAYBOOK_MESSAGE_END\n"
        )
    );
    assert_eq!(
        fs::read_to_string(&file).unwrap(),
        text,
        "the file is untouched"
    );
    assert!(out.contains("additionalContext"), "{out}");
    assert!(!out.contains("noreply@anthropic.com"), "{out}");
    let head = lab.commit_through_hook(&command);
    assert_eq!(head.trim(), "feat: x\n\nRefs: 1");
}

#[test]
fn the_heredoc_delimiter_never_collides_with_a_line_of_the_message() {
    let lab = Lab::new("delimiter");
    write(
        &lab.repo,
        "msg.txt",
        &format!("feat: x\n\nPLAYBOOK_MESSAGE_END\nPLAYBOOK_MESSAGE_END_\n{COAUTHOR}\n"),
    );

    let head = lab.commit_through_hook("git commit --allow-empty -F msg.txt");

    assert_eq!(
        head.trim(),
        "feat: x\n\nPLAYBOOK_MESSAGE_END\nPLAYBOOK_MESSAGE_END_"
    );
}

#[test]
fn a_message_file_in_every_option_spelling_is_fed_through_the_heredoc() {
    let spellings = [
        "-F msg.txt",
        "-Fmsg.txt",
        "--file msg.txt",
        "--file=msg.txt",
        "--fil=msg.txt",
        "-qF msg.txt",
    ];
    for spelling in spellings {
        let lab = Lab::new("file-spellings");
        let text = format!("feat: x\n\n{COAUTHOR}\n");
        let file = write(&lab.repo, "msg.txt", &text);

        let head = lab.commit_through_hook(&format!("git commit --allow-empty {spelling}"));

        assert_eq!(head.trim(), "feat: x", "{spelling}");
        assert_eq!(fs::read_to_string(file).unwrap(), text, "{spelling}");
    }
}

#[test]
fn a_message_file_that_cannot_be_fed_safely_is_reported_and_left_to_the_backstop() {
    let lab = Lab::new("file-skipped");
    let dirty = format!("feat: x\n\n{COAUTHOR}\n");
    let real = write(&lab.repo, "real.txt", &dirty);
    #[cfg(unix)]
    std::os::unix::fs::symlink(&real, lab.repo.join("link.txt")).unwrap();
    fs::write(lab.repo.join("binary.txt"), b"feat: x\n\n\xff\xfe\n").unwrap();
    fs::write(
        lab.repo.join("big.txt"),
        format!("{dirty}{}", "x".repeat(1 << 20)),
    )
    .unwrap();
    let cases = [
        ("link.txt", "a symbolic link"),
        ("binary.txt", "not valid UTF-8"),
        ("big.txt", "under 1 MiB"),
        ("missing.txt", "could not read"),
    ];
    for (name, reason) in cases {
        let command = format!("git commit --allow-empty -F {name}");

        let out = lab.hook(&command);

        assert_eq!(updated_command(&out), None, "{name}: {out}");
        assert!(out.contains(reason), "{name}: {out}");
        assert!(!out.contains("noreply@anthropic.com"), "{name}: {out}");
    }
    assert_eq!(fs::read_to_string(&real).unwrap(), dirty);
}

#[test]
fn a_message_file_written_earlier_in_the_same_command_is_not_trusted() {
    let lab = Lab::new("file-written");
    write(&lab.repo, "msg.txt", &format!("old\n\n{COAUTHOR}\n"));
    let command = "printf 'feat: new\\n' > msg.txt && git commit --allow-empty -F msg.txt";

    let out = lab.hook(command);

    assert_eq!(updated_command(&out), None, "{out}");
    assert!(out.contains("changed earlier in the same command"), "{out}");
}

#[test]
fn a_message_file_is_found_through_a_variable_and_a_relative_path() {
    let lab = Lab::new("file-var");
    write(&lab.repo, "m.txt", &format!("feat: x\n\n{SESSION}\n"));

    let head = lab.commit_through_hook("git commit --allow-empty -F ./m.txt");

    assert_eq!(head.trim(), "feat: x");
}

#[test]
fn an_unreadable_message_file_is_reported_without_blocking() {
    let lab = Lab::new("unreadable");
    let command = "git commit --allow-empty -F \"$UNSET_PLAYBOOK_DIR/m.txt\"";

    let out = lab.hook(command);

    let value: Value = serde_json::from_str(out.trim()).expect("JSON");
    let inner = &value["hookSpecificOutput"];
    assert!(inner.get("permissionDecision").is_none(), "{out}");
    assert!(
        inner["additionalContext"]
            .as_str()
            .unwrap()
            .contains("could not read"),
        "{out}"
    );
}

#[test]
fn tags_and_merges_are_cleaned() {
    let lab = Lab::new("tag-merge");
    let dirty = format!("feat: x\n\n{COAUTHOR}");

    let tag = lab.rewritten(&format!("git tag -a v1 -m {}", sh(&dirty)));
    assert!(lab.run(&tag).0);
    assert_eq!(
        lab.git(&["tag", "-l", "--format=%(contents)", "v1"]).trim(),
        "feat: x"
    );

    lab.git(&["checkout", "-q", "-b", "side"]);
    lab.git(&["commit", "-q", "--allow-empty", "-m", "side"]);
    lab.git(&["checkout", "-q", "main"]);
    lab.git(&["commit", "-q", "--allow-empty", "-m", "main work"]);
    let merge = lab.rewritten(&format!("git merge --no-ff side -m {}", sh(&dirty)));
    let (ok, text) = lab.run(&merge);
    assert!(ok, "{text}");
    assert_eq!(lab.head().trim(), "feat: x");
}

#[test]
fn a_cluster_of_short_options_still_finds_the_message() {
    let lab = Lab::new("cluster");
    write(&lab.repo, "f.txt", "one\n");
    lab.git(&["add", "f.txt"]);
    lab.git(&["commit", "-q", "-m", "add f"]);
    write(&lab.repo, "f.txt", "two\n");

    let head = lab.commit_through_hook(&format!(
        "git commit -am {}",
        sh(&format!("feat: x\n\n{COAUTHOR}"))
    ));

    assert_eq!(head.trim(), "feat: x");
}

#[test]
fn a_script_nested_in_a_word_is_rewritten() {
    let lab = Lab::new("nested");
    let inner = format!(
        "git commit --allow-empty -m {}",
        sh(&format!("feat: x\n\n{COAUTHOR}"))
    );
    let commands = [
        format!("git rebase -x {} HEAD", sh(&inner)),
        format!("env -S {}", sh(&inner)),
        format!("eval {}", sh(&inner)),
        format!("bash -c {}", sh(&format!("bash -c {}", sh(&inner)))),
    ];
    for command in commands {
        let rewritten = lab.rewritten(&command);
        assert_ne!(rewritten, command, "{command}");
        assert_no_attribution(&rewritten);
    }
}

#[test]
fn a_message_file_under_the_home_directory_is_found() {
    let lab = Lab::new("home-file");
    let file = write(
        &lab.scratch.home,
        "m.txt",
        &format!("feat: x\n\n{SESSION}\n"),
    );

    let head = lab.commit_through_hook("git commit --allow-empty -F ~/m.txt");

    assert_eq!(head.trim(), "feat: x");
    assert_eq!(
        fs::read_to_string(file).unwrap(),
        format!("feat: x\n\n{SESSION}\n")
    );
}

#[test]
fn a_file_message_keeps_its_comment_lines_and_is_not_cut_at_a_scissors_line() {
    let lab = Lab::new("verbatim");
    let scissors = "# ------------------------ >8 ------------------------";
    let text = format!(
        "feat: x\n\n# kept note\n{COAUTHOR}\nRefs: 1\n{scissors}\nbelow the scissors\n{SESSION}\n"
    );
    let file = write(&lab.repo, "m.txt", &text);

    let head = lab.commit_through_hook("git commit --allow-empty -F m.txt");

    // git does not cut a message read with -F, so the line below the scissors
    // line is part of the commit and is sanitised like any other.
    assert_eq!(
        head,
        format!("feat: x\n\n# kept note\nRefs: 1\n{scissors}\nbelow the scissors\n\n")
    );
    assert_eq!(fs::read_to_string(file).unwrap(), text);
}

#[test]
fn a_rewrite_has_no_permission_decision_and_keeps_the_other_input_fields() {
    let lab = Lab::new("shape");

    let out = lab.hook(&format!(
        "git commit --allow-empty -m {}",
        sh(&format!("feat: x\n\n{COAUTHOR}"))
    ));

    let value: Value = serde_json::from_str(out.trim()).expect("JSON");
    let inner = &value["hookSpecificOutput"];
    assert_eq!(inner["hookEventName"], "PreToolUse");
    assert!(inner.get("permissionDecision").is_none());
    assert!(inner.get("permissionDecisionReason").is_none());
    assert_eq!(inner["updatedInput"]["description"], "d");
    assert_eq!(inner["updatedInput"]["timeout"], 5000);
    assert!(
        inner["additionalContext"]
            .as_str()
            .unwrap()
            .contains("credit trailer"),
        "{out}"
    );
}

#[test]
fn unrelated_and_clean_commands_print_nothing() {
    let lab = Lab::new("silent");
    let commands = [
        "ls -la",
        "echo hello",
        "git status",
        "git log --oneline -3",
        "git diff HEAD",
        "cargo test",
        "git commit --allow-empty -m 'feat: clean'",
        "git commit --allow-empty -m 'feat: x' -m 'Refs: 1' -m 'Signed-off-by: A <a@b.c>'",
        "git commit --no-verify -m 'feat: x'",
        "git config core.hooksPath /tmp/hooks",
        "git config core.hooksPath",
        "GIT_DIR=/tmp/x git commit -m 'feat: x'",
        "git commit --allow-empty -F - <<'EOF'\nfeat: x\n\nCo-authored-by: Sam Lee <sam@example.com>\nEOF",
        "git commit --allow-empty -m 'document the Claude Code plugin'",
        "echo 'git commit -m x'",
        "",
    ];
    for command in commands {
        assert_eq!(lab.hook(command), "", "{command:?}");
    }
}

#[test]
fn a_payload_without_a_command_prints_nothing() {
    let lab = Lab::new("no-command");
    let mut child = hook_command(&lab.scratch, HOOK)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .spawn()
        .expect("spawns");
    child
        .stdin
        .take()
        .unwrap()
        .write_all(b"{\"tool_name\":\"Read\"}")
        .unwrap();

    let out = child.wait_with_output().unwrap();

    assert!(out.stdout.is_empty());
    assert_eq!(out.status.code(), Some(0));
}

#[test]
fn text_printed_by_other_programs_is_never_changed() {
    let lab = Lab::new("not-scripts");
    let inner = format!("git commit -m {}", sh(&format!("feat: x\n\n{COAUTHOR}")));
    let commands = [
        format!("echo {}", sh(&inner)),
        format!("printf '%s\\n' {}", sh(&inner)),
        format!("rg {}", sh(&inner)),
        format!("gh pr comment 1 --body {}", sh(&inner)),
        format!("grep -r {} .", sh(&inner)),
        format!("git log --grep {}", sh(&inner)),
        format!("git notes add -m {}", sh(&inner)),
    ];
    for command in commands {
        assert_eq!(lab.hook(&command), "", "{command}");
    }
}

#[test]
fn a_script_run_by_eval_and_a_rebase_exec_is_still_rewritten() {
    let lab = Lab::new("executed");
    let inner = format!(
        "git commit --allow-empty -m {}",
        sh(&format!("feat: x\n\n{COAUTHOR}"))
    );
    let commands = [
        format!("eval {}", sh(&inner)),
        format!("git rebase -x {} HEAD", sh(&inner)),
        format!("git rebase --exec {} HEAD", sh(&inner)),
        format!("git rebase --exec={} HEAD", sh(&inner)),
        format!("git -C . rebase -x{} HEAD", sh(&inner)),
    ];
    for command in commands {
        let rewritten = lab.rewritten(&command);
        assert_ne!(rewritten, command, "{command}");
        assert_no_attribution(&rewritten);
    }
}

#[test]
fn dropping_an_attribution_only_message_keeps_the_other_letters_of_its_cluster() {
    let lab = Lab::new("cluster-drop");
    let only = sh(COAUTHOR);
    let cases = [
        (
            format!("git commit --allow-empty -m {only}"),
            "git commit --allow-empty ",
        ),
        (
            format!("git commit --allow-empty -nm {only}"),
            "git commit --allow-empty -n",
        ),
        (
            format!("git commit --allow-empty -nm{only}"),
            "git commit --allow-empty -n",
        ),
        (
            format!("git commit --allow-empty -vnm {only} --no-verify"),
            "git commit --allow-empty -vn --no-verify",
        ),
    ];
    for (command, expected) in cases {
        assert_eq!(lab.rewritten(&command), expected, "{command}");
    }
}

#[test]
fn a_pipe_the_hook_cannot_see_into_is_reported_as_unread() {
    let lab = Lab::new("pipe");

    let out = lab.hook("generate-message | git commit --allow-empty -F -");
    let seen = lab.hook(&format!(
        "cat <<'EOF' | git commit --allow-empty -F -\nfeat: x\n{COAUTHOR}\nEOF"
    ));

    let note = |out: &str| {
        let value: Value = serde_json::from_str(out.trim()).expect("JSON");
        value["hookSpecificOutput"]["additionalContext"]
            .as_str()
            .map(str::to_string)
    };
    let unread = note(&out).expect("a note");
    assert!(unread.contains("read from a pipe"), "{unread}");
    assert!(!unread.contains("feat: x"), "{unread}");
    assert!(note(&seen).unwrap().contains("credit trailer"), "{seen}");
}

#[test]
fn a_cd_inside_parentheses_does_not_leak_to_later_commands() {
    let lab = Lab::new("subshell-cd");
    fs::create_dir_all(lab.repo.join("sub")).unwrap();
    write(&lab.repo, "m.txt", &format!("feat: x\n\n{COAUTHOR}\n"));
    write(
        &lab.repo.join("sub"),
        "m.txt",
        &format!("feat: sub\n\n{COAUTHOR}\n"),
    );

    let after = lab.rewritten("(cd sub && true) && git commit --allow-empty -F m.txt");
    let inside = lab.rewritten("(cd sub && git commit --allow-empty -F m.txt)");
    let following = lab.rewritten("cd sub && git commit --allow-empty -F m.txt");

    assert!(after.contains("feat: x\n"), "{after}");
    assert!(inside.contains("feat: sub\n"), "{inside}");
    assert!(following.contains("feat: sub\n"), "{following}");
}

/// The note the hook adds for the agent, or an empty string when there is none.
fn note(out: &str) -> String {
    let value: Value = serde_json::from_str(out.trim()).unwrap_or(Value::Null);
    value["hookSpecificOutput"]["additionalContext"]
        .as_str()
        .unwrap_or_default()
        .to_string()
}

/// The hook leaves the command as it is and says why it could not read the file.
fn assert_left_unread(lab: &Lab, command: &str, reason: &str) {
    let out = lab.hook(command);

    assert!(!out.is_empty(), "no note for: {command}");
    assert_eq!(updated_command(&out), None, "{command}: {out}");
    assert!(note(&out).contains(reason), "{command}: {out}");
    assert!(!out.contains("noreply@anthropic.com"), "{command}: {out}");
}

#[test]
fn a_message_file_feeds_through_the_end_of_its_logical_line() {
    let cases = [
        "git commit --allow-empty -F msg.txt && \\\ngit log -1 --format=%s",
        "git commit --allow-empty -F msg.txt && echo \"a\nb\"",
        "git commit --allow-empty -F msg.txt && echo \"$(cat <<'EOF'\nhello\nEOF\n)\"",
        "git commit --allow-empty -F msg.txt &&\ngit log -1 --format=%s",
        "git commit --allow-empty -F msg.txt # a note\ngit log -1 --format=%s",
    ];
    for command in cases {
        let lab = Lab::new("file-line-end");
        write(&lab.repo, "msg.txt", &format!("feat: x\n\n{COAUTHOR}\n"));

        let head = lab.commit_through_hook(command);

        assert_eq!(head.trim(), "feat: x", "{command}");
    }
}

#[test]
fn a_message_file_with_another_heredoc_open_on_its_line_is_left_unread() {
    let lab = Lab::new("file-busy-line");
    write(&lab.repo, "msg.txt", &format!("feat: x\n\n{COAUTHOR}\n"));

    assert_left_unread(
        &lab,
        "cat <<'EOF' | git commit --allow-empty -F msg.txt\nnote\nEOF",
        "another heredoc is open",
    );
}

#[test]
fn a_message_file_is_never_inlined_into_an_expanding_or_ambiguous_outer_heredoc() {
    let lab = Lab::new("file-in-heredoc");
    write(
        &lab.repo,
        "msg.txt",
        &format!("feat: x\n\nuses $(echo hi) and `id`\n{COAUTHOR}\n"),
    );
    write(
        &lab.repo,
        "eof.txt",
        &format!("feat: x\n\nEOF\n{COAUTHOR}\n"),
    );
    write(
        &lab.repo,
        "tab.txt",
        &format!("feat: x\n\n\tEOF\n{COAUTHOR}\n"),
    );
    let commit = |file: &str| format!("git commit --allow-empty -F {file}");

    assert_left_unread(
        &lab,
        &format!("bash <<EOF\n{}\nEOF", commit("msg.txt")),
        "outer heredoc",
    );
    assert_left_unread(
        &lab,
        &format!("bash <<'EOF'\n{}\nEOF", commit("eof.txt")),
        "outer heredoc",
    );
    assert_left_unread(
        &lab,
        &format!("bash <<-'EOF'\n\t{}\n\tEOF", commit("tab.txt")),
        "outer heredoc",
    );
    let head = lab.commit_through_hook(&format!("bash <<'EOF'\n{}\nEOF", commit("msg.txt")));
    assert_eq!(head.trim(), "feat: x\n\nuses $(echo hi) and `id`");
    let head = lab.commit_through_hook(&format!("bash <<'EOF'\n{}\nEOF", commit("tab.txt")));
    assert_eq!(head.trim(), "feat: x\n\n\tEOF");
}

#[test]
fn a_message_file_is_not_swapped_for_standard_input_when_stdin_is_spoken_for() {
    let lab = Lab::new("file-own-stdin");
    write(&lab.repo, "msg.txt", &format!("feat: x\n\n{COAUTHOR}\n"));
    let commands = [
        "git commit --allow-empty -F msg.txt < /dev/null",
        "git commit --allow-empty -F msg.txt <<< 'text'",
        "printf 'a\\n' | xargs git commit --allow-empty -F msg.txt",
        "printf 'a\\n' | xargs -n1 rtk git commit --allow-empty -F msg.txt",
        "parallel git commit --allow-empty -F msg.txt ::: a",
    ];
    for command in commands {
        assert_left_unread(&lab, command, "standard input");
    }
}

#[test]
fn a_message_read_from_a_redirect_or_here_string_is_reported_as_unread() {
    let lab = Lab::new("stdin-redirect");
    write(&lab.repo, "msg.txt", &format!("feat: x\n\n{COAUTHOR}\n"));
    let commands = [
        "git commit --allow-empty -F - < msg.txt",
        "git commit --allow-empty -F - <<< \"feat: x\"",
        "git commit --allow-empty -F /dev/stdin 0< msg.txt",
    ];
    for command in commands {
        assert_left_unread(&lab, command, "standard input");
    }
}

#[test]
fn sibling_subshells_do_not_share_a_directory() {
    let lab = Lab::new("sibling-subshells");
    fs::create_dir_all(lab.repo.join("sub")).unwrap();
    write(&lab.repo, "m.txt", &format!("feat: top\n\n{COAUTHOR}\n"));
    write(
        &lab.repo.join("sub"),
        "m.txt",
        &format!("feat: sub\n\n{COAUTHOR}\n"),
    );

    let sibling = lab.rewritten("(cd sub && true) && (git commit --allow-empty -F m.txt)");
    let nested = lab.rewritten("(cd sub && (true); git commit --allow-empty -F m.txt)");
    let after = lab.rewritten("(cd sub) ; (cd sub) ; (git commit --allow-empty -F m.txt)");

    assert!(sibling.contains("feat: top\n"), "{sibling}");
    assert!(nested.contains("feat: sub\n"), "{nested}");
    assert!(after.contains("feat: top\n"), "{after}");
}

#[test]
fn a_carrier_that_runs_a_shell_hands_the_script_to_the_nested_walk() {
    let lab = Lab::new("carrier-shell");
    let inner = format!(
        "git commit --allow-empty -m {}",
        sh(&format!("feat: x\n\n{COAUTHOR}"))
    );
    let commands = [
        format!("rtk bash -c {}", sh(&inner)),
        format!("nohup sh -c {}", sh(&inner)),
        format!("printf 'a\\n' | xargs -I{{}} sh -c {}", sh(&inner)),
        format!("find . -maxdepth 0 -exec bash -c {} \\;", sh(&inner)),
        format!("rtk eval {}", sh(&inner)),
    ];
    for command in commands {
        let rewritten = lab.rewritten(&command);

        assert_ne!(rewritten, command, "{command}");
        assert_no_attribution(&rewritten);
    }
}

/// `text` as one single-quoted shell word.
fn sh(text: &str) -> String {
    format!("'{}'", text.replace('\'', "'\\''"))
}
