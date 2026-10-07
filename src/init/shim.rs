// SPDX-FileCopyrightText: 2026 Igor Santos
// SPDX-License-Identifier: MIT

//! Wires the rc file to load the `cc`/`ccd` launcher with
//! `eval "$(playbook shell-init)"`, and upgrades the legacy `source` lines.

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

    /// The launcher paths earlier installs sourced, relative to `$HOME`:
    /// the pre-ADR-0012 `.claude/shell` paths, then the `.config/playbook`
    /// ones, each with and without the bash/zsh/shared split.
    fn legacy_launcher_paths(self) -> [&'static str; 4] {
        match self {
            ShellKind::Bash => [
                ".claude/shell/bash/cc.sh",
                ".claude/shell/cc.sh",
                ".config/playbook/shell/bash/cc.sh",
                ".config/playbook/shell/cc.sh",
            ],
            ShellKind::Zsh => [
                ".claude/shell/zsh/cc.zsh",
                ".claude/shell/cc.zsh",
                ".config/playbook/shell/zsh/cc.zsh",
                ".config/playbook/shell/cc.zsh",
            ],
        }
    }

    const ALL: [ShellKind; 2] = [ShellKind::Bash, ShellKind::Zsh];
}

/// The line a new or migrated install runs, guarded so a shell started
/// without `playbook` on PATH still comes up.
pub const SOURCE_LINE: &str =
    "command -v playbook >/dev/null 2>&1 && eval \"$(playbook shell-init)\"";

/// Comment line that opens the managed block; uninstall keys on it too.
const BLOCK_COMMENT: &str = "# playbook launchers (cc/ccd)";

/// What `rewire_rc_file` did, for a caller to report to the user.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ShimOutcome {
    /// The rc file that was checked (and possibly changed).
    pub rc_file: PathBuf,
    /// Whether this call changed the rc file, appended or replaced in place.
    pub appended: bool,
    /// The rc file needed a change but lacks the owner write bit, so it was
    /// left untouched.
    pub unwritable: bool,
}

/// Place `hooks/lib/config-hash.sh`, which `cc::config_drift` runs from the
/// config dir outside any plugin context. A partial checkout without it is skipped.
pub fn place_config_hash(self_root: &Path, home: &Path) -> io::Result<bool> {
    let src = self_root.join("hooks/lib/config-hash.sh");
    if !src.is_file() {
        return Ok(false);
    }
    let dst = playbook_root_from(home).join("hooks/lib/config-hash.sh");
    if let Some(parent) = dst.parent() {
        fs::create_dir_all(parent)?;
    }
    let before = fs::read(&dst).ok();
    fs::copy(&src, &dst)?;
    Ok(before.as_deref() != Some(fs::read(&dst)?.as_slice()))
}

/// Make sure `home`'s rc file for `shell_kind` sources the current launcher
/// location exactly once: every legacy source line is removed, the first one
/// replaced in place when the current line is missing, else the current line
/// is appended. Works on bytes so a non-UTF-8 rc file is never mistaken for
/// an empty one.
pub fn rewire_rc_file(home: &Path, shell_kind: ShellKind) -> io::Result<ShimOutcome> {
    let rc_file = home.join(shell_kind.rc_file_name());
    let existing = match fs::read(&rc_file) {
        Ok(bytes) => Some(bytes),
        Err(err) if err.kind() == io::ErrorKind::NotFound => None,
        Err(err) => return Err(err),
    };
    let unchanged = ShimOutcome {
        rc_file: rc_file.clone(),
        appended: false,
        unwritable: false,
    };

    let Some(existing) = existing else {
        append_source_line(&rc_file)?;
        return Ok(ShimOutcome {
            appended: true,
            ..unchanged
        });
    };

    let rewritten = rewrite_legacy_lines(&existing, shell_kind);
    if rewritten.is_none() && has_line(&existing, SOURCE_LINE) {
        return Ok(unchanged);
    }
    if !owner_can_write(&rc_file) {
        return Ok(ShimOutcome {
            unwritable: true,
            ..unchanged
        });
    }
    match rewritten {
        Some(content) => atomic_write_rc_file(&rc_file, &content)?,
        None => append_source_line(&rc_file)?,
    }
    Ok(ShimOutcome {
        appended: true,
        ..unchanged
    })
}

/// Rewrite the legacy `source` launcher lines in every shell's rc file under
/// `home` to the `shell-init` line. Files without a legacy line are not
/// touched. Returns the rc files changed.
pub fn upgrade_legacy_rc_files(home: &Path) -> io::Result<Vec<PathBuf>> {
    let mut changed = Vec::new();
    for kind in ShellKind::ALL {
        let rc_file = home.join(kind.rc_file_name());
        let Ok(existing) = fs::read(&rc_file) else {
            continue;
        };
        let Some(content) = rewrite_legacy_lines(&existing, kind) else {
            continue;
        };
        if !owner_can_write(&rc_file) {
            continue;
        }
        atomic_write_rc_file(&rc_file, &content)?;
        changed.push(rc_file);
    }
    Ok(changed)
}

/// Whether some line of `content` trims to exactly `wanted`.
fn has_line(content: &[u8], wanted: &str) -> bool {
    content
        .split(|&b| b == b'\n')
        .any(|line| line.trim_ascii() == wanted.as_bytes())
}

/// Drop every legacy source line from `content`. The first one becomes the
/// current line when no current line exists yet. `None` if there was no
/// legacy line to touch.
fn rewrite_legacy_lines(content: &[u8], shell_kind: ShellKind) -> Option<Vec<u8>> {
    let mut current_present = has_line(content, SOURCE_LINE);
    let mut out = Vec::with_capacity(content.len());
    let mut touched = false;
    for line in content.split_inclusive(|&b| b == b'\n') {
        if !is_legacy_source_line(line, shell_kind) {
            out.extend_from_slice(line);
            continue;
        }
        touched = true;
        if !current_present {
            out.extend_from_slice(SOURCE_LINE.as_bytes());
            out.push(b'\n');
            current_present = true;
        }
    }
    touched.then_some(out)
}

/// A non-comment line that sources (`source` or `.`) a legacy launcher path,
/// written with `$HOME`, `${HOME}` or `~`, quoted or not.
fn is_legacy_source_line(line: &[u8], shell_kind: ShellKind) -> bool {
    let line = line.trim_ascii();
    let Some(args) = line
        .strip_prefix(b"source")
        .or_else(|| line.strip_prefix(b"."))
    else {
        return false;
    };
    if !args.first().is_some_and(u8::is_ascii_whitespace) {
        return false;
    }
    shell_kind.legacy_launcher_paths().iter().any(|path| {
        ["$HOME", "${HOME}", "~"].iter().any(|home| {
            let wanted = format!("{home}/{path}");
            names_path(args, wanted.as_bytes())
        })
    })
}

/// Whether `args` holds `path` as a whole argument: the byte before is
/// whitespace or a quote, the byte after is whitespace, a quote or the end.
fn names_path(args: &[u8], path: &[u8]) -> bool {
    args.windows(path.len())
        .enumerate()
        .filter(|(_, window)| *window == path)
        .any(|(at, _)| {
            let before = args[at - 1];
            let after = args.get(at + path.len()).copied();
            (before.is_ascii_whitespace() || before == b'"' || before == b'\'')
                && after.is_none_or(|b| b.is_ascii_whitespace() || b == b'"' || b == b'\'')
        })
}

#[cfg(unix)]
fn owner_can_write(path: &Path) -> bool {
    use std::os::unix::fs::PermissionsExt;
    fs::metadata(path).is_ok_and(|m| m.permissions().mode() & 0o200 != 0)
}

#[cfg(not(unix))]
fn owner_can_write(path: &Path) -> bool {
    fs::metadata(path).is_ok_and(|m| !m.permissions().readonly())
}

/// Overwrite `rc_file` via a temp file beside its real target plus rename,
/// preserving permission bits. A symlinked rc (stow, chezmoi) stays a
/// symlink: only the file it points at is replaced.
fn atomic_write_rc_file(rc_file: &Path, content: &[u8]) -> io::Result<()> {
    let target = fs::canonicalize(rc_file)?;
    let dir = target
        .parent()
        .filter(|p| !p.as_os_str().is_empty())
        .unwrap_or_else(|| Path::new("."));
    let original_permissions = fs::metadata(&target).ok().map(|m| m.permissions());
    let tmp_path = dir.join(format!(".rc-rewire-{}.tmp", std::process::id()));
    if let Err(err) = fs::write(&tmp_path, content) {
        let _ = fs::remove_file(&tmp_path);
        return Err(err);
    }
    if let Some(permissions) = original_permissions {
        if let Err(err) = fs::set_permissions(&tmp_path, permissions) {
            let _ = fs::remove_file(&tmp_path);
            return Err(err);
        }
    }
    if let Err(err) = fs::rename(&tmp_path, &target) {
        let _ = fs::remove_file(&tmp_path);
        return Err(err);
    }
    Ok(())
}

/// Append the `shell-init` line to `rc_file`, creating it if needed.
fn append_source_line(rc_file: &Path) -> io::Result<()> {
    if let Some(parent) = rc_file.parent() {
        if !parent.as_os_str().is_empty() {
            fs::create_dir_all(parent)?;
        }
    }
    let mut file = fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(rc_file)?;
    write!(file, "\n{BLOCK_COMMENT}\n{SOURCE_LINE}\n")
}
