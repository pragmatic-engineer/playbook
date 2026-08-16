// SPDX-FileCopyrightText: 2026 Igor Santos
// SPDX-License-Identifier: MIT

//! Installs the `cc`/`ccd` shell launcher and wires the user's rc file to
//! source it, porting `shell/setup-local.sh:180-269`'s `--aliases` block.
//!
//! Deliberate divergence: `setup-local.sh` copies every file directly under
//! `shell/` except `*.test.sh` (its own comment at :172-178 calls this
//! "every file/dir in shell/"), which also carries installer-only scripts
//! this binary now supersedes: `ensure-deps.sh`, `merge-settings.py`,
//! `gen-shared-settings.py`, `check-shared-settings.py`, and
//! `setup-local.sh` itself. Those aren't part of what the launcher sources
//! at runtime (only `shell/bash/cc.sh`, `shell/zsh/cc.zsh` and
//! `shell/shared/*.sh` are), so copying them into a user's `~/.claude/shell`
//! would be dead weight with no runtime purpose, not behavioural parity.
//! `copy_launcher_runtime` below copies exactly the launcher runtime and
//! nothing else.
//!
//! Divergence also on the pre-reorganisation migration at
//! `setup-local.sh:232-262`: that block rewrites an OLD `shell/cc.zsh` /
//! `shell/cc.sh` source line (from before the bash/zsh split) to the current
//! `shell/zsh/cc.zsh` / `shell/bash/cc.sh` form. No rc file `playbook init`
//! ever writes can contain that old form, since this binary never wrote it
//! in the first place, so that migration path is unreachable here and is
//! not ported. A `playbook init` run against an rc file with the genuinely
//! old line is out of scope: that machine has never run this binary before.

use std::fs;
use std::io;
use std::io::Write;
use std::path::{Path, PathBuf};

/// Which shell family the rc-file wiring targets. Mirrors
/// `setup-local.sh:208-230`'s `case "$_SHELL_BIN" in zsh|bash|*)`, minus the
/// `*` (unrecognised shell) arm: deciding what to do when `$SHELL` names
/// neither is `init`'s call (WU-8/WU-11's wiring), not this module's, the
/// same separation `init::merge` draws between computing a result and
/// deciding what a caller does with it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ShellKind {
    Bash,
    Zsh,
}

impl ShellKind {
    /// Detect a shell family from a `$SHELL` value (e.g. `/bin/zsh`), taking
    /// the string as a parameter rather than reading the environment
    /// directly so a caller (and every test here) controls it without
    /// mutating real process state. Mirrors
    /// `setup-local.sh:208`'s `basename "${SHELL:-}"` plus its `case`.
    pub fn detect(shell_env: &str) -> Option<ShellKind> {
        let basename = shell_env.rsplit('/').next().unwrap_or(shell_env);
        match basename {
            "zsh" => Some(ShellKind::Zsh),
            "bash" => Some(ShellKind::Bash),
            _ => None,
        }
    }

    /// The rc file this shell reads on interactive startup, relative to
    /// `$HOME`. Matches `setup-local.sh:211` (`$HOME/.zshrc`) and `:218`
    /// (`$HOME/.bashrc`).
    fn rc_file_name(self) -> &'static str {
        match self {
            ShellKind::Bash => ".bashrc",
            ShellKind::Zsh => ".zshrc",
        }
    }

    /// The literal line appended to the rc file. Kept as a literal `$HOME`
    /// token, not an already-expanded path, because it is evaluated by the
    /// shell every time the rc file is sourced, not once at install time.
    /// Matches `setup-local.sh:213` and `:220`.
    fn source_line(self) -> &'static str {
        match self {
            ShellKind::Bash => "source \"$HOME/.claude/shell/bash/cc.sh\"",
            ShellKind::Zsh => "source \"$HOME/.claude/shell/zsh/cc.zsh\"",
        }
    }

    /// The substring an idempotency check greps the rc file for before
    /// appending, matching `setup-local.sh:214` and `:221`
    /// (`GREP_PAT`). A substring of `source_line`, so this stays correct by
    /// construction if that literal ever changes.
    fn grep_pattern(self) -> &'static str {
        match self {
            ShellKind::Bash => "shell/bash/cc.sh",
            ShellKind::Zsh => "shell/zsh/cc.zsh",
        }
    }
}

/// What `install_shim` did, for a caller to report to the user.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ShimOutcome {
    /// The rc file that was checked (and possibly appended to).
    pub rc_file: PathBuf,
    /// Whether this call actually appended the source line. `false` means
    /// the rc file already had it: the idempotent, common case on a re-run.
    pub appended: bool,
}

/// Install the launcher runtime under `claude_home/shell` and make sure
/// `home`'s rc file for `shell_kind` sources it, appending the source line
/// only if it is not already present. Ports the `--aliases` block of
/// `setup-local.sh:180-269`; see the module doc comment for the two
/// deliberate divergences from it.
pub fn install_shim(
    self_root: &Path,
    claude_home: &Path,
    home: &Path,
    shell_kind: ShellKind,
) -> io::Result<ShimOutcome> {
    copy_launcher_runtime(self_root, claude_home)?;
    let rc_file = home.join(shell_kind.rc_file_name());
    let appended = append_source_line_idempotently(&rc_file, shell_kind)?;
    Ok(ShimOutcome { rc_file, appended })
}

/// Copy the launcher entry points and the shared modules they source into
/// `claude_home/shell`, skipping `*.test.sh` the way
/// `setup-local.sh:187-189` does. See the module doc comment for why this
/// is narrower than "every file under `shell/`".
fn copy_launcher_runtime(self_root: &Path, claude_home: &Path) -> io::Result<()> {
    let dst_shell = claude_home.join("shell");
    copy_file_into(
        &self_root.join("shell/bash/cc.sh"),
        &dst_shell.join("bash/cc.sh"),
    )?;
    copy_file_into(
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
        copy_file_into(&entry.path(), &shared_dst.join(&name))?;
    }
    Ok(())
}

/// Copy one file, creating its destination's parent directory first. A thin
/// helper so `copy_launcher_runtime` reads as a list of what gets copied,
/// not how.
fn copy_file_into(src: &Path, dst: &Path) -> io::Result<()> {
    if let Some(parent) = dst.parent() {
        fs::create_dir_all(parent)?;
    }
    fs::copy(src, dst)?;
    Ok(())
}

/// Append `shell_kind`'s source line to `rc_file`, creating the file (and
/// any missing parent directory) if it does not exist yet, unless the file
/// already contains the grep pattern. Returns whether it actually appended.
/// Matches `setup-local.sh:263-268`: a missing rc file makes
/// `grep -qF ... "$RC_FILE"` fail, falling through to the `printf ... >>`
/// branch, which creates the file via append-mode redirection; the same
/// happens here via `OpenOptions::create(true)`.
fn append_source_line_idempotently(rc_file: &Path, shell_kind: ShellKind) -> io::Result<bool> {
    let existing = fs::read_to_string(rc_file).unwrap_or_default();
    if existing.contains(shell_kind.grep_pattern()) {
        return Ok(false);
    }
    if let Some(parent) = rc_file.parent() {
        if !parent.as_os_str().is_empty() {
            fs::create_dir_all(parent)?;
        }
    }
    let mut file = fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(rc_file)?;
    // Matches setup-local.sh:266's
    // `printf '\n# playbook launchers (cc/ccd)\n%s\n' "$SOURCE_LINE"` byte
    // for byte: a leading blank line, the comment, the source line, then a
    // single trailing newline.
    write!(
        file,
        "\n# playbook launchers (cc/ccd)\n{}\n",
        shell_kind.source_line()
    )?;
    Ok(true)
}
