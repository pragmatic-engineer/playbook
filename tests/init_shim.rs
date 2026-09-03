// SPDX-FileCopyrightText: 2026 Igor Santos
// SPDX-License-Identifier: MIT

//! Integration tests for `playbook::init::shim` and
//! `playbook::init::statusline`, against the launcher's fixed config home.

#![allow(dead_code)]

use playbook::init::shim::{copy_launcher_runtime, rewire_rc_file, ShellKind};
use playbook::init::statusline::{
    place_statusline, playbook_statusline_path, resolve_statusline_path,
};
use std::env;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

/// The repo checkout root, where the shipped launcher runtime and
/// `statusline.sh` actually live.
fn self_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
}

static SCRATCH_COUNTER: AtomicU64 = AtomicU64::new(0);

/// A fresh scratch directory tree, standing in for a user's `$HOME`.
fn temp_home(tag: &str) -> PathBuf {
    let n = SCRATCH_COUNTER.fetch_add(1, Ordering::Relaxed);
    let home = env::temp_dir().join(format!(
        "playbook-init-shim-{}-{tag}-{n}",
        std::process::id()
    ));
    fs::create_dir_all(&home).expect("temp home should be creatable");
    home
}

fn write_file(path: &Path, content: &str) {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).expect("parent dir should be creatable");
    }
    fs::write(path, content).expect("scratch file should be writable");
}

#[test]
fn rc_file_gains_exactly_one_source_line_across_repeated_runs() {
    // Arrange: table-driven over both shells.
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
            grep_pattern: ".config/playbook/shell/zsh/cc.zsh",
        },
        Case {
            name: "bash",
            shell_kind: ShellKind::Bash,
            rc_file_name: ".bashrc",
            grep_pattern: ".config/playbook/shell/bash/cc.sh",
        },
    ];

    for case in cases {
        let home = temp_home(&format!("rc-idempotent-{}", case.name));
        let rc_file = home.join(case.rc_file_name);

        // Act: three calls; one call would not catch an append-every-time bug.
        for _ in 0..3 {
            rewire_rc_file(&home, case.shell_kind)
                .unwrap_or_else(|e| panic!("{}: rewire_rc_file failed: {e}", case.name));
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
fn rewire_rc_file_creates_a_missing_rc_file() {
    // Arrange: a home directory with no `.zshrc` at all.
    let home = temp_home("rc-missing");
    let rc_file = home.join(".zshrc");
    assert!(!rc_file.exists(), "arrange: rc file should not exist yet");

    // Act
    let outcome = rewire_rc_file(&home, ShellKind::Zsh)
        .expect("rewire_rc_file should create a missing rc file");

    // Assert
    assert!(
        outcome.appended,
        "the source line should have been appended"
    );
    assert_eq!(outcome.rc_file, rc_file);
    let contents = fs::read_to_string(&rc_file).expect("rc file should now exist");
    assert!(contents.contains(".config/playbook/shell/zsh/cc.zsh"));

    let _ = fs::remove_dir_all(&home);
}

#[test]
fn rewire_rc_file_preserves_unrelated_rc_content() {
    // Arrange: a `.bashrc` with content unrelated to the launcher.
    let home = temp_home("rc-unrelated-content");
    let rc_file = home.join(".bashrc");
    let unrelated = "export EDITOR=vim\nalias ll='ls -la'\n";
    write_file(&rc_file, unrelated);

    // Act
    rewire_rc_file(&home, ShellKind::Bash)
        .expect("rewire_rc_file should succeed against an rc file with existing content");

    // Assert
    let contents = fs::read_to_string(&rc_file).expect("rc file should still exist");
    assert!(
        contents.starts_with(unrelated),
        "unrelated existing content should survive untouched, got:\n{contents}"
    );
    assert!(contents.contains(".config/playbook/shell/bash/cc.sh"));

    let _ = fs::remove_dir_all(&home);
}

/// The migration case: an rc file sourcing the pre-ADR-0012 line gets that
/// exact line replaced in place, across two calls, never duplicated.
#[test]
fn rewire_rc_file_replaces_legacy_line_in_place_without_duplicating() {
    // Arrange
    let home = temp_home("rc-legacy-replace");
    let rc_file = home.join(".zshrc");
    let before = "export EDITOR=vim\n\n# playbook launchers (cc/ccd)\nsource \"$HOME/.claude/shell/zsh/cc.zsh\"\n\nalias ll='ls -la'\n";
    write_file(&rc_file, before);

    // Act: twice, to prove the second call is a true no-op.
    let first = rewire_rc_file(&home, ShellKind::Zsh).expect("first rewire should succeed");
    let second = rewire_rc_file(&home, ShellKind::Zsh).expect("second rewire should succeed");

    // Assert
    assert!(first.appended, "the legacy line should have been replaced");
    assert!(
        !second.appended,
        "a second call should find the current line already in place"
    );
    let contents = fs::read_to_string(&rc_file).expect("rc file should still exist");
    let legacy_count = contents
        .lines()
        .filter(|l| l.trim() == "source \"$HOME/.claude/shell/zsh/cc.zsh\"")
        .count();
    let current_count = contents
        .lines()
        .filter(|l| l.trim() == "source \"$HOME/.config/playbook/shell/zsh/cc.zsh\"")
        .count();
    assert_eq!(
        legacy_count, 0,
        "the legacy line should be gone: {contents}"
    );
    assert_eq!(
        current_count, 1,
        "the current line should appear exactly once: {contents}"
    );
    assert!(contents.contains("export EDITOR=vim"));
    assert!(contents.contains("alias ll='ls -la'"));

    let _ = fs::remove_dir_all(&home);
}

#[test]
fn copy_launcher_runtime_copies_the_launcher_runtime_files() {
    // Arrange
    let home = temp_home("copies-runtime");

    // Act
    copy_launcher_runtime(&self_root(), &home).expect("copy_launcher_runtime should succeed");

    // Assert: files land under `$HOME/.config/playbook/shell`.
    let dst_shell = home.join(".config/playbook/shell");
    assert!(dst_shell.join("zsh/cc.zsh").is_file());
    assert!(dst_shell.join("bash/cc.sh").is_file());
    assert!(dst_shell.join("shared/dispatch.sh").is_file());
    assert!(
        !dst_shell.join("shared/launcher.test.sh").exists(),
        "*.test.sh files should not be copied"
    );

    let _ = fs::remove_dir_all(&home);
}

#[test]
fn detect_shell_recognises_zsh_bash_and_neither() {
    // Arrange, Act, Assert: table-driven.
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
    // Arrange: no statusline.sh present yet at the fixed destination.
    let home = temp_home("statusline-regression");
    let expected_path = playbook_statusline_path(&home);
    assert!(
        !expected_path.exists(),
        "arrange: statusline.sh should be missing before init"
    );

    // Act
    let placed = place_statusline(&self_root(), &home)
        .expect("place_statusline should restore the missing statusline");
    assert_eq!(placed, expected_path);

    // Assert: stat and open it directly, rather than trusting the return value.
    let metadata = fs::metadata(&placed)
        .unwrap_or_else(|e| panic!("statusline path should exist after init: {e}"));
    assert!(
        metadata.is_file(),
        "statusline path should be a regular file"
    );
    fs::File::open(&placed).unwrap_or_else(|e| panic!("statusline path should be readable: {e}"));

    let _ = fs::remove_dir_all(&home);
}

#[test]
fn statusline_placement_matches_source_content_and_is_executable() {
    // Arrange
    let home = temp_home("statusline-content");
    let source = self_root().join("statusline.sh");
    let expected_content = fs::read(&source).expect("repo statusline.sh should be readable");

    // Act
    let placed = place_statusline(&self_root(), &home).expect("place_statusline should succeed");

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

    // Act
    let first = place_statusline(&self_root(), &home).expect("first place_statusline");
    let second = place_statusline(&self_root(), &home).expect("second place_statusline");

    // Assert
    assert_eq!(first, second, "both runs should resolve to the same path");
    let metadata = fs::metadata(&first).expect("statusline.sh should exist after both runs");
    assert!(metadata.is_file());

    let _ = fs::remove_dir_all(&home);
}

#[test]
fn resolve_statusline_path_agrees_with_the_fixed_destination() {
    // Arrange: a settings.json shaped like a freshly written one.
    let home = temp_home("resolve-agrees");
    let settings_path = home.join(".claude/settings.json");
    write_file(
        &settings_path,
        r#"{"statusLine":{"command":"bash $HOME/.config/playbook/statusline.sh"}}"#,
    );

    // Act
    let resolved = resolve_statusline_path(&settings_path, &home).expect("command should resolve");

    // Assert: the two independent sources of truth agree.
    assert_eq!(resolved, playbook_statusline_path(&home));

    let _ = fs::remove_dir_all(&home);
}

#[test]
fn resolve_statusline_path_fails_on_missing_settings_file() {
    // Arrange: no settings.json at all under this home.
    let home = temp_home("resolve-missing-settings");
    let settings_path = home.join(".claude/settings.json");

    // Act, Assert
    assert!(resolve_statusline_path(&settings_path, &home).is_err());

    let _ = fs::remove_dir_all(&home);
}
