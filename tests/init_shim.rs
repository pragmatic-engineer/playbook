// SPDX-FileCopyrightText: 2026 Igor Santos
// SPDX-License-Identifier: Apache-2.0

//! Integration tests for `playbook::init::shim`, against the launcher's fixed config home.

#![allow(dead_code)]

use playbook::init::shim::{rewire_rc_file, upgrade_legacy_rc_files, ShellKind, SOURCE_LINE};
use std::env;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

/// The repo checkout root, where the shipped launcher runtime lives.
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
            grep_pattern: "playbook shell-init",
        },
        Case {
            name: "bash",
            shell_kind: ShellKind::Bash,
            rc_file_name: ".bashrc",
            grep_pattern: "playbook shell-init",
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
    assert!(contents.contains("eval \"$(playbook shell-init)\""));

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
    assert!(contents.contains("eval \"$(playbook shell-init)\""));

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
    let current_count = contents.lines().filter(|l| l.trim() == SOURCE_LINE).count();
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

/// An rc file written before the bash/zsh/shared layout split sources
/// `~/.claude/shell/cc.<ext>`, a transitional shim that itself sources the
/// current entry point. Appending beside it would load the launcher twice.
#[test]
fn rewire_rc_file_replaces_pre_layout_split_line_in_place_without_duplicating() {
    // Arrange
    struct Case {
        shell_kind: ShellKind,
        rc_file_name: &'static str,
        old_line: &'static str,
        current_line: &'static str,
    }
    let cases = [
        Case {
            shell_kind: ShellKind::Zsh,
            rc_file_name: ".zshrc",
            old_line: "source \"$HOME/.claude/shell/cc.zsh\"",
            current_line: SOURCE_LINE,
        },
        Case {
            shell_kind: ShellKind::Bash,
            rc_file_name: ".bashrc",
            old_line: "source \"$HOME/.claude/shell/cc.sh\"",
            current_line: SOURCE_LINE,
        },
    ];

    for case in cases {
        let home = temp_home(&format!("rc-pre-split-{}", case.rc_file_name));
        let rc_file = home.join(case.rc_file_name);
        write_file(
            &rc_file,
            &format!(
                "export EDITOR=vim\n\n# playbook launchers (cc/ccd)\n{}\n",
                case.old_line
            ),
        );

        // Act: twice, to prove the second call is a true no-op.
        let first = rewire_rc_file(&home, case.shell_kind).expect("first rewire should succeed");
        let second = rewire_rc_file(&home, case.shell_kind).expect("second rewire should succeed");

        // Assert
        assert!(first.appended, "the old line should have been replaced");
        assert!(!second.appended, "a second call should be a no-op");
        let contents = fs::read_to_string(&rc_file).expect("rc file should still exist");
        let count_of = |wanted: &str| contents.lines().filter(|l| l.trim() == wanted).count();
        assert_eq!(count_of(case.old_line), 0, "old line remains: {contents}");
        assert_eq!(
            count_of(case.current_line),
            1,
            "current line should appear exactly once: {contents}"
        );
        assert_eq!(
            contents.matches("launchers (cc/ccd)").count(),
            1,
            "the launchers comment should not be duplicated: {contents}"
        );

        let _ = fs::remove_dir_all(&home);
    }
}

const ZSH_CURRENT: &str = SOURCE_LINE;

fn count_lines_sourcing(contents: &str, needle: &str) -> usize {
    contents.lines().filter(|l| l.contains(needle)).count()
}

/// A re-run after a partial migration: the current line is already there and
/// a legacy line lingers. The legacy line must go, not stay beside it.
#[test]
fn rewire_rc_file_removes_legacy_lines_even_when_the_current_line_exists() {
    // Arrange
    let home = temp_home("rc-both-lines");
    let rc_file = home.join(".zshrc");
    write_file(
        &rc_file,
        &format!(
            "# playbook launchers (cc/ccd)\n{ZSH_CURRENT}\nsource \"$HOME/.claude/shell/cc.zsh\"\nalias ll='ls -la'\n"
        ),
    );

    // Act
    let first = rewire_rc_file(&home, ShellKind::Zsh).expect("rewire should succeed");
    let second = rewire_rc_file(&home, ShellKind::Zsh).expect("rewire should succeed again");

    // Assert
    assert!(first.appended, "removing a legacy line is a change");
    assert!(!second.appended, "the second run should be a no-op");
    let contents = fs::read_to_string(&rc_file).unwrap();
    assert_eq!(count_lines_sourcing(&contents, "cc.zsh"), 0, "{contents}");
    assert_eq!(
        count_lines_sourcing(&contents, "shell-init"),
        1,
        "{contents}"
    );
    assert_eq!(count_lines_sourcing(&contents, ".claude/shell"), 0);
    assert_eq!(contents.matches("launchers (cc/ccd)").count(), 1);
    assert!(contents.contains("alias ll='ls -la'"));

    let _ = fs::remove_dir_all(&home);
}

/// Every spelling of a legacy source line is migrated, and a comment that
/// merely mentions the path is left alone.
#[test]
fn rewire_rc_file_matches_legacy_lines_loosely() {
    // Arrange
    let forms = [
        "source \"$HOME/.claude/shell/zsh/cc.zsh\"",
        "source \"${HOME}/.claude/shell/zsh/cc.zsh\"",
        "source ~/.claude/shell/zsh/cc.zsh",
        "source '$HOME/.claude/shell/cc.zsh'",
        "source $HOME/.claude/shell/cc.zsh",
        "  source    \"$HOME/.claude/shell/zsh/cc.zsh\"   # launchers",
        ". \"$HOME/.claude/shell/zsh/cc.zsh\"",
        "\tsource ~/.claude/shell/cc.zsh",
    ];
    for (i, form) in forms.iter().enumerate() {
        let home = temp_home(&format!("rc-loose-{i}"));
        let rc_file = home.join(".zshrc");
        let comment = "# old: source ~/.claude/shell/zsh/cc.zsh";
        write_file(&rc_file, &format!("{comment}\n{form}\nexport A=1\n"));

        // Act
        let outcome = rewire_rc_file(&home, ShellKind::Zsh).expect("rewire should succeed");

        // Assert
        assert!(outcome.appended, "form {form:?} should be migrated");
        let contents = fs::read_to_string(&rc_file).unwrap();
        assert_eq!(
            contents,
            format!("{comment}\nexport A=1\n\n# playbook launchers (cc/ccd)\n{ZSH_CURRENT}\n"),
            "form {form:?}"
        );

        let _ = fs::remove_dir_all(&home);
    }
}

/// A lookalike that is not the legacy launcher must not be rewritten.
#[test]
fn rewire_rc_file_leaves_unrelated_cc_zsh_paths_alone() {
    // Arrange
    let home = temp_home("rc-lookalike");
    let rc_file = home.join(".zshrc");
    let unrelated =
        "source \"$HOME/.claude/shell/zsh/cc.zsh.bak\"\nsource ~/other/.claude/shell/cc.zsh\n";
    write_file(&rc_file, unrelated);

    // Act
    rewire_rc_file(&home, ShellKind::Zsh).expect("rewire should succeed");

    // Assert: appended after, originals untouched.
    let contents = fs::read_to_string(&rc_file).unwrap();
    assert!(contents.starts_with(unrelated), "{contents}");
    assert_eq!(contents.matches(ZSH_CURRENT).count(), 1);

    let _ = fs::remove_dir_all(&home);
}

/// A dotfile manager (stow, chezmoi) leaves `.zshrc` as a symlink; the
/// rewrite must update the real file, not replace the link.
#[cfg(unix)]
#[test]
fn rewire_rc_file_keeps_a_symlinked_rc_and_updates_its_target() {
    // Arrange
    let home = temp_home("rc-symlink");
    let target = home.join("dotfiles/zshrc");
    write_file(&target, "source \"$HOME/.claude/shell/zsh/cc.zsh\"\n");
    let rc_file = home.join(".zshrc");
    std::os::unix::fs::symlink(&target, &rc_file).unwrap();

    // Act
    let outcome = rewire_rc_file(&home, ShellKind::Zsh).expect("rewire should succeed");

    // Assert
    assert!(outcome.appended);
    assert!(
        fs::symlink_metadata(&rc_file)
            .unwrap()
            .file_type()
            .is_symlink(),
        "the rc file should still be a symlink"
    );
    assert_eq!(
        fs::read_to_string(&target).unwrap(),
        format!("# playbook launchers (cc/ccd)\n{ZSH_CURRENT}\n")
    );

    let _ = fs::remove_dir_all(&home);
}

/// An rc file with a non-UTF-8 byte is existing content, not an empty file:
/// it must survive byte for byte and gain exactly one source line.
#[test]
fn rewire_rc_file_preserves_a_non_utf8_rc_file() {
    // Arrange: a Latin-1 comment ahead of the current line.
    let home = temp_home("rc-latin1");
    let rc_file = home.join(".zshrc");
    let mut original = b"# configura\xe7\xe3o\n".to_vec();
    original.extend_from_slice(format!("{ZSH_CURRENT}\n").as_bytes());
    fs::write(&rc_file, &original).unwrap();

    // Act
    let unchanged = rewire_rc_file(&home, ShellKind::Zsh).expect("rewire should succeed");

    // Assert: nothing appended, bytes intact.
    assert!(!unchanged.appended);
    assert_eq!(fs::read(&rc_file).unwrap(), original);

    // Arrange: same rc, but with a legacy line instead.
    let mut legacy = b"# configura\xe7\xe3o\n".to_vec();
    legacy.extend_from_slice(b"source \"$HOME/.claude/shell/cc.zsh\"\n");
    fs::write(&rc_file, &legacy).unwrap();

    // Act
    rewire_rc_file(&home, ShellKind::Zsh).expect("rewire should succeed");

    // Assert: the legacy line is replaced by a block at the end, the Latin-1 bytes are intact.
    let mut migrated = b"# configura\xe7\xe3o\n\n# playbook launchers (cc/ccd)\n".to_vec();
    migrated.extend_from_slice(format!("{ZSH_CURRENT}\n").as_bytes());
    assert_eq!(fs::read(&rc_file).unwrap(), migrated);

    let _ = fs::remove_dir_all(&home);
}

/// Without the owner write bit the rc file is left alone and the outcome
/// says so, rather than renaming over it.
#[cfg(unix)]
#[test]
fn rewire_rc_file_leaves_a_read_only_rc_untouched() {
    use std::os::unix::fs::PermissionsExt;

    // Arrange
    let home = temp_home("rc-readonly");
    let rc_file = home.join(".zshrc");
    let before = "source \"$HOME/.claude/shell/cc.zsh\"\n";
    write_file(&rc_file, before);
    fs::set_permissions(&rc_file, fs::Permissions::from_mode(0o444)).unwrap();

    // Act
    let outcome = rewire_rc_file(&home, ShellKind::Zsh).expect("rewire should not error");

    // Assert
    assert!(outcome.unwritable);
    assert!(!outcome.appended);
    assert_eq!(fs::read_to_string(&rc_file).unwrap(), before);
    assert_eq!(
        fs::metadata(&rc_file).unwrap().permissions().mode() & 0o777,
        0o444
    );

    let _ = fs::remove_dir_all(&home);
}

/// The migration path: both rc files lose their `source` line for the
/// `shell-init` line, and a file without a legacy line is not touched.
#[test]
fn upgrade_legacy_rc_files_rewrites_every_legacy_form_and_skips_clean_files() {
    // Arrange
    let home = temp_home("upgrade-legacy");
    write_file(
        &home.join(".zshrc"),
        "# playbook launchers (cc/ccd)\nsource \"$HOME/.config/playbook/shell/zsh/cc.zsh\"\n",
    );
    write_file(
        &home.join(".bashrc"),
        "source ~/.config/playbook/shell/cc.sh\nexport A=1\n",
    );

    // Act
    let first = upgrade_legacy_rc_files(&home).expect("upgrade should succeed");
    let second = upgrade_legacy_rc_files(&home).expect("upgrade should be repeatable");

    // Assert
    assert_eq!(first.len(), 2);
    assert!(second.is_empty(), "a second run changes nothing");
    assert_eq!(
        fs::read_to_string(home.join(".zshrc")).unwrap(),
        format!("# playbook launchers (cc/ccd)\n{SOURCE_LINE}\n")
    );
    assert_eq!(
        fs::read_to_string(home.join(".bashrc")).unwrap(),
        format!("export A=1\n\n# playbook launchers (cc/ccd)\n{SOURCE_LINE}\n")
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
