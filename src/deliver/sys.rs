// SPDX-FileCopyrightText: 2026 Igor Santos
// SPDX-License-Identifier: Apache-2.0

//! The small adapter between `deliver` and the machine: `git` and `gh` run as
//! child processes with fixed argument vectors, never through a shell, in the
//! worktree being delivered. Output is never echoed, so a token that a tool
//! prints on a failure only reaches the caller through [`scrub`].

use crate::common::proc::run_with_timeout;
use std::path::PathBuf;
use std::process::Command;
use std::time::Duration;

const SHORT: Duration = Duration::from_secs(30);
const NET: Duration = Duration::from_secs(120);
const HOOKS: Duration = Duration::from_secs(600);

/// How long each kind of call may run.
#[derive(Clone, Copy)]
pub enum Wait {
    Short,
    Net,
    Hooks,
}

/// Where and how child processes run.
#[derive(Clone)]
pub struct Sys {
    pub dir: PathBuf,
    pub git: String,
    pub gh: String,
    /// Extra environment for every child, such as a scratch `HOME` in tests.
    pub env: Vec<(String, String)>,
}

/// What a child printed and whether it succeeded.
pub struct Out {
    pub ok: bool,
    pub stdout: String,
    pub stderr: String,
}

impl Sys {
    fn run(&self, program: &str, args: &[&str], wait: Wait) -> Out {
        let mut command = Command::new(program);
        command.args(args).current_dir(&self.dir);
        command
            .env("GIT_TERMINAL_PROMPT", "0")
            .env("GH_PROMPT_DISABLED", "1");
        for (key, value) in &self.env {
            command.env(key, value);
        }
        let limit = match wait {
            Wait::Short => SHORT,
            Wait::Net => NET,
            Wait::Hooks => HOOKS,
        };
        match run_with_timeout(&mut command, limit) {
            Some(o) => Out {
                ok: o.status.success(),
                stdout: scrub(String::from_utf8_lossy(&o.stdout).trim()),
                stderr: scrub(String::from_utf8_lossy(&o.stderr).trim()),
            },
            None => Out {
                ok: false,
                stdout: String::new(),
                stderr: format!("{program} did not finish (missing, or timed out)"),
            },
        }
    }

    pub fn git(&self, args: &[&str]) -> Out {
        self.run(&self.git, args, Wait::Short)
    }

    pub fn git_wait(&self, args: &[&str], wait: Wait) -> Out {
        self.run(&self.git, args, wait)
    }

    /// Trimmed stdout of a git call that succeeded.
    pub fn git_out(&self, args: &[&str]) -> Option<String> {
        let out = self.git(args);
        out.ok.then_some(out.stdout)
    }

    pub fn gh(&self, args: &[&str]) -> Out {
        self.run(&self.gh, args, Wait::Net)
    }
}

/// Hides anything shaped like a token or a credential in a URL.
pub fn scrub(text: &str) -> String {
    let token = crate::common::re::static_regex(
        r"(gh[pousr]_[A-Za-z0-9]{20,}|github_pat_[A-Za-z0-9_]{20,}|://[^/@\s]+:[^/@\s]+@)",
    );
    token.replace_all(text, "[redacted]").into_owned()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_token_in_tool_output_is_redacted() {
        let raw = "fatal: https://user:hunter2@host/x and ghp_abcdefghijklmnopqrstuvwxyz0123";

        let clean = scrub(raw);

        assert!(
            !clean.contains("hunter2") && !clean.contains("ghp_abc"),
            "got {clean}"
        );
    }
}
