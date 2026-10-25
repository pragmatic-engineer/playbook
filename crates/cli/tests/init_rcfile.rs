// SPDX-FileCopyrightText: 2026 Igor Santos
// SPDX-License-Identifier: Apache-2.0

//! The rc-file matcher behind the `retired-launcher-rc-line` migration (ADR
//! 0044): which lines the retired launcher wrote, and how they are removed,
//! in a scratch `$HOME`.

use playbook::init::rcfile::{retired_rc_files, strip_retired_rc_lines, SOURCE_LINE};
use std::env;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

static SCRATCH_COUNTER: AtomicU64 = AtomicU64::new(0);

/// A fresh scratch directory tree, standing in for a user's `$HOME`.
fn temp_home(tag: &str) -> PathBuf {
    let n = SCRATCH_COUNTER.fetch_add(1, Ordering::Relaxed);
    let home = env::temp_dir().join(format!(
        "playbook-init-rcfile-{}-{tag}-{n}",
        playbook::testing::run_id()
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

fn backups(home: &Path, name: &str) -> Vec<PathBuf> {
    fs::read_dir(home)
        .unwrap()
        .flatten()
        .map(|e| e.path())
        .filter(|p| {
            p.file_name()
                .unwrap()
                .to_string_lossy()
                .starts_with(&format!("{name}.bak-"))
        })
        .collect()
}

#[test]
fn the_old_marker_block_is_removed_after_a_backup_and_a_second_run_finds_nothing() {
    // Arrange
    let home = temp_home("block");
    let rc_file = home.join(".zshrc");
    let before = format!(
        "export EDITOR=vim\n\n# playbook launchers (ccc/ccd)\n{SOURCE_LINE}\nalias ll=ls\n"
    );
    write_file(&rc_file, &before);

    // Act
    let pending = retired_rc_files(&home);
    let changed = strip_retired_rc_lines(&home, 42);
    let again = strip_retired_rc_lines(&home, 43);

    // Assert
    assert_eq!(pending, vec![rc_file.clone()]);
    assert_eq!(changed.len(), 1);
    assert_eq!(changed[0].backup, Some(home.join(".zshrc.bak-42")));
    assert_eq!(
        fs::read_to_string(home.join(".zshrc.bak-42")).unwrap(),
        before
    );
    assert_eq!(
        fs::read_to_string(&rc_file).unwrap(),
        "export EDITOR=vim\n\nalias ll=ls\n"
    );
    assert!(again.is_empty());
    assert!(retired_rc_files(&home).is_empty());
    assert_eq!(backups(&home, ".zshrc").len(), 1);
}

#[test]
fn a_lone_marker_and_the_older_wording_are_removed_in_both_rc_files() {
    // Arrange
    let home = temp_home("lone");
    write_file(
        &home.join(".zshrc"),
        "a=1\n# playbook launchers (ccc/ccd)\nb=2\n",
    );
    write_file(
        &home.join(".bashrc"),
        "# playbook launchers (cc/ccd)\nsource \"$HOME/.claude/shell/bash/cc.sh\"\nc=3\n",
    );

    // Act
    let changed = strip_retired_rc_lines(&home, 1);

    // Assert
    assert_eq!(changed.len(), 2);
    assert_eq!(
        fs::read_to_string(home.join(".zshrc")).unwrap(),
        "a=1\nb=2\n"
    );
    assert_eq!(fs::read_to_string(home.join(".bashrc")).unwrap(), "c=3\n");
}

#[test]
fn a_bare_hand_written_eval_goes_and_other_content_stays() {
    // Arrange
    let home = temp_home("bare");
    let rc_file = home.join(".zshrc");
    write_file(
        &rc_file,
        "# my own launcher\neval \"$(playbook shell-init)\"\nexport A=1\n",
    );

    // Act
    let pending = retired_rc_files(&home);
    let changed = strip_retired_rc_lines(&home, 1);

    // Assert
    assert_eq!(pending, std::slice::from_ref(&rc_file));
    assert_eq!(changed.len(), 1);
    assert_eq!(
        fs::read_to_string(&rc_file).unwrap(),
        "# my own launcher\nexport A=1\n"
    );
    assert_eq!(backups(&home, ".zshrc").len(), 1);
}

#[test]
fn a_bare_eval_with_the_old_shell_flag_goes_too() {
    for shell in ["zsh", "bash"] {
        // Arrange
        let home = temp_home(&format!("flag-{shell}"));
        let rc_file = home.join(".zshrc");
        write_file(
            &rc_file,
            &format!("a=1\neval \"$(playbook shell-init --shell {shell})\"\n"),
        );

        // Act
        let changed = strip_retired_rc_lines(&home, 1);

        // Assert
        assert_eq!(changed.len(), 1, "{shell}");
        assert_eq!(fs::read_to_string(&rc_file).unwrap(), "a=1\n", "{shell}");
    }
}

#[test]
fn an_indented_bare_eval_inside_the_users_own_block_stays() {
    // Arrange
    let home = temp_home("indented");
    let rc_file = home.join(".bashrc");
    let before = "if command -v playbook >/dev/null; then\n  eval \"$(playbook shell-init)\"\nfi\n";
    write_file(&rc_file, before);

    // Act
    let pending = retired_rc_files(&home);
    let changed = strip_retired_rc_lines(&home, 1);

    // Assert
    assert!(pending.is_empty());
    assert!(changed.is_empty());
    assert_eq!(fs::read_to_string(&rc_file).unwrap(), before);
}

#[test]
fn legacy_source_lines_match_loosely() {
    let forms = [
        "source \"$HOME/.claude/shell/zsh/cc.zsh\"",
        "source \"${HOME}/.claude/shell/zsh/cc.zsh\"",
        "source ~/.claude/shell/zsh/cc.zsh",
        "source '$HOME/.claude/shell/cc.zsh'",
        "source $HOME/.claude/shell/cc.zsh",
        "  source    \"$HOME/.claude/shell/zsh/cc.zsh\"   # launchers",
        ". \"$HOME/.config/playbook/shell/zsh/cc.zsh\"",
        "\tsource ~/.config/playbook/shell/cc.zsh",
    ];
    for (i, form) in forms.iter().enumerate() {
        // Arrange
        let home = temp_home(&format!("loose-{i}"));
        let rc_file = home.join(".zshrc");
        let comment = "# old: source ~/.claude/shell/zsh/cc.zsh";
        write_file(&rc_file, &format!("{comment}\n{form}\nexport A=1\n"));

        // Act
        strip_retired_rc_lines(&home, 1);

        // Assert
        assert_eq!(
            fs::read_to_string(&rc_file).unwrap(),
            format!("{comment}\nexport A=1\n"),
            "form {form:?}"
        );
    }
}

#[test]
fn lookalike_paths_are_not_matched() {
    // Arrange
    let home = temp_home("lookalike");
    write_file(
        &home.join(".zshrc"),
        "source \"$HOME/.claude/shell/zsh/cc.zsh.bak\"\nsource ~/other/.claude/shell/cc.zsh\n",
    );

    // Act and assert
    assert!(retired_rc_files(&home).is_empty());
}

/// A dotfile manager (stow, chezmoi) leaves `.zshrc` as a symlink.
#[cfg(unix)]
#[test]
fn a_symlinked_rc_stays_a_symlink_and_its_target_changes() {
    // Arrange
    let home = temp_home("symlink");
    let target = home.join("dotfiles/zshrc");
    write_file(&target, &format!("x=1\n{SOURCE_LINE}\n"));
    let rc_file = home.join(".zshrc");
    std::os::unix::fs::symlink(&target, &rc_file).unwrap();

    // Act
    strip_retired_rc_lines(&home, 1);

    // Assert
    assert!(fs::symlink_metadata(&rc_file)
        .unwrap()
        .file_type()
        .is_symlink());
    assert_eq!(fs::read_to_string(&target).unwrap(), "x=1\n");
}

#[test]
fn a_non_utf8_rc_file_keeps_its_other_bytes() {
    // Arrange
    let home = temp_home("latin1");
    let rc_file = home.join(".zshrc");
    let mut before = b"# configura\xe7\xe3o\n".to_vec();
    before.extend_from_slice(format!("{SOURCE_LINE}\n").as_bytes());
    fs::write(&rc_file, &before).unwrap();

    // Act
    strip_retired_rc_lines(&home, 1);

    // Assert
    assert_eq!(fs::read(&rc_file).unwrap(), b"# configura\xe7\xe3o\n");
}

#[test]
fn a_failed_backup_reports_the_error_and_leaves_the_rc_file() {
    // Arrange
    let home = temp_home("bak-fails");
    let rc_file = home.join(".zshrc");
    let before = format!("{SOURCE_LINE}\n");
    write_file(&rc_file, &before);
    fs::create_dir_all(home.join(".zshrc.bak-7")).unwrap();

    // Act
    let changed = strip_retired_rc_lines(&home, 7);

    // Assert
    assert_eq!(changed.len(), 1);
    assert!(changed[0].error.is_some());
    assert_eq!(changed[0].backup, None);
    assert_eq!(fs::read_to_string(&rc_file).unwrap(), before);
}

#[cfg(unix)]
#[test]
fn a_read_only_rc_is_not_reported_or_touched() {
    use std::os::unix::fs::PermissionsExt;

    // Arrange
    let home = temp_home("readonly");
    let rc_file = home.join(".zshrc");
    let before = format!("{SOURCE_LINE}\n");
    write_file(&rc_file, &before);
    fs::set_permissions(&rc_file, fs::Permissions::from_mode(0o444)).unwrap();

    // Act
    let pending = retired_rc_files(&home);
    let changed = strip_retired_rc_lines(&home, 1);

    // Assert
    assert!(pending.is_empty());
    assert!(changed.is_empty());
    assert_eq!(fs::read_to_string(&rc_file).unwrap(), before);
    assert!(backups(&home, ".zshrc").is_empty());
}
