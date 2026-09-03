// SPDX-FileCopyrightText: 2026 Igor Santos
// SPDX-License-Identifier: MIT

//! Installs the `cc`/`ccd` launcher runtime under
//! `$HOME/.config/playbook/shell/` and wires the rc file to source it.

use crate::common::paths::playbook_root_from;
use std::fs;
use std::io;
use std::io::Write;
use std::path::{Path, PathBuf};

/// Which shell family the rc-file wiring targets.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ShellKind {
    Bash,
    Zsh,
}

impl ShellKind {
    /// Detect a shell family from a `$SHELL` value (e.g. `/bin/zsh`).
    pub fn detect(shell_env: &str) -> Option<ShellKind> {
        let basename = shell_env.rsplit('/').next().unwrap_or(shell_env);
        match basename {
            "zsh" => Some(ShellKind::Zsh),
            "bash" => Some(ShellKind::Bash),
            _ => None,
        }
    }

    /// The rc file this shell reads on startup, relative to `$HOME`.
    fn rc_file_name(self) -> &'static str {
        match self {
            ShellKind::Bash => ".bashrc",
            ShellKind::Zsh => ".zshrc",
        }
    }

    /// The line a new or migrated install sources.
    fn source_line(self) -> &'static str {
        match self {
            ShellKind::Bash => "source \"$HOME/.config/playbook/shell/bash/cc.sh\"",
            ShellKind::Zsh => "source \"$HOME/.config/playbook/shell/zsh/cc.zsh\"",
        }
    }

    /// The exact line a pre-ADR-0012 install sourced.
    fn legacy_source_line(self) -> &'static str {
        match self {
            ShellKind::Bash => "source \"$HOME/.claude/shell/bash/cc.sh\"",
            ShellKind::Zsh => "source \"$HOME/.claude/shell/zsh/cc.zsh\"",
        }
    }
}

/// What `rewire_rc_file` did, for a caller to report to the user.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ShimOutcome {
    /// The rc file that was checked (and possibly changed).
    pub rc_file: PathBuf,
    /// Whether this call changed the rc file, appended or replaced in place.
    pub appended: bool,
}

/// Copy the launcher entry points and shared modules into
/// `playbook_root_from(home)/shell`, skipping `*.test.sh`.
pub fn copy_launcher_runtime(self_root: &Path, home: &Path) -> io::Result<bool> {
    let dst_shell = playbook_root_from(home).join("shell");
    let mut changed = false;
    changed |= copy_file_into(
        &self_root.join("shell/bash/cc.sh"),
        &dst_shell.join("bash/cc.sh"),
    )?;
    changed |= copy_file_into(
        &self_root.join("shell/zsh/cc.zsh"),
        &dst_shell.join("zsh/cc.zsh"),
    )?;

    let shared_src = self_root.join("shell/shared");
    let shared_dst = dst_shell.join("shared");
    fs::create_dir_all(&shared_dst)?;
    for entry in fs::read_dir(&shared_src)? {
        let entry = entry?;
        if !entry.file_type()?.is_file() {
            continue;
        }
        let name = entry.file_name();
        if name.to_string_lossy().ends_with(".test.sh") {
            continue;
        }
        changed |= copy_file_into(&entry.path(), &shared_dst.join(&name))?;
    }
    Ok(changed)
}

/// Copy one file. Returns whether the destination's bytes changed.
fn copy_file_into(src: &Path, dst: &Path) -> io::Result<bool> {
    if let Some(parent) = dst.parent() {
        fs::create_dir_all(parent)?;
    }
    let before = fs::read(dst).ok();
    fs::copy(src, dst)?;
    let after = fs::read(dst)?;
    Ok(before.as_deref() != Some(after.as_slice()))
}

/// Make sure `home`'s rc file for `shell_kind` sources the current launcher
/// location, replacing an exact legacy line in place rather than appending.
pub fn rewire_rc_file(home: &Path, shell_kind: ShellKind) -> io::Result<ShimOutcome> {
    let rc_file = home.join(shell_kind.rc_file_name());
    let existing = fs::read_to_string(&rc_file).unwrap_or_default();

    if existing
        .lines()
        .any(|l| l.trim() == shell_kind.source_line())
    {
        return Ok(ShimOutcome {
            rc_file,
            appended: false,
        });
    }

    if let Some(replaced) =
        replace_exact_line(&existing, shell_kind.legacy_source_line(), shell_kind)
    {
        atomic_write_rc_file(&rc_file, &replaced)?;
        return Ok(ShimOutcome {
            rc_file,
            appended: true,
        });
    }

    append_source_line(&rc_file, shell_kind)?;
    Ok(ShimOutcome {
        rc_file,
        appended: true,
    })
}

/// Replace the one line trimming exactly to `legacy`. `None` if absent.
fn replace_exact_line(content: &str, legacy: &str, shell_kind: ShellKind) -> Option<String> {
    let mut lines: Vec<&str> = content.lines().collect();
    let idx = lines.iter().position(|l| l.trim() == legacy)?;
    lines[idx] = shell_kind.source_line();
    let mut out = lines.join("\n");
    out.push('\n');
    Some(out)
}

/// Overwrite `rc_file` via a sibling temp file plus rename.
fn atomic_write_rc_file(rc_file: &Path, content: &str) -> io::Result<()> {
    let dir = rc_file
        .parent()
        .filter(|p| !p.as_os_str().is_empty())
        .unwrap_or_else(|| Path::new("."));
    fs::create_dir_all(dir)?;
    let tmp_path = dir.join(format!(".rc-rewire-{}.tmp", std::process::id()));
    if let Err(err) = fs::write(&tmp_path, content) {
        let _ = fs::remove_file(&tmp_path);
        return Err(err);
    }
    if let Err(err) = fs::rename(&tmp_path, rc_file) {
        let _ = fs::remove_file(&tmp_path);
        return Err(err);
    }
    Ok(())
}

/// Append `shell_kind`'s source line to `rc_file`, creating it if needed.
fn append_source_line(rc_file: &Path, shell_kind: ShellKind) -> io::Result<()> {
    if let Some(parent) = rc_file.parent() {
        if !parent.as_os_str().is_empty() {
            fs::create_dir_all(parent)?;
        }
    }
    let mut file = fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(rc_file)?;
    write!(
        file,
        "\n# playbook launchers (cc/ccd)\n{}\n",
        shell_kind.source_line()
    )
}
