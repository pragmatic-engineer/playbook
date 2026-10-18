// SPDX-FileCopyrightText: 2026 Igor Santos
// SPDX-License-Identifier: Apache-2.0

//! Finds and removes the rc-file lines older playbook versions wrote for the
//! retired shell launcher (ADR 0044), and the installer's `PATH` block.

use std::fs;
use std::io;
use std::path::{Path, PathBuf};

/// Which shell family the rc-file wiring targets.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ShellKind {
    Bash,
    Zsh,
}

impl ShellKind {
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

/// The guarded line setup wrote before ADR 0044.
pub const SOURCE_LINE: &str =
    "command -v playbook >/dev/null 2>&1 && eval \"$(playbook shell-init)\"";

/// The comment setup wrote above `SOURCE_LINE`, in its current and older wording.
const BLOCK_COMMENTS: [&str; 2] = [
    "# playbook launchers (ccc/ccd)",
    "# playbook launchers (cc/ccd)",
];

/// The `eval` line without the guard, as a user may write it by hand. The
/// `shell-init` command is gone, so this line now prints a notice on every
/// shell start, and migration 0018 removes it too.
const BARE_EVAL: &str = "eval \"$(playbook shell-init)\"";

/// An rc file that holds a retired launcher line, and whether setup can
/// rewrite it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RetiredRc {
    pub rc_file: PathBuf,
    pub writable: bool,
}

/// One file a strip changed, or would change on a dry run.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RcStrip {
    pub rc_file: PathBuf,
    /// The copy taken before the write; `None` on a dry run, when unwritable
    /// or on an error.
    pub backup: Option<PathBuf>,
    /// The file needs a change but lacks the owner write bit, so it was left alone.
    pub unwritable: bool,
    /// Why the backup or the write failed; the file was left as it was.
    pub error: Option<String>,
}

/// Remove the launcher lines, the bare `eval` included, from every shell's
/// rc file under `home`, and with `binary_path` also the `PATH` block the
/// installer appends. Anything else in the file is kept byte for byte. A
/// changed file is first copied to `<rc>.bak-<stamp>`.
pub fn strip_rc_files(home: &Path, stamp: u64, binary_path: bool, dry_run: bool) -> Vec<RcStrip> {
    rc_rewrites(home, |existing, kind| {
        let content = strip_launcher(existing, kind);
        if !binary_path {
            return content;
        }
        strip_binary_path(content.as_deref().unwrap_or(existing)).or(content)
    })
    .into_iter()
    .map(|(rc_file, content)| apply(rc_file, &content, stamp, dry_run))
    .collect()
}

/// Each rc file under `home` that holds a line migration 0018 removes, and
/// whether setup can rewrite it. Reads only.
pub fn retired_rc_entries(home: &Path) -> Vec<RetiredRc> {
    retired_rewrites(home)
        .into_iter()
        .map(|(rc_file, _)| RetiredRc {
            writable: can_rewrite(&rc_file),
            rc_file,
        })
        .collect()
}

/// The writable rc files under `home` that hold a line migration 0018
/// removes. Reads only.
pub fn retired_rc_files(home: &Path) -> Vec<PathBuf> {
    retired_rc_entries(home)
        .into_iter()
        .filter(|r| r.writable)
        .map(|r| r.rc_file)
        .collect()
}

/// Remove the retired launcher lines from each writable rc file under
/// `home`, after a copy to `<rc>.bak-<stamp>`, the bare hand-written `eval`
/// included. Files setup cannot rewrite are left out.
pub fn strip_retired_rc_lines(home: &Path, stamp: u64) -> Vec<RcStrip> {
    retired_rewrites(home)
        .into_iter()
        .map(|(rc_file, content)| apply(rc_file, &content, stamp, false))
        .filter(|entry| !entry.unwritable)
        .collect()
}

fn retired_rewrites(home: &Path) -> Vec<(PathBuf, Vec<u8>)> {
    rc_rewrites(home, strip_launcher)
}

/// Each shell's rc file under `home` that `rewrite` changes, with its new content.
fn rc_rewrites(
    home: &Path,
    rewrite: impl Fn(&[u8], ShellKind) -> Option<Vec<u8>>,
) -> Vec<(PathBuf, Vec<u8>)> {
    ShellKind::ALL
        .into_iter()
        .filter_map(|kind| {
            let rc_file = home.join(kind.rc_file_name());
            let existing = fs::read(&rc_file).ok()?;
            Some((rc_file, rewrite(&existing, kind)?))
        })
        .collect()
}

/// `strip_binary_path` over each of `files` that exists. A changed file is
/// first copied to `<file>.bak-<stamp>`. Used for the files outside the two
/// rc files `strip_rc_files` walks.
pub fn strip_binary_path_files(files: &[PathBuf], stamp: u64, dry_run: bool) -> Vec<RcStrip> {
    files
        .iter()
        .filter_map(|rc_file| {
            let content = strip_binary_path(&fs::read(rc_file).ok()?)?;
            Some(apply(rc_file.clone(), &content, stamp, dry_run))
        })
        .collect()
}

/// Write `content` to `rc_file` after a copy to `<rc>.bak-<stamp>`, unless
/// this is a dry run or the file lacks the owner write bit.
fn apply(rc_file: PathBuf, content: &[u8], stamp: u64, dry_run: bool) -> RcStrip {
    let mut entry = RcStrip {
        rc_file,
        backup: None,
        unwritable: false,
        error: None,
    };
    if !can_rewrite(&entry.rc_file) {
        entry.unwritable = true;
    } else if !dry_run {
        let rc_file = &entry.rc_file;
        let name = rc_file.file_name().unwrap_or_default().to_string_lossy();
        let backup = rc_file.with_file_name(format!("{name}.bak-{stamp}"));
        match fs::copy(rc_file, &backup).and_then(|_| atomic_write_rc_file(rc_file, content)) {
            Ok(()) => entry.backup = Some(backup),
            Err(err) => entry.error = Some(err.to_string()),
        }
    }
    entry
}

/// `content` without the launcher lines, a lone marker comment, and a
/// launchers marker comment right above a removed line. `None` when there
/// was nothing to remove.
fn strip_launcher(content: &[u8], shell_kind: ShellKind) -> Option<Vec<u8>> {
    strip_lines(
        content,
        |lines, i| {
            let line = lines[i];
            let trimmed = line.trim_ascii();
            let ours = trimmed == SOURCE_LINE.as_bytes()
                || trimmed == BARE_EVAL.as_bytes()
                || is_block_comment(line);
            usize::from(ours || is_legacy_source_line(line, shell_kind))
        },
        |prev| {
            let prev = prev.trim_ascii();
            prev.starts_with(b"#")
                && BLOCK_COMMENTS.iter().any(|c| {
                    let tail = c.trim_start_matches("# playbook ").as_bytes();
                    prev.windows(tail.len()).any(|w| w == tail)
                })
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

/// Whether `line` is the marker comment, in its current or older wording.
fn is_block_comment(line: &[u8]) -> bool {
    let line = line.trim_ascii();
    BLOCK_COMMENTS.iter().any(|c| line == c.as_bytes())
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

/// Whether this process can replace `path`: write the file, and create the
/// temp file and rename in the directory of its real target.
#[cfg(unix)]
pub(crate) fn can_rewrite(path: &Path) -> bool {
    use std::os::unix::ffi::OsStrExt;
    let writable = |p: &Path| {
        std::ffi::CString::new(p.as_os_str().as_bytes())
            // SAFETY: access(2) only reads the NUL-terminated path.
            .is_ok_and(|c| unsafe { libc::access(c.as_ptr(), libc::W_OK) } == 0)
    };
    let Ok(target) = fs::canonicalize(path) else {
        return false;
    };
    owner_can_write(&target) && writable(&target) && target.parent().is_some_and(writable)
}

#[cfg(not(unix))]
pub(crate) fn can_rewrite(path: &Path) -> bool {
    owner_can_write(path)
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
