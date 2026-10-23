// SPDX-FileCopyrightText: 2026 Igor Santos
// SPDX-License-Identifier: Apache-2.0

//! `playbook completions <shell>` prints a script for the approved shells and
//! touches nothing, from the real binary.

#[path = "support/isolated.rs"]
mod isolated;

use std::io::{BufRead, BufReader, Write};
#[cfg(unix)]
use std::os::unix::process::ExitStatusExt;
use std::process::{Command, Output, Stdio};

const FISH_FIRST_LINE: &str = "# Print an optspec for argparse to handle cmd's options that are independent of any subcommand.";
#[cfg(unix)]
const SIGPIPE: i32 = 13;
const PIPE_BUFFER_BYTES: usize = 64 * 1024;

fn skip(test: &str, why: &str) {
    eprintln!("skipped {test}: {why}");
}

fn run(args: &[&str]) -> Output {
    isolated::playbook()
        .args(args)
        .env("HTTPS_PROXY", "127.0.0.1:9")
        .env("HTTP_PROXY", "127.0.0.1:9")
        .output()
        .expect("spawn playbook")
}

fn script(shell: &str) -> String {
    let out = run(&["completions", shell]);
    assert_eq!(out.status.code(), Some(0), "{shell} exited non-zero");
    String::from_utf8(out.stdout).expect("utf-8 script")
}

#[test]
fn completions_prints_a_script_for_each_approved_shell() {
    for shell in ["bash", "zsh", "fish"] {
        // Arrange
        let args = ["completions", shell];

        // Act
        let out = run(&args);

        // Assert
        let stdout = String::from_utf8_lossy(&out.stdout);
        assert_eq!(out.status.code(), Some(0), "{shell}");
        assert!(stdout.contains("playbook"), "{shell}: {stdout}");
        assert!(out.stderr.is_empty(), "{shell}: {:?}", out.stderr);
    }
}

#[test]
fn completions_rejects_a_shell_outside_the_approved_three_and_names_it() {
    for shell in ["elvish", "powershell"] {
        // Arrange
        let args = ["completions", shell];

        // Act
        let out = run(&args);

        // Assert
        let stderr = String::from_utf8_lossy(&out.stderr);
        assert_eq!(out.status.code(), Some(2), "{shell}");
        assert!(stderr.contains(shell), "{shell}: {stderr}");
    }
}

#[test]
fn completions_touches_nothing_under_home() {
    // Arrange
    let home = isolated::fresh_home();

    // Act
    let out = isolated::playbook_in(&home)
        .args(["completions", "zsh"])
        .output()
        .expect("spawn playbook");

    // Assert
    let entries: Vec<_> = std::fs::read_dir(&home).unwrap().collect();
    assert_eq!(out.status.code(), Some(0));
    assert!(!out.stdout.is_empty(), "no script was printed");
    assert!(entries.is_empty(), "home was written: {entries:?}");
}

#[cfg(unix)]
#[test]
fn completions_exits_cleanly_on_a_closed_pipe() {
    // Arrange
    let mut cmd = isolated::playbook();
    let mut child = cmd
        .args(["completions", "zsh"])
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("spawn playbook");
    let mut first = String::new();
    let full_len = script("zsh").len();

    // Act
    BufReader::new(child.stdout.take().unwrap())
        .read_line(&mut first)
        .unwrap();
    let out = child.wait_with_output().unwrap();

    // Assert
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(!first.is_empty(), "no script line was printed");
    assert!(
        full_len > PIPE_BUFFER_BYTES,
        "the script fits the pipe buffer, so the pipe never closes early"
    );
    assert!(
        !stderr.contains("panicked") && !stderr.contains("Broken pipe"),
        "{stderr}"
    );
    // The default SIGPIPE action ends the process, as in `playbook ... | head`.
    assert_eq!(out.status.signal(), Some(SIGPIPE), "{stderr}");
}

#[test]
fn completions_is_a_hot_path_command_and_skips_the_home_check() {
    // Arrange
    // The helper builds the command; env_clear then drops HOME on purpose.
    let mut cmd = isolated::playbook();
    cmd.env_clear().args(["completions", "bash"]);

    // Act
    let out = cmd.output().expect("spawn playbook");

    // Assert
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert_eq!(out.status.code(), Some(0), "{stderr}");
    assert!(!out.stdout.is_empty(), "no script was printed");
}

#[test]
fn the_bash_script_starts_with_the_function_line_has_a_complete_line_and_guards_pb() {
    // Arrange
    let text = script("bash");

    // Act
    let first = text.lines().next();

    // Assert
    assert_eq!(first, Some("_playbook() {"));
    assert!(text.contains("complete -F _playbook"), "no complete line");
    assert!(
        text.contains("command -v pb"),
        "pb is registered without a guard"
    );
}

#[test]
fn the_bash_script_passes_a_syntax_check() {
    // Arrange
    let text = script("bash");
    let Ok(mut child) = Command::new("bash")
        .args(["-n", "/dev/stdin"])
        .stdin(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
    else {
        skip(
            "the_bash_script_passes_a_syntax_check",
            "bash cannot be spawned",
        );
        return;
    };

    // Act
    child
        .stdin
        .take()
        .unwrap()
        .write_all(text.as_bytes())
        .unwrap();
    let out = child.wait_with_output().unwrap();

    // Assert
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(!text.is_empty(), "the script is empty");
    assert_eq!(out.status.code(), Some(0), "{stderr}");
}

#[test]
fn the_zsh_script_starts_with_compdef_for_both_names() {
    // Arrange
    let text = script("zsh");

    // Act
    let first = text.lines().next();

    // Assert
    assert_eq!(first, Some("#compdef playbook pb"));
}

#[test]
fn the_fish_script_starts_with_the_pinned_line() {
    // Arrange
    let text = script("fish");

    // Act
    let first = text.lines().next();

    // Assert
    assert_eq!(first, Some(FISH_FIRST_LINE));
    assert!(text.contains("-c playbook"), "playbook is not registered");
    assert!(text.contains("pb"), "pb is not registered");
    assert!(
        text.contains("command -sq pb") || text.contains("type -q pb"),
        "pb is registered without a guard"
    );
}

#[test]
fn the_zsh_script_guards_pb_with_a_same_file_check_and_an_if_block() {
    // Arrange
    let text = script("zsh");

    // Act
    let has_guard = text.contains(
        "if (( $+functions[compdef] )) && [[ ${commands[pb]} -ef ${commands[playbook]} ]]; then",
    );

    // Assert
    assert!(has_guard, "the zsh guard lacks the same-file check");
    assert!(text.contains("-ef"));
}

/// Sources the script in `shell` with a PATH holding no `pb`, under errexit.
fn sourcing_with_no_pb_succeeds(shell: &str, prelude: &str) {
    // Arrange
    let test = format!("sourcing_{shell}_script_without_pb");
    if Command::new(shell).arg("--version").output().is_err() {
        skip(&test, &format!("{shell} cannot be spawned"));
        return;
    }
    let scratch = isolated::fresh_home();
    let body =
        format!("PATH=\"$1\"; {prelude} set -e; source <(\"$2\" completions {shell}); echo ok");

    // Act
    let out = Command::new(shell)
        .args(["-c", &body, "_"])
        .arg(&scratch)
        .arg(env!("CARGO_BIN_EXE_playbook"))
        .env("HOME", &scratch)
        .output()
        .expect("spawn shell");

    // Assert
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert_eq!(out.status.code(), Some(0), "{shell}: {stderr}");
    assert_eq!(String::from_utf8_lossy(&out.stdout).trim(), "ok", "{shell}");
}

#[test]
fn sourcing_the_bash_script_without_pb_on_path_succeeds_under_errexit() {
    sourcing_with_no_pb_succeeds("bash", "");
}

#[test]
fn sourcing_the_zsh_script_without_pb_on_path_succeeds_under_errexit() {
    sourcing_with_no_pb_succeeds("zsh", "compdef() { :; };");
}
