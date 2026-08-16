// SPDX-FileCopyrightText: 2026 Igor Santos
// SPDX-License-Identifier: MIT

//! Integration tests for `playbook::init::shim` and
//! `playbook::init::statusline`, covering every scenario the Work Unit
//! brief names:
//! - the rc file gains EXACTLY ONE source line across three repeated `init`
//!   runs (`rc_file_gains_exactly_one_source_line_across_repeated_runs`)
//! - both shells, `.zshrc` and `.bashrc`
//!   (`rc_file_gains_exactly_one_source_line_across_repeated_runs`, table-driven)
//! - an rc file that does not exist yet
//!   (`install_shim_creates_a_missing_rc_file`)
//! - an rc file with unrelated content that must survive
//!   (`install_shim_preserves_unrelated_rc_content`)
//! - the 2026-08-12 outage regression pin: after `init`, the `statusLine`
//!   command path exists and is readable, resolved independently out of
//!   `settings.json` rather than trusted from the placer's return value
//!   (`statusline_regression_pin_command_path_exists_and_is_readable_after_init`)
//!
//! Every test here uses a scratch directory standing in for `$HOME`; none
//! read or write the developer's real `~/.zshrc`, `~/.bashrc` or
//! `~/.claude`.

#![allow(dead_code)]

use playbook::init::shim::{install_shim, ShellKind};
use playbook::init::statusline::{place_statusline, resolve_statusline_path};
use std::env;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

/// The repo checkout root, where `shell/bash/cc.sh`, `shell/zsh/cc.zsh`,
/// `shell/shared/*.sh` and `statusline.sh` actually live.
fn self_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
}

static SCRATCH_COUNTER: AtomicU64 = AtomicU64::new(0);

/// A fresh scratch directory tree under the OS temp dir, standing in for a
/// user's `$HOME`, unique per call so parallel tests never collide and never
/// touch the real filesystem outside of `env::temp_dir()`.
fn temp_home(tag: &str) -> PathBuf {
    let n = SCRATCH_COUNTER.fetch_add(1, Ordering::Relaxed);
    let home = env::temp_dir().join(format!(
        "playbook-init-shim-{}-{tag}-{n}",
        std::process::id()
    ));
    fs::create_dir_all(&home).expect("temp home should be creatable");
    home
}

/// The `~/.claude` directory under a `temp_home`, matching `CLAUDE_HOME` in
/// `setup-local.sh`.
fn claude_home_of(home: &Path) -> PathBuf {
    home.join(".claude")
}

fn write_file(path: &Path, content: &str) {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).expect("parent dir should be creatable");
    }
    fs::write(path, content).expect("scratch file should be writable");
}

/// A `~/.claude/settings.json` naming a statusline path under `home` via a
/// literal `$HOME` token, matching the live shape:
/// `"command": "bash $HOME/.claude/statusline.sh"`.
fn write_statusline_settings(home: &Path) -> PathBuf {
    let claude_home = claude_home_of(home);
    let settings_path = claude_home.join("settings.json");
    write_file(
        &settings_path,
        r#"{"statusLine":{"type":"command","command":"bash $HOME/.claude/statusline.sh","refreshInterval":30}}"#,
    );
    settings_path
}

#[test]
fn rc_file_gains_exactly_one_source_line_across_repeated_runs() {
    // Arrange: table-driven over both shells, since the two cases share
    // exactly the same shape (only the shell kind and rc file name differ).
    struct Case {
        name: &'static str,
        shell_kind: ShellKind,
        rc_file_name: &'static str,
        grep_pattern: &'static str,
    }
    let cases = [
        Case {
            name: "zsh",
            shell_kind: ShellKind::Zsh,
            rc_file_name: ".zshrc",
            grep_pattern: "shell/zsh/cc.zsh",
        },
        Case {
            name: "bash",
            shell_kind: ShellKind::Bash,
            rc_file_name: ".bashrc",
            grep_pattern: "shell/bash/cc.sh",
        },
    ];

    for case in cases {
        let home = temp_home(&format!("rc-idempotent-{}", case.name));
        let claude_home = claude_home_of(&home);
        let rc_file = home.join(case.rc_file_name);

        // Act: run `install_shim` three times. Once would not catch an
        // append-every-time bug; three proves the line count stays fixed
        // past the first run, not just that the first run and a hypothetical
        // second happen to agree.
        for _ in 0..3 {
            install_shim(&self_root(), &claude_home, &home, case.shell_kind)
                .unwrap_or_else(|e| panic!("{}: install_shim failed: {e}", case.name));
        }

        // Assert
        let contents = fs::read_to_string(&rc_file)
            .unwrap_or_else(|e| panic!("{}: rc file should exist: {e}", case.name));
        let matching_lines = contents
            .lines()
            .filter(|line| line.contains(case.grep_pattern))
            .count();
        assert_eq!(
            matching_lines, 1,
            "{}: rc file should gain exactly one source line across 3 runs, got:\n{contents}",
            case.name
        );

        let _ = fs::remove_dir_all(&home);
    }
}

#[test]
fn install_shim_creates_a_missing_rc_file() {
    // Arrange: a home directory with no `.zshrc` at all.
    let home = temp_home("rc-missing");
    let claude_home = claude_home_of(&home);
    let rc_file = home.join(".zshrc");
    assert!(!rc_file.exists(), "arrange: rc file should not exist yet");

    // Act
    let outcome = install_shim(&self_root(), &claude_home, &home, ShellKind::Zsh)
        .expect("install_shim should create a missing rc file");

    // Assert
    assert!(
        outcome.appended,
        "the source line should have been appended"
    );
    assert_eq!(outcome.rc_file, rc_file);
    let contents = fs::read_to_string(&rc_file).expect("rc file should now exist");
    assert!(contents.contains("shell/zsh/cc.zsh"));

    let _ = fs::remove_dir_all(&home);
}

#[test]
fn install_shim_preserves_unrelated_rc_content() {
    // Arrange: a `.bashrc` with content that has nothing to do with the
    // launcher, which must survive `init` untouched.
    let home = temp_home("rc-unrelated-content");
    let claude_home = claude_home_of(&home);
    let rc_file = home.join(".bashrc");
    let unrelated = "export EDITOR=vim\nalias ll='ls -la'\n";
    write_file(&rc_file, unrelated);

    // Act
    install_shim(&self_root(), &claude_home, &home, ShellKind::Bash)
        .expect("install_shim should succeed against an rc file with existing content");

    // Assert
    let contents = fs::read_to_string(&rc_file).expect("rc file should still exist");
    assert!(
        contents.starts_with(unrelated),
        "unrelated existing content should survive untouched, got:\n{contents}"
    );
    assert!(contents.contains("shell/bash/cc.sh"));

    let _ = fs::remove_dir_all(&home);
}

#[test]
fn install_shim_copies_the_launcher_runtime_files() {
    // Arrange
    let home = temp_home("copies-runtime");
    let claude_home = claude_home_of(&home);

    // Act
    install_shim(&self_root(), &claude_home, &home, ShellKind::Zsh)
        .expect("install_shim should succeed");

    // Assert: the entry point for the wired shell, and the shared modules it
    // sources, are present. The unwired shell's entry point (bash's cc.sh)
    // is copied too: setup-local.sh copies both regardless of which one gets
    // wired into the rc file, so a later shell switch still works.
    assert!(claude_home.join("shell/zsh/cc.zsh").is_file());
    assert!(claude_home.join("shell/bash/cc.sh").is_file());
    assert!(claude_home.join("shell/shared/dispatch.sh").is_file());
    assert!(
        !claude_home.join("shell/shared/launcher.test.sh").exists(),
        "*.test.sh files should not be copied"
    );

    let _ = fs::remove_dir_all(&home);
}

#[test]
fn detect_shell_recognises_zsh_bash_and_neither() {
    // Arrange, Act, Assert: table-driven, all three cases share the same
    // "call detect and compare" shape.
    let cases = [
        ("/bin/zsh", Some(ShellKind::Zsh)),
        ("/usr/bin/zsh", Some(ShellKind::Zsh)),
        ("/bin/bash", Some(ShellKind::Bash)),
        ("/usr/local/bin/bash", Some(ShellKind::Bash)),
        ("/usr/bin/fish", None),
        ("", None),
    ];
    for (shell_env, expected) in cases {
        assert_eq!(
            ShellKind::detect(shell_env),
            expected,
            "detect({shell_env:?}) should be {expected:?}"
        );
    }
}

#[test]
fn statusline_regression_pin_command_path_exists_and_is_readable_after_init() {
    // Arrange: settings.json shaped exactly like the live outage case,
    // `statusLine.command` = "bash $HOME/.claude/statusline.sh", with no
    // statusline.sh present yet at that path, matching "a machine with a
    // missing statusline".
    let home = temp_home("statusline-regression");
    let settings_path = write_statusline_settings(&home);
    let expected_path = claude_home_of(&home).join("statusline.sh");
    assert!(
        !expected_path.exists(),
        "arrange: statusline.sh should be missing before init, reproducing the outage"
    );

    // Act
    let placed = place_statusline(&self_root(), &settings_path, &home)
        .expect("place_statusline should restore the missing statusline");
    assert_eq!(placed, expected_path);

    // Assert: resolve the path a second time, independently out of
    // settings.json via the public resolver, rather than trusting
    // `place_statusline`'s own return value, then stat and open it directly.
    let resolved_again = resolve_statusline_path(&settings_path, &home)
        .expect("the path should still resolve out of settings.json after init");
    assert_eq!(resolved_again, expected_path);
    let metadata = fs::metadata(&resolved_again)
        .unwrap_or_else(|e| panic!("statusLine command path should exist after init: {e}"));
    assert!(
        metadata.is_file(),
        "statusLine command path should be a regular file"
    );
    fs::File::open(&resolved_again)
        .unwrap_or_else(|e| panic!("statusLine command path should be readable: {e}"));

    let _ = fs::remove_dir_all(&home);
}

#[test]
fn statusline_placement_matches_source_content_and_is_executable() {
    // Arrange
    let home = temp_home("statusline-content");
    let settings_path = write_statusline_settings(&home);
    let source = self_root().join("statusline.sh");
    let expected_content = fs::read(&source).expect("repo statusline.sh should be readable");

    // Act
    let placed = place_statusline(&self_root(), &settings_path, &home)
        .expect("place_statusline should succeed");

    // Assert
    let placed_content = fs::read(&placed).expect("placed statusline.sh should be readable");
    assert_eq!(
        placed_content, expected_content,
        "placed statusline.sh should match the repo source byte for byte"
    );

    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mode = fs::metadata(&placed).unwrap().permissions().mode() & 0o111;
        assert_ne!(mode, 0, "placed statusline.sh should stay executable");
    }

    let _ = fs::remove_dir_all(&home);
}

#[test]
fn statusline_placement_is_idempotent_on_repeated_runs() {
    // Arrange
    let home = temp_home("statusline-idempotent");
    let settings_path = write_statusline_settings(&home);

    // Act: run twice, matching "Running init on a machine with a missing
    // statusline restores it" plus a subsequent run on a machine that
    // already has it.
    let first = place_statusline(&self_root(), &settings_path, &home)
        .expect("first place_statusline should succeed");
    let second = place_statusline(&self_root(), &settings_path, &home)
        .expect("second place_statusline should succeed");

    // Assert
    assert_eq!(first, second, "both runs should resolve to the same path");
    let metadata = fs::metadata(&first).expect("statusline.sh should exist after both runs");
    assert!(metadata.is_file());

    let _ = fs::remove_dir_all(&home);
}

#[test]
fn statusline_missing_settings_file_fails_with_a_settings_error() {
    // Arrange: no settings.json at all under this home.
    let home = temp_home("statusline-missing-settings");
    let settings_path = claude_home_of(&home).join("settings.json");

    // Act
    let result = place_statusline(&self_root(), &settings_path, &home);

    // Assert
    assert!(
        result.is_err(),
        "placing a statusline with no settings.json should fail, not silently no-op"
    );

    let _ = fs::remove_dir_all(&home);
}

#[test]
fn statusline_missing_status_line_key_fails_with_a_settings_error() {
    // Arrange: a settings.json with no `statusLine` key at all.
    let home = temp_home("statusline-missing-key");
    let settings_path = claude_home_of(&home).join("settings.json");
    write_file(&settings_path, r#"{"otherKey":"value"}"#);

    // Act
    let result = place_statusline(&self_root(), &settings_path, &home);

    // Assert
    assert!(
        result.is_err(),
        "a settings.json with no statusLine.command should fail, not guess a destination"
    );

    let _ = fs::remove_dir_all(&home);
}
