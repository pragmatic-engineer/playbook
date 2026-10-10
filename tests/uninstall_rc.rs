// SPDX-FileCopyrightText: 2026 Igor Santos
// SPDX-License-Identifier: Apache-2.0

//! `strip_rc_files`: the launcher block comes out of the rc files, in every
//! form an install of any generation wrote, and nothing else does.

use playbook::init::shim::strip_rc_files;
use std::fs;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};

static COUNTER: AtomicU64 = AtomicU64::new(0);

/// A scratch `$HOME` that derefs to its path and is removed on drop.
struct Home(PathBuf);

impl std::ops::Deref for Home {
    type Target = PathBuf;
    fn deref(&self) -> &PathBuf {
        &self.0
    }
}

impl Drop for Home {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

fn home(tag: &str) -> Home {
    let n = COUNTER.fetch_add(1, Ordering::Relaxed);
    let dir = std::env::temp_dir().join(format!(
        "playbook-rc-{tag}-{}-{n}",
        playbook::testing::run_id()
    ));
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir).expect("scratch home");
    Home(dir)
}

/// Strip `before` from `.zshrc` and return what is left.
fn strip_zsh(tag: &str, before: &[u8]) -> Vec<u8> {
    let h = home(tag);
    fs::write(h.join(".zshrc"), before).expect("rc file");
    strip_rc_files(&h, 1, false, false);
    fs::read(h.join(".zshrc")).expect("rc file")
}

#[test]
fn the_current_shell_init_block_goes_with_its_comment() {
    let before = b"# BEFORE\n\n# playbook launchers (cc/ccd)\ncommand -v playbook >/dev/null 2>&1 && eval \"$(playbook shell-init)\"\n# AFTER\n";
    assert_eq!(strip_zsh("current", before), b"# BEFORE\n\n# AFTER\n");
}

#[test]
fn a_hand_written_eval_line_goes_too() {
    assert_eq!(
        strip_zsh("bare", b"a\neval \"$(playbook shell-init)\"\nb\n"),
        b"a\nb\n"
    );
}

#[test]
fn both_older_comment_variants_and_a_missing_comment_are_handled() {
    for comment in [
        "# claude-config launchers (cc/ccd)",
        "# playbook launchers (cc/ccd)",
        "# playbook launchers (ccc/ccd)",
    ] {
        let before =
            format!("# BEFORE\n{comment}\nsource \"$HOME/.claude/shell/cc.zsh\"\n# AFTER\n");
        assert_eq!(strip_zsh("old", before.as_bytes()), b"# BEFORE\n# AFTER\n");
    }
    let bare = b"# BEFORE\nsource \"$HOME/.claude/shell/zsh/cc.zsh\"\n# AFTER\n";
    assert_eq!(strip_zsh("bare-source", bare), b"# BEFORE\n# AFTER\n");
}

#[test]
fn the_config_dir_launcher_paths_are_recognised() {
    let before = b"x\n. ~/.config/playbook/shell/zsh/cc.zsh\ny\n";
    assert_eq!(strip_zsh("config-dir", before), b"x\ny\n");
}

#[test]
fn a_blank_line_on_each_side_collapses_to_one() {
    let before = b"export A=1\n\n# playbook launchers (cc/ccd)\nsource \"$HOME/.claude/shell/zsh/cc.zsh\"\n\nexport B=2\n";
    assert_eq!(strip_zsh("blank", before), b"export A=1\n\nexport B=2\n");
}

#[test]
fn the_blank_line_init_added_before_the_block_goes_with_it_at_the_end_of_file() {
    let before = b"export A=1\n\n# playbook launchers (cc/ccd)\neval \"$(playbook shell-init)\"\n";
    assert_eq!(strip_zsh("tail", before), b"export A=1\n");
}

#[test]
fn a_file_without_the_block_is_not_touched_or_backed_up() {
    let h = home("untouched");
    let before = b"export A=1\n\n\n\nexport B=2\n";
    fs::write(h.join(".zshrc"), before).unwrap();
    let changed = strip_rc_files(&h, 1, true, false);
    assert!(changed.is_empty());
    assert_eq!(fs::read(h.join(".zshrc")).unwrap(), before);
    assert!(!h.join(".zshrc.bak-1").exists());
}

#[test]
fn a_user_blank_run_elsewhere_is_left_as_written() {
    let before = b"a\n\n\n\nb\n# playbook launchers (cc/ccd)\neval \"$(playbook shell-init)\"\n";
    assert_eq!(strip_zsh("runs", before), b"a\n\n\n\nb\n");
}

#[test]
fn bash_loses_only_its_own_block_and_zsh_keeps_its_own() {
    let h = home("trap");
    let before = b"# BEFORE\n# playbook launchers (cc/ccd)\nsource \"$HOME/.claude/shell/bash/cc.sh\"\n# playbook launchers (cc/ccd)\nsource \"$HOME/.claude/shell/zsh/cc.zsh\"\n# AFTER\n";
    fs::write(h.join(".bashrc"), before).unwrap();
    strip_rc_files(&h, 1, false, false);
    assert_eq!(
        fs::read(h.join(".bashrc")).unwrap(),
        b"# BEFORE\n# playbook launchers (cc/ccd)\nsource \"$HOME/.claude/shell/zsh/cc.zsh\"\n# AFTER\n"
    );
}

#[test]
fn bash_old_form_and_shell_init_blocks_are_stripped_from_bashrc() {
    let h = home("bash-forms");
    fs::write(
        h.join(".bashrc"),
        b"# BEFORE\n\n# playbook launchers (cc/ccd)\nsource \"$HOME/.claude/shell/cc.sh\"\n# AFTER\n",
    )
    .unwrap();
    strip_rc_files(&h, 1, false, false);
    assert_eq!(
        fs::read(h.join(".bashrc")).unwrap(),
        b"# BEFORE\n\n# AFTER\n"
    );

    let current = b"x=1\n\n# playbook launchers (cc/ccd)\ncommand -v playbook >/dev/null 2>&1 && eval \"$(playbook shell-init)\"\n";
    fs::write(h.join(".bashrc"), current).unwrap();
    strip_rc_files(&h, 2, false, false);
    assert_eq!(fs::read(h.join(".bashrc")).unwrap(), b"x=1\n");
}

#[cfg(unix)]
#[test]
fn a_read_only_rc_file_is_reported_and_left_alone() {
    use std::os::unix::fs::PermissionsExt;
    let h = home("readonly");
    let before = b"# playbook launchers (cc/ccd)\neval \"$(playbook shell-init)\"\n";
    let rc = h.join(".zshrc");
    fs::write(&rc, before).unwrap();
    fs::set_permissions(&rc, fs::Permissions::from_mode(0o444)).unwrap();
    let changed = strip_rc_files(&h, 1, false, false);
    assert_eq!(changed.len(), 1);
    assert!(changed[0].unwritable);
    assert_eq!(fs::read(&rc).unwrap(), before);
    assert!(!h.join(".zshrc.bak-1").exists());
}

#[test]
fn a_non_utf8_rc_file_keeps_its_other_bytes() {
    let before = b"\xff\xfe=1\n# playbook launchers (cc/ccd)\neval \"$(playbook shell-init)\"\n";
    assert_eq!(strip_zsh("bytes", before), b"\xff\xfe=1\n");
}

#[test]
fn the_binary_path_block_goes_only_when_asked_and_a_users_path_edit_stays() {
    let before = b"export PATH=\"/opt/mytools:$PATH\"\n\n# playbook binary\nexport PATH=\"/h/.local/bin:$PATH\"\n";
    let h = home("path");
    fs::write(h.join(".zshrc"), before).unwrap();
    assert!(strip_rc_files(&h, 1, false, false).is_empty());
    strip_rc_files(&h, 2, true, false);
    assert_eq!(
        fs::read(h.join(".zshrc")).unwrap(),
        b"export PATH=\"/opt/mytools:$PATH\"\n"
    );
    assert_eq!(fs::read(h.join(".zshrc.bak-2")).unwrap(), before);
}

#[test]
fn a_dry_run_reports_the_file_and_writes_nothing() {
    let h = home("dry");
    let before = b"# playbook launchers (cc/ccd)\neval \"$(playbook shell-init)\"\n";
    fs::write(h.join(".zshrc"), before).unwrap();
    let changed = strip_rc_files(&h, 1, false, true);
    assert_eq!(changed.len(), 1);
    assert_eq!(fs::read(h.join(".zshrc")).unwrap(), before);
    assert!(!h.join(".zshrc.bak-1").exists());
}
