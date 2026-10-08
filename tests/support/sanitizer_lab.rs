// SPDX-FileCopyrightText: 2026 Igor Santos
// SPDX-License-Identifier: Apache-2.0

//! A scratch git repository, stub `gh` and `rtk` programs, and a way to run
//! the `commit-message-sanitizer` hook against them, shared by the tests of
//! its PreToolUse rewrite and its PostToolUse backstop.

#![allow(dead_code)]

use super::auto_env::{hook_command, scratch, Scratch};
use serde_json::{json, Value};
use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

pub const HOOK: &str = "commit-message-sanitizer";
pub const COAUTHOR: &str = "Co-Authored-By: Claude <noreply@anthropic.com>";
pub const SESSION: &str = "Claude-Session: https://claude.ai/code/session_01AbCdEfGhIjKlMnOpQr";
pub const FOOTER: &str = "Generated with [Claude Code](https://claude.com/claude-code)";

pub struct Lab {
    pub scratch: Scratch,
    pub repo: PathBuf,
    pub bin: PathBuf,
}

impl Lab {
    pub fn new(tag: &str) -> Lab {
        let scratch = scratch(tag);
        let repo = scratch.cwd.join("repo");
        let bin = scratch.cwd.join("bin");
        fs::create_dir_all(&repo).expect("repo dir");
        fs::create_dir_all(&bin).expect("bin dir");
        let lab = Lab { scratch, repo, bin };
        lab.write_stub("rtk", "#!/bin/sh\nexec \"$@\"\n");
        lab.write_stub(
            "gh",
            "#!/bin/sh\nfor a in \"$@\"; do printf '%s\\n' \"$a\"; done >> \"$(dirname \"$0\")/gh.args\"\nwhile [ $# -gt 0 ]; do case \"$1\" in --body-file) cat \"$2\" >> \"$(dirname \"$0\")/gh.body\";; esac; shift; done\n",
        );
        lab.git(&["init", "-q", "-b", "main"]);
        lab.git(&["config", "user.name", "Test"]);
        lab.git(&["config", "user.email", "test@example.com"]);
        lab.git(&["config", "commit.gpgsign", "false"]);
        lab.git(&["config", "core.hooksPath", "/dev/null"]);
        lab.git(&["commit", "-q", "--allow-empty", "-m", "base"]);
        lab
    }

    pub fn write_global_config(&self, json: &str) {
        let dir = self.scratch.home.join(".config").join("playbook");
        fs::create_dir_all(&dir).expect("config dir");
        fs::write(dir.join("config.json"), json).expect("config file");
    }

    pub fn write_stub(&self, name: &str, body: &str) {
        let path = self.bin.join(name);
        fs::write(&path, body).expect("stub");
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            fs::set_permissions(&path, fs::Permissions::from_mode(0o755)).expect("chmod");
        }
    }

    pub fn base(&self, program: &str) -> Command {
        let mut command = Command::new(program);
        command
            .current_dir(&self.repo)
            .env("HOME", &self.scratch.home)
            .env("PATH", self.path())
            .env("GIT_CONFIG_NOSYSTEM", "1")
            .env_remove("GIT_DIR")
            .env("GIT_EDITOR", "true");
        command
    }

    pub fn git(&self, args: &[&str]) -> String {
        let out = self.base("git").args(args).output().expect("git runs");
        assert!(
            out.status.success(),
            "git {args:?}: {}",
            String::from_utf8_lossy(&out.stderr)
        );
        String::from_utf8_lossy(&out.stdout).into_owned()
    }

    pub fn head(&self) -> String {
        self.git(&["log", "-1", "--format=%B"])
    }

    pub fn payload(&self, event: &str, command: &str, stdout: &str) -> String {
        json!({
            "hook_event_name": event,
            "tool_name": "Bash",
            "session_id": "lab-session",
            "cwd": self.repo,
            "tool_input": { "command": command, "description": "d", "timeout": 5000 },
            "tool_response": { "stdout": stdout, "stderr": "" },
        })
        .to_string()
    }

    fn run_hook(&self, payload: &str) -> String {
        let mut child = hook_command(&self.scratch, HOOK)
            .current_dir(&self.repo)
            .env("PATH", self.path())
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()
            .expect("playbook spawns");
        child
            .stdin
            .take()
            .expect("stdin is piped")
            .write_all(payload.as_bytes())
            .expect("stdin accepts the payload");
        let out = child.wait_with_output().expect("playbook exits");
        assert_eq!(out.status.code(), Some(0), "{payload}");
        String::from_utf8_lossy(&out.stdout).into_owned()
    }

    /// The PreToolUse output for `command`.
    pub fn hook(&self, command: &str) -> String {
        self.run_hook(&self.payload("PreToolUse", command, ""))
    }

    /// The PostToolUse output after `command` printed `stdout`.
    pub fn post(&self, command: &str, stdout: &str) -> String {
        self.run_hook(&self.payload("PostToolUse", command, stdout))
    }

    /// A call as Claude Code runs it: the PreToolUse hook sees `command`, `ran`
    /// does what the shell did, and the PostToolUse hook sees the call after.
    pub fn through_hooks(&self, command: &str, ran: impl FnOnce(&Lab)) -> String {
        self.hook(command);
        ran(self);
        self.post(command, "")
    }

    /// [`Lab::through_hooks`] with `command` itself as what ran, not the
    /// PreToolUse rewrite, so the backstop sees what the rewrite missed. The
    /// command may fail, as a real commit can.
    pub fn run_through_hooks(&self, command: &str) -> String {
        self.through_hooks(command, |lab| {
            lab.run(command);
        })
    }

    fn path(&self) -> String {
        format!(
            "{}:{}",
            self.bin.display(),
            std::env::var("PATH").unwrap_or_default()
        )
    }

    /// The command to run: the rewrite when the hook returns one, else the
    /// original. Asserts the output is a well-formed rewrite when present.
    pub fn rewritten(&self, command: &str) -> String {
        let out = self.hook(command);
        if out.is_empty() {
            return command.to_string();
        }
        updated_command(&out).unwrap_or_else(|| command.to_string())
    }

    pub fn run(&self, command: &str) -> (bool, String) {
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
    pub fn commit_through_hook(&self, command: &str) -> String {
        let command = self.rewritten(command);
        let (ok, text) = self.run(&command);
        assert!(ok, "{command}\n{text}");
        self.head_without_sign_off()
    }

    /// The stored message without the sign-off of this repository's own
    /// identity, which the hook adds to a commit that has none.
    pub fn head_without_sign_off(&self) -> String {
        let head = self.head();
        let kept: Vec<&str> = head
            .lines()
            .filter(|line| *line != "Signed-off-by: Test <test@example.com>")
            .collect();
        format!("{}\n\n", kept.join("\n").trim_end())
    }
}

pub fn updated_command(out: &str) -> Option<String> {
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

pub fn assert_no_attribution(message: &str) {
    for needle in ["Claude", "claude.ai", "anthropic", "Generated with"] {
        assert!(
            !message.contains(needle),
            "{needle} survived in: {message:?}"
        );
    }
}

pub fn write(dir: &Path, name: &str, text: &str) -> PathBuf {
    let path = dir.join(name);
    fs::write(&path, text).expect("file");
    path
}

pub fn write_executable(dir: &Path, name: &str, text: &str) {
    let path = write(dir, name, text);
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(&path, fs::Permissions::from_mode(0o755)).expect("chmod");
    }
}

/// `text` as one single-quoted shell word.
pub fn sh(text: &str) -> String {
    format!("'{}'", text.replace('\'', "'\\''"))
}
