// SPDX-FileCopyrightText: 2026 Igor Santos
// SPDX-License-Identifier: Apache-2.0

//! The `claude` function from `playbook shell-init` with `launcher.claudeOverride`
//! on: the bypass matrix and the definition guards, in bash and zsh, with a fake
//! `claude` and a fake `playbook` on PATH and a scratch HOME. Stdin is closed.

use playbook::init::shell_init::script_with;
use playbook::init::shim::ShellKind;
use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicU64, Ordering};

static COUNTER: AtomicU64 = AtomicU64::new(0);

struct Sandbox {
    root: PathBuf,
}

fn exec(path: &Path, body: &str) {
    fs::write(path, body).unwrap();
    fs::set_permissions(path, fs::Permissions::from_mode(0o755)).unwrap();
}

impl Sandbox {
    fn new(with_claude: bool) -> Self {
        let n = COUNTER.fetch_add(1, Ordering::Relaxed);
        let root = fs::canonicalize(std::env::temp_dir())
            .unwrap()
            .join(format!("pb-override-{}-{n}", playbook::testing::run_id()));
        fs::create_dir_all(root.join("bin")).unwrap();
        fs::create_dir_all(root.join("home")).unwrap();
        if with_claude {
            exec(
                &root.join("bin/claude"),
                "#!/bin/sh\necho \"CLAUDE $*\" >> \"$LOG\"\n",
            );
        }
        exec(
            &root.join("bin/playbook"),
            "#!/bin/sh\necho \"PLAYBOOK $*\" >> \"$LOG\"\n",
        );
        Sandbox { root }
    }

    /// Runs `prelude`, the generated script, then `body` in `shell`. `tty`
    /// stands in for the terminal test, which has no terminal here.
    fn run(
        &self,
        shell: &str,
        tty: bool,
        env: &[(&str, &str)],
        prelude: &str,
        body: &str,
    ) -> Vec<String> {
        let kind = if shell == "zsh" {
            ShellKind::Zsh
        } else {
            ShellKind::Bash
        };
        let script = self.root.join("init.sh");
        fs::write(&script, script_with(kind, true)).unwrap();
        let tty_stub = if tty {
            "_playbook_is_tty() { return 0; }"
        } else {
            ""
        };
        let program = format!("{prelude}\n. '{}'\n{tty_stub}\n{body}\n", script.display());
        let log = self.root.join("log");
        let _ = fs::remove_file(&log);
        let mut cmd = Command::new(shell);
        cmd.args(["-c", &program])
            .env("HOME", self.root.join("home"))
            .env(
                "PATH",
                format!("{}:/usr/bin:/bin", self.root.join("bin").display()),
            )
            .env("LOG", &log)
            .env_remove("PLAYBOOK_BYPASS")
            .stdin(Stdio::null());
        for (k, v) in env {
            cmd.env(k, v);
        }
        let out = cmd.output().unwrap();
        assert!(
            out.status.success(),
            "{shell}: {}",
            String::from_utf8_lossy(&out.stderr)
        );
        fs::read_to_string(&log)
            .unwrap_or_default()
            .lines()
            .map(String::from)
            .collect()
    }
}

fn shells() -> Vec<&'static str> {
    ["bash", "zsh"]
        .into_iter()
        .filter(|s| Command::new(s).arg("-c").arg("true").status().is_ok())
        .collect()
}

fn args_line(args: &[&str]) -> String {
    args.iter()
        .map(|a| format!("'{a}'"))
        .collect::<Vec<_>>()
        .join(" ")
}

const BYPASS: &[&[&str]] = &[
    &["-p", "hi"],
    &["--print", "hi"],
    &["-c", "-p", "hi"],
    &["--output-format", "json"],
    &["--output-format=stream-json"],
    &["--input-format", "stream-json"],
    &["--json-schema", "{}"],
    &["--max-turns", "3"],
    &["--max-budget-usd", "1"],
    &["--no-session-persistence"],
    &["--include-partial-messages"],
    &["--bare"],
    &["--safe-mode"],
    &["--restricted"],
    &["--bg", "task"],
    &["--version"],
    &["-v"],
    &["--help"],
    &["-h"],
    &["mcp", "list"],
    &["plugin", "install", "x"],
    &["plugins", "list"],
    &["update"],
    &["doctor"],
    &["config", "list"],
    &["auth", "status"],
    &["install", "stable"],
    &["agents", "--json"],
    &["--dangerously-skip-permissions", "doctor"],
    &["--system-prompt", "x"],
    &["--system-prompt=x"],
    &["--system-prompt-file", "f"],
    &["--append-system-prompt", "x"],
    &["--append-system-prompt-file=f"],
    &["--settings", "{}"],
    &["--settings=f.json"],
    &["--setting-sources", "user"],
    &["--agent", "a"],
];

const ROUTED: &[&[&str]] = &[
    &[],
    &["fix the bug"],
    &["-c"],
    &["--resume", "x"],
    &["--model", "opus"],
    &["--dangerously-skip-permissions"],
    &["--agents", "{}"],
    &["--", "-p"],
];

#[test]
fn every_bypass_case_reaches_the_real_claude_untouched() {
    for shell in shells() {
        let sb = Sandbox::new(true);
        for args in BYPASS {
            let log = sb.run(shell, true, &[], "", &format!("claude {}", args_line(args)));
            assert_eq!(
                log,
                vec![format!("CLAUDE {}", args.join(" "))],
                "{shell} {args:?}"
            );
        }
    }
}

#[test]
fn a_plain_interactive_session_goes_through_the_launcher() {
    for shell in shells() {
        let sb = Sandbox::new(true);
        for args in ROUTED {
            let log = sb.run(shell, true, &[], "", &format!("claude {}", args_line(args)));
            let want = if args.is_empty() {
                "PLAYBOOK cc launch --".to_string()
            } else {
                format!("PLAYBOOK cc launch -- {}", args.join(" "))
            };
            assert_eq!(log, vec![want], "{shell} {args:?}");
        }
    }
}

#[test]
fn no_terminal_on_stdin_or_stdout_bypasses() {
    for shell in shells() {
        let sb = Sandbox::new(true);
        // Without the terminal stand-in the real test sees a closed stdin.
        let log = sb.run(shell, false, &[], "", "claude");
        assert_eq!(log, vec!["CLAUDE ".to_string()], "{shell}");
        let log = sb.run(shell, false, &[], "", "claude 'fix it' | cat");
        assert_eq!(log, vec!["CLAUDE fix it".to_string()], "{shell}");
    }
}

#[test]
fn playbook_bypass_env_bypasses() {
    for shell in shells() {
        let sb = Sandbox::new(true);
        let log = sb.run(
            shell,
            true,
            &[("PLAYBOOK_BYPASS", "1")],
            "",
            "claude 'hello'",
        );
        assert_eq!(log, vec!["CLAUDE hello".to_string()], "{shell}");
        let log = sb.run(
            shell,
            true,
            &[("PLAYBOOK_BYPASS", "0")],
            "",
            "claude 'hello'",
        );
        assert_eq!(
            log,
            vec!["PLAYBOOK cc launch -- hello".to_string()],
            "{shell}"
        );
    }
}

#[test]
fn command_claude_skips_the_function_and_a_backslash_does_not() {
    for shell in shells() {
        let sb = Sandbox::new(true);
        let log = sb.run(shell, true, &[], "", "command claude hi; \\claude there");
        // A backslash only skips aliases; shells still run a function of that name.
        assert_eq!(
            log,
            vec![
                "CLAUDE hi".to_string(),
                "PLAYBOOK cc launch -- there".to_string()
            ],
            "{shell}"
        );
    }
}

#[test]
fn a_missing_launcher_binary_falls_back_to_claude() {
    for shell in shells() {
        let sb = Sandbox::new(true);
        fs::remove_file(sb.root.join("bin/playbook")).unwrap();
        let log = sb.run(shell, true, &[], "", "claude hi");
        assert_eq!(log, vec!["CLAUDE hi".to_string()], "{shell}");
    }
}

#[test]
fn nothing_is_defined_when_claude_is_not_installed() {
    for shell in shells() {
        let sb = Sandbox::new(false);
        let log = sb.run(
            shell,
            true,
            &[],
            "",
            "type claude >/dev/null 2>&1 && echo defined >> \"$LOG\"; true",
        );
        assert!(log.is_empty(), "{shell}: {log:?}");
    }
}

#[test]
fn a_user_function_named_claude_is_left_alone() {
    for shell in shells() {
        let sb = Sandbox::new(true);
        let log = sb.run(
            shell,
            true,
            &[],
            "claude() { echo USER \"$@\" >> \"$LOG\"; }",
            "claude hi",
        );
        assert_eq!(log, vec!["USER hi".to_string()], "{shell}");
    }
}

#[test]
fn a_user_alias_named_claude_is_left_alone() {
    for shell in shells() {
        let sb = Sandbox::new(true);
        let prelude = if shell == "bash" {
            "shopt -s expand_aliases"
        } else {
            ""
        };
        let body = "eval 'claude hi'";
        let alias = "alias claude='echo ALIAS >> \"$LOG\" #'";
        let log = sb.run(shell, true, &[], &format!("{prelude}\n{alias}"), body);
        assert_eq!(log, vec!["ALIAS".to_string()], "{shell}");
    }
}

#[test]
fn sourcing_twice_keeps_one_working_definition() {
    for shell in shells() {
        let sb = Sandbox::new(true);
        let script = sb.root.join("init.sh");
        let prelude = "";
        let body = format!(
            ". '{}'; _playbook_is_tty() {{ return 0; }}; claude hi",
            script.display()
        );
        // The first run writes init.sh; the body sources it a second time.
        let _ = sb.run(shell, true, &[], prelude, "true");
        let log = sb.run(shell, true, &[], prelude, &body);
        assert_eq!(log, vec!["PLAYBOOK cc launch -- hi".to_string()], "{shell}");
    }
}

#[test]
fn the_override_is_absent_unless_asked_for() {
    let text = playbook::init::shell_init::script_with(ShellKind::Bash, false);
    assert!(!text.contains("claude()"));
    assert!(text.contains("\nplaybook() {"));
}
