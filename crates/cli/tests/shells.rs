// SPDX-FileCopyrightText: 2026 Igor Santos
// SPDX-License-Identifier: Apache-2.0

//! The completion scripts from `playbook completions <shell>` parse and
//! complete in the real shells. `PLAYBOOK_REQUIRE_SHELLS=1` turns a missing
//! shell into a failure (the CI job sets it); without it a missing or ungated
//! shell test prints a skip message (nextest shows it only with
//! `--no-capture`). Only the bash syntax check always runs
//! wherever bash exists.

#[path = "support/isolated.rs"]
mod isolated;
#[path = "support/shell_exec.rs"]
mod shell_exec;
#[path = "support/shell_fixture.rs"]
mod shell_fixture;

use shell_fixture::Work;
use std::path::{Path, PathBuf};
use std::process::{Command, Output, Stdio};

const REQUIRE_SHELLS_VAR: &str = "PLAYBOOK_REQUIRE_SHELLS";
const REQUIRE_SHELLS_ON: &str = "1";
const SKIP_MESSAGE: &str = "SKIPPED: shell tests need PLAYBOOK_REQUIRE_SHELLS=1";
const BASH: &str = "bash";
const ZSH: &str = "zsh";
const FISH: &str = "fish";
const BINARY: &str = "playbook";
const ALIAS: &str = "pb";
/// Seconds the zsh driver waits for any one read from the pty.
const PTY_DEADLINE_SECS: u32 = 10;
const PTY_PROMPT: &str = "READY> ";
const TAB: &str = "\t";
const TERMINAL: &str = "xterm-256color";
/// A zsh with no startup files, for the fpath-function scenario.
const BARE_ZSH_ARGS: &str = "-f";
/// An interactive zsh that reads `$ZDOTDIR/.zshrc` but not the global files.
const SETUP_ZSH_ARGS: &str = "-d -i";
/// Proxies nothing listens on, so a shell started by a test cannot reach the
/// network. Apply after `env_clear`.
const OFFLINE_ENV: [(&str, &str); 2] = [
    ("HTTPS_PROXY", "127.0.0.1:9"),
    ("HTTP_PROXY", "127.0.0.1:9"),
];

/// What a gated test does, decided from the opt-in value and the shell.
#[derive(Debug, PartialEq)]
enum Gate {
    Run,
    SkipVisible,
    Fail,
}

/// Takes the opt-in value as an argument so the decision is testable
/// without touching the process environment.
fn shell_gate(opt_in: Option<&str>, shell_found: bool) -> Gate {
    match (opt_in == Some(REQUIRE_SHELLS_ON), shell_found) {
        (true, true) => Gate::Run,
        (true, false) => Gate::Fail,
        (false, _) => Gate::SkipVisible,
    }
}

fn find_on_path(shell: &str) -> Option<PathBuf> {
    let paths = std::env::var_os("PATH")?;
    std::env::split_paths(&paths)
        .map(|dir| dir.join(shell))
        .find(|candidate| candidate.is_file())
}

fn on_path(shell: &str) -> bool {
    find_on_path(shell).is_some()
}

/// True when the test should run. Otherwise prints the skip message (nextest
/// captures it for a passing test, so it shows only with `--no-capture`) or
/// panics naming the missing shell.
fn gated(shell: &str) -> bool {
    let opt_in = std::env::var(REQUIRE_SHELLS_VAR).ok();
    match shell_gate(opt_in.as_deref(), on_path(shell)) {
        Gate::Run => true,
        Gate::SkipVisible => {
            println!("{SKIP_MESSAGE}");
            false
        }
        Gate::Fail => panic!("{REQUIRE_SHELLS_VAR} is set but {shell} is not on PATH"),
    }
}

fn completions_script(shell: &str) -> String {
    let out = isolated::playbook()
        .args(["completions", shell])
        .output()
        .expect("spawn playbook");
    assert_eq!(out.status.code(), Some(0), "{shell} completions failed");
    String::from_utf8(out.stdout).expect("utf-8 script")
}

fn write_script(work: &Work, shell: &str) -> PathBuf {
    let path = work.path(&format!("{shell}-completions"));
    std::fs::write(&path, completions_script(shell)).expect("write script");
    path
}

/// A directory holding only the built `playbook` and, when asked, a `pb`
/// symlink to it.
fn bin_dir(work: &Work, with_alias: bool) -> PathBuf {
    let dir = work.dir(if with_alias { "bin-alias" } else { "bin" });
    let built = Path::new(env!("CARGO_BIN_EXE_playbook"));
    std::os::unix::fs::symlink(built, dir.join(BINARY)).expect("link playbook");
    if with_alias {
        std::os::unix::fs::symlink(built, dir.join(ALIAS)).expect("link pb");
    }
    dir
}

/// `bin` first, then the caller's PATH so the shell itself still resolves.
fn path_with(bin: &Path) -> std::ffi::OsString {
    let mut dirs = vec![bin.to_path_buf()];
    dirs.extend(std::env::split_paths(
        &std::env::var_os("PATH").unwrap_or_default(),
    ));
    std::env::join_paths(dirs).expect("join PATH")
}

fn shell_in(work: &Work, shell: &str, bin: &Path) -> Command {
    let mut cmd = Command::new(shell);
    cmd.env_clear()
        .env("HOME", work.dir("home"))
        .env("ZDOTDIR", work.dir("home"))
        .env("XDG_CONFIG_HOME", work.dir("home"))
        .env("XDG_DATA_HOME", work.dir("home"))
        .env("PATH", path_with(bin))
        .envs(OFFLINE_ENV);
    cmd
}

fn syntax_check(shell: &str) {
    // Arrange
    let work = Work::new("syntax");
    let script = write_script(&work, shell);

    // Act
    let out = shell_in(&work, shell, &work.dir("bin"))
        .arg("-n")
        .arg(&script)
        .output()
        .expect("spawn shell");

    // Assert
    assert!(out.status.success(), "{shell} -n: {}", text(&out));
}

fn text(out: &Output) -> String {
    format!(
        "{}{}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    )
}

/// Runs bash completion for `words` with the cursor on the last one and
/// returns the candidates, one per line.
fn bash_candidates(command: &str, bin: &Path, work: &Work, words: &[&str]) -> String {
    let script = write_script(work, BASH);
    let cword = words.len() - 1;
    let driver = format!(
        "source '{script}'\n\
         COMP_WORDS=({words})\nCOMP_CWORD={cword}\n\
         COMP_LINE='{words}'\nCOMP_POINT=${{#COMP_LINE}}\n\
         fn=$(complete -p '{command}' | sed -n 's/.*-F \\([^ ]*\\).*/\\1/p')\n\
         [ -n \"$fn\" ] || {{ echo 'no completion registered for {command}' >&2; exit 1; }}\n\
         \"$fn\" {command} '{cur}' '{prev}'\n\
         printf '%s\\n' \"${{COMPREPLY[@]}}\"\n",
        script = script.display(),
        words = words.join(" "),
        cur = words[cword],
        prev = words[cword - 1],
    );
    let out = shell_in(work, BASH, bin)
        .args(["--noprofile", "--norc", "-c", &driver])
        .output()
        .expect("spawn bash");
    assert!(out.status.success(), "bash completion: {}", text(&out));
    String::from_utf8_lossy(&out.stdout).into_owned()
}

fn has_line(output: &str, wanted: &str) -> bool {
    output.lines().any(|l| l == wanted)
}

/// Types `typed` plus a tab into an interactive zsh on a pty and waits for
/// `expected` to show up. With `PB_STUB_DIR` set, zsh starts bare and gets the
/// completion script as an fpath function; otherwise it starts with
/// `PB_ZSH_ARGS` and reads the `.zshrc` that setup wrote. The driver is a zsh
/// script because zsh/zpty needs no extra crate.
const ZPTY_DRIVER: &str = r#"
zmodload zsh/zpty zsh/system
transcript=''
fail() { print -r -- "$1"; print -r -- "--- transcript ---"; print -r -- "$transcript"; exit 1 }
setopt extendedglob
# Reads until `$1` shows up in what arrived after this call, or the deadline.
# The line editor styles the inserted text, so escape codes are dropped first.
wait_for() {
  local needle=$1 chunk mark=${#transcript} plain
  local deadline=$(( SECONDS + PB_DEADLINE ))
  while true; do
    plain=${transcript[mark+1,-1]//$'\e'\[[0-9;?]#[a-zA-Z]/}
    [[ $plain == *"$needle"* ]] && break
    (( SECONDS >= deadline )) && return 1
    sysread -t 1 -i $fd chunk && transcript+=$chunk
  done
}
zpty child "$PB_ZSH" ${(z)PB_ZSH_ARGS}
fd=$REPLY
send() { syswrite -o $fd -- "$1" }
if [[ -n $PB_STUB_DIR ]]; then
  send $'fpath=('"$PB_STUB_DIR"$' $fpath)\n'
  send $'autoload -Uz compinit && compinit -u -D\n'
fi
# The prompt reaches the inner shell by env so the echoed line never holds
# the prompt text, which would satisfy the wait before the prompt is drawn.
send $'PS1="$PB_PROMPT"\n'
wait_for "$PB_PROMPT" || fail "no prompt within ${PB_DEADLINE}s"
send "$PB_TYPED"
wait_for "$PB_EXPECTED" || fail "completion did not produce '$PB_EXPECTED'"
zpty -d child
"#;

fn zsh_pty_completes(with_alias: bool, typed: &str, expected: &str) {
    let work = Work::new("zpty");
    let bin = bin_dir(&work, with_alias);
    let stub_dir = work.dir("stub");
    std::fs::write(stub_dir.join("_playbook"), completions_script(ZSH)).expect("stub");
    zsh_pty(&work, &bin, BARE_ZSH_ARGS, &stub_dir, typed, expected);
}

fn zsh_pty(work: &Work, bin: &Path, zsh_args: &str, stub_dir: &Path, typed: &str, expected: &str) {
    // Arrange
    let driver = work.path("driver.zsh");
    std::fs::write(&driver, ZPTY_DRIVER).expect("driver");
    let home = work.dir("home");

    // Act
    let zsh = find_on_path(ZSH).expect("zsh on PATH");
    let out = Command::new(&zsh)
        .env_clear()
        .envs(OFFLINE_ENV)
        .env("PB_ZSH", &zsh)
        .env("HOME", &home)
        .env("ZDOTDIR", &home)
        .env("PATH", bin)
        .env("TERM", TERMINAL)
        .env("PB_ZSH_ARGS", zsh_args)
        .env("PB_STUB_DIR", stub_dir)
        .env("PB_DEADLINE", PTY_DEADLINE_SECS.to_string())
        .env("PB_PROMPT", PTY_PROMPT)
        .env("PB_TYPED", format!("{typed}{TAB}"))
        .env("PB_EXPECTED", expected)
        .args(["-f"])
        .arg(&driver)
        .output()
        .expect("spawn zsh");

    // Assert
    assert!(out.status.success(), "zsh pty: {}", text(&out));
}

#[test]
fn the_shell_gate_decides_from_the_injected_value() {
    // Arrange
    let cases = [
        (Some("1"), true, Gate::Run),
        (Some("1"), false, Gate::Fail),
        (Some(""), true, Gate::SkipVisible),
        (Some("0"), true, Gate::SkipVisible),
        (Some(""), false, Gate::SkipVisible),
        (Some("0"), false, Gate::SkipVisible),
        (None, true, Gate::SkipVisible),
        (None, false, Gate::SkipVisible),
    ];

    for (opt_in, found, expected) in cases {
        // Act
        let gate = shell_gate(opt_in, found);

        // Assert
        assert_eq!(gate, expected, "opt_in {opt_in:?}, found {found}");
    }
}

#[test]
fn bash_script_parses() {
    if !on_path(BASH) && !gated(BASH) {
        return;
    }
    syntax_check(BASH);
}

#[test]
fn zsh_script_parses() {
    if !gated(ZSH) {
        return;
    }
    syntax_check(ZSH);
}

#[test]
fn fish_script_parses() {
    if !gated(FISH) {
        return;
    }
    syntax_check(FISH);
}

#[test]
fn bash_completes_a_subcommand() {
    if !gated(BASH) {
        return;
    }
    for (with_alias, command) in [(false, BINARY), (true, ALIAS)] {
        // Arrange
        let work = Work::new("bash-complete");
        let bin = bin_dir(&work, with_alias);

        // Act
        let candidates = bash_candidates(command, &bin, &work, &[command, "plan", "ch"]);

        // Assert
        assert!(
            has_line(&candidates, "checkpoint"),
            "{command}: {candidates}"
        );
    }
}

#[test]
fn fish_completes_a_subcommand() {
    if !gated(FISH) {
        return;
    }
    for (with_alias, command, typed, wanted) in [
        (false, BINARY, "playbook tr", "trust"),
        (true, ALIAS, "pb pl", "plan"),
    ] {
        // Arrange
        let work = Work::new("fish-complete");
        let bin = bin_dir(&work, with_alias);
        let script = write_script(&work, FISH);
        let program = format!("source '{}'; complete -C '{typed}'", script.display());

        // Act
        let out = shell_in(&work, FISH, &bin)
            .args(["--no-config", "-c", &program])
            .output()
            .expect("spawn fish");

        // Assert
        let stdout = String::from_utf8_lossy(&out.stdout);
        assert!(
            stdout.lines().any(|l| l.split('\t').next() == Some(wanted)),
            "{command}: {}",
            text(&out)
        );
    }
}

#[test]
fn zsh_completes_through_a_pty() {
    if !gated(ZSH) {
        return;
    }
    zsh_pty_completes(false, "playbook tr", "playbook trust ");
    // `pb pl` is ambiguous (plan, plans), so zsh adds no trailing space.
    zsh_pty_completes(true, "pb tr", "pb trust ");
}

/// Runs `playbook setup --yes --only completions` for zsh in the work home,
/// as a user would, so the `.zshrc` and the loader are the ones setup wrote.
fn setup_zsh_home(work: &Work, bin: &Path) {
    let home = work.dir("home");
    let zsh = find_on_path(ZSH).expect("zsh on PATH");
    let out = Command::new(bin.join(BINARY))
        .args(["setup", "--yes", "--only", "completions"])
        .env_clear()
        .envs(OFFLINE_ENV)
        .env("HOME", &home)
        .env("PATH", bin)
        .env("SHELL", zsh)
        .env("CLAUDE_PLUGIN_ROOT", env!("PLAYBOOK_REPO_ROOT"))
        .current_dir(&home)
        .stdin(Stdio::null())
        .output()
        .expect("spawn setup");
    assert!(out.status.success(), "setup: {}", text(&out));
}

#[test]
fn zsh_completes_through_the_files_setup_wrote() {
    if !gated(ZSH) {
        return;
    }
    for (with_alias, typed, expected) in [
        (false, "playbook tr", "playbook trust "),
        (true, "pb tr", "pb trust "),
    ] {
        // Arrange
        let work = Work::new("zpty-setup");
        let bin = bin_dir(&work, with_alias);
        setup_zsh_home(&work, &bin);

        // Act and assert
        zsh_pty(&work, &bin, SETUP_ZSH_ARGS, Path::new(""), typed, expected);
    }
}

#[test]
fn a_zsh_started_after_setup_prints_nothing_without_the_binary_on_path() {
    if !gated(ZSH) {
        return;
    }
    // Arrange
    let work = Work::new("zsh-silent");
    let bin = bin_dir(&work, true);
    setup_zsh_home(&work, &bin);
    let home = work.dir("home");
    let zsh = find_on_path(ZSH).expect("zsh on PATH");

    // Act
    let out = Command::new(zsh)
        .env_clear()
        .envs(OFFLINE_ENV)
        .env("HOME", &home)
        .env("ZDOTDIR", &home)
        .env("PATH", work.dir("empty"))
        .args(["-d", "-i", "-c", "exit 0"])
        .stdin(Stdio::null())
        .output()
        .expect("spawn zsh");

    // Assert
    assert_eq!(out.status.code(), Some(0), "{}", text(&out));
    assert!(
        out.stdout.is_empty() && out.stderr.is_empty(),
        "{}",
        text(&out)
    );
}

/// A `playbook` that fails noisily, standing in for a broken install.
const FAILING_PLAYBOOK: &str = "#!/bin/sh\necho 'playbook: broken' >&2\necho 'noise'\nexit 1\n";

#[test]
fn a_stub_is_silent_when_completions_fails_or_the_binary_is_missing() {
    for shell in [BASH, ZSH, FISH] {
        if !gated(shell) {
            continue;
        }
        let kind = playbook::init::completion::Shell::detect(shell);
        for playbook_fails in [true, false] {
            // Arrange
            let work = Work::new("stub-silence");
            let bin = work.dir("bin");
            if playbook_fails {
                shell_exec::write_exec(&bin.join(BINARY), FAILING_PLAYBOOK);
            }
            let stub = work.path("stub");
            let script = match kind {
                playbook::init::completion::Shell::Zsh => playbook::init::completion::loader_text(),
                _ => playbook::init::completion::stub_text(kind),
            };
            std::fs::write(&stub, script).expect("stub");
            let shell_path = find_on_path(shell).expect("shell on PATH");

            // Act
            let out = Command::new(&shell_path)
                .env_clear()
                .envs(OFFLINE_ENV)
                .env("HOME", work.dir("home"))
                .env("PATH", &bin)
                .arg("-c")
                .arg(format!("source '{}'", stub.display()))
                .output()
                .expect("spawn shell");

            // Assert
            let case = format!("{shell}, playbook fails: {playbook_fails}");
            assert_eq!(out.status.code(), Some(0), "{case}: {}", text(&out));
            assert!(out.stdout.is_empty(), "{case}: stdout {}", text(&out));
            assert!(out.stderr.is_empty(), "{case}: stderr {}", text(&out));
        }
    }
}
