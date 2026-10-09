// SPDX-FileCopyrightText: 2026 Igor Santos
// SPDX-License-Identifier: Apache-2.0

//! Wires the rc file to load the `ccc`/`ccd` launcher with
//! `eval "$(playbook shell-init)"`, and upgrades the legacy `source` lines.

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

/// Comment line that opens the managed block; uninstall keys on it too, and
/// still strips the older `(cc/ccd)` wording.
const BLOCK_COMMENT: &str = "# playbook launchers (ccc/ccd)";

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
    if rewritten.is_none() && has_current_line(&existing) {
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

/// One rc file `strip_rc_files` changed, or would change on a dry run.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RcStrip {
    pub rc_file: PathBuf,
    /// The copy taken before the write; `None` on a dry run or when unwritable.
    pub backup: Option<PathBuf>,
    /// The file needs a change but lacks the owner write bit, so it was left alone.
    pub unwritable: bool,
    /// Why the backup or the write failed; the file was left as it was.
    pub error: Option<String>,
}

/// Remove the managed launcher block (and legacy `source` lines) from every
/// shell's rc file under `home`, and with `binary_path` also the `PATH` block
/// the installer appends. Anything else in the file is kept byte for byte.
/// A changed file is first copied to `<rc>.bak-<stamp>`.
pub fn strip_rc_files(home: &Path, stamp: u64, binary_path: bool, dry_run: bool) -> Vec<RcStrip> {
    let mut changed = Vec::new();
    for kind in ShellKind::ALL {
        let rc_file = home.join(kind.rc_file_name());
        let Ok(existing) = fs::read(&rc_file) else {
            continue;
        };
        let mut content = strip_launcher(&existing, kind);
        if binary_path {
            let base = content.as_deref().unwrap_or(&existing);
            content = strip_binary_path(base).or(content);
        }
        let Some(content) = content else { continue };
        let mut entry = RcStrip {
            rc_file: rc_file.clone(),
            backup: None,
            unwritable: false,
            error: None,
        };
        if !owner_can_write(&rc_file) {
            entry.unwritable = true;
        } else if !dry_run {
            let name = rc_file.file_name().unwrap_or_default().to_string_lossy();
            let backup = rc_file.with_file_name(format!("{name}.bak-{stamp}"));
            match fs::copy(&rc_file, &backup).and_then(|_| atomic_write_rc_file(&rc_file, &content))
            {
                Ok(()) => entry.backup = Some(backup),
                Err(err) => entry.error = Some(err.to_string()),
            }
        }
        changed.push(entry);
    }
    changed
}

/// `strip_binary_path` over each of `files` that exists. A changed file is
/// first copied to `<file>.bak-<stamp>`. Used for the files outside the two
/// rc files `strip_rc_files` walks.
pub fn strip_binary_path_files(files: &[PathBuf], stamp: u64, dry_run: bool) -> Vec<RcStrip> {
    let mut changed = Vec::new();
    for rc_file in files {
        let Ok(existing) = fs::read(rc_file) else {
            continue;
        };
        let Some(content) = strip_binary_path(&existing) else {
            continue;
        };
        let mut entry = RcStrip {
            rc_file: rc_file.clone(),
            backup: None,
            unwritable: false,
            error: None,
        };
        if !owner_can_write(rc_file) {
            entry.unwritable = true;
        } else if !dry_run {
            let name = rc_file.file_name().unwrap_or_default().to_string_lossy();
            let backup = rc_file.with_file_name(format!("{name}.bak-{stamp}"));
            match fs::copy(rc_file, &backup).and_then(|_| atomic_write_rc_file(rc_file, &content)) {
                Ok(()) => entry.backup = Some(backup),
                Err(err) => entry.error = Some(err.to_string()),
            }
        }
        changed.push(entry);
    }
    changed
}

/// `content` without the launcher line(s) and the comment right above each.
/// `None` when there was nothing to remove.
fn strip_launcher(content: &[u8], shell_kind: ShellKind) -> Option<Vec<u8>> {
    strip_lines(
        content,
        |lines, i| {
            let line = lines[i];
            let trimmed = line.trim_ascii();
            let current =
                trimmed == SOURCE_LINE.as_bytes() || trimmed == b"eval \"$(playbook shell-init)\"";
            usize::from(current || is_legacy_source_line(line, shell_kind))
        },
        |prev| {
            let prev = prev.trim_ascii();
            prev.starts_with(b"#")
                && (prev.windows(18).any(|w| w == b"launchers (cc/ccd)")
                    || prev.windows(19).any(|w| w == b"launchers (ccc/ccd)"))
        },
    )
}

/// `content` without the `# playbook binary` marker and the `export PATH=`
/// line the installer writes right after it. Anchored on the marker, so a
/// user's own `PATH` edits are never touched.
fn strip_binary_path(content: &[u8]) -> Option<Vec<u8>> {
    strip_lines(
        content,
        |lines, i| {
            let marker = lines[i].trim_ascii() == b"# playbook binary";
            let export = lines
                .get(i + 1)
                .is_some_and(|l| l.trim_ascii().starts_with(b"export PATH="));
            if marker && export {
                2
            } else {
                0
            }
        },
        |_| false,
    )
}

/// Remove the spans `remove_at` reports (a line count at line `i`, 0 for
/// none), with the line above a span when `is_marker` accepts it, and the
/// blank line the gap would otherwise double or leave dangling at the end.
fn strip_lines(
    content: &[u8],
    remove_at: impl Fn(&[&[u8]], usize) -> usize,
    is_marker: impl Fn(&[u8]) -> bool,
) -> Option<Vec<u8>> {
    let lines: Vec<&[u8]> = content.split_inclusive(|&b| b == b'\n').collect();
    let is_blank = |l: &[u8]| l.trim_ascii().is_empty();
    let mut out: Vec<&[u8]> = Vec::new();
    let mut touched = false;
    let mut just_removed = false;
    let mut i = 0;
    while i < lines.len() {
        let span = remove_at(&lines, i);
        if span > 0 {
            touched = true;
            just_removed = true;
            if out.last().is_some_and(|prev| is_marker(prev)) {
                out.pop();
            }
            i += span;
            continue;
        }
        let line = lines[i];
        i += 1;
        if just_removed && is_blank(line) && out.last().is_none_or(|l| is_blank(l)) {
            just_removed = false;
            continue;
        }
        just_removed = false;
        out.push(line);
    }
    if !touched {
        return None;
    }
    if just_removed && out.last().is_some_and(|l| is_blank(l)) {
        out.pop();
    }
    Some(out.concat())
}

/// Whether `line` is the managed block's comment, in its current or older wording.
fn is_block_comment(line: &[u8]) -> bool {
    let line = line.trim_ascii();
    line == BLOCK_COMMENT.as_bytes() || line == b"# playbook launchers (cc/ccd)"
}

/// Whether some line of `content` trims to exactly `wanted`.
fn has_line(content: &[u8], wanted: &str) -> bool {
    content
        .split(|&b| b == b'\n')
        .any(|line| line.trim_ascii() == wanted.as_bytes())
}

/// Drop every legacy source line (and the managed comment right above it)
/// from `content`, then append the current block at the end unless it is
/// already there. At the end, so the line runs after any `PATH` export the
/// installer appended below the old `source` line. `None` if there was no
/// legacy line to touch.
fn rewrite_legacy_lines(content: &[u8], shell_kind: ShellKind) -> Option<Vec<u8>> {
    let mut lines: Vec<&[u8]> = Vec::new();
    let mut touched = false;
    for line in content.split_inclusive(|&b| b == b'\n') {
        if !is_legacy_source_line(line, shell_kind) {
            lines.push(line);
            continue;
        }
        touched = true;
        if lines.last().is_some_and(|prev| is_block_comment(prev)) {
            lines.pop();
        }
    }
    if !touched {
        return None;
    }
    let mut out: Vec<u8> = lines.concat();
    if !has_current_line(&out) {
        while out.ends_with(b"\n\n") {
            out.pop();
        }
        if !out.is_empty() && !out.ends_with(b"\n") {
            out.push(b'\n');
        }
        if !out.is_empty() {
            out.push(b'\n');
        }
        out.extend_from_slice(format!("{BLOCK_COMMENT}\n{SOURCE_LINE}\n").as_bytes());
    }
    Some(out)
}

/// The guarded line, or the bare `eval` a user may have written by hand.
fn has_current_line(content: &[u8]) -> bool {
    has_line(content, SOURCE_LINE) || has_line(content, "eval \"$(playbook shell-init)\"")
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
pub(crate) fn owner_can_write(path: &Path) -> bool {
    use std::os::unix::fs::PermissionsExt;
    fs::metadata(path).is_ok_and(|m| m.permissions().mode() & 0o200 != 0)
}

#[cfg(not(unix))]
pub(crate) fn owner_can_write(path: &Path) -> bool {
    fs::metadata(path).is_ok_and(|m| !m.permissions().readonly())
}

/// Overwrite `rc_file` via a temp file beside its real target plus rename,
/// preserving permission bits. A symlinked rc (stow, chezmoi) stays a
/// symlink: only the file it points at is replaced.
fn atomic_write_rc_file(rc_file: &Path, content: &[u8]) -> io::Result<()> {
    let target = fs::canonicalize(rc_file)?;
    crate::common::atomic::write_atomic(&target, content)
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
