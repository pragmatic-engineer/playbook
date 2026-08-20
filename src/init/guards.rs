// SPDX-FileCopyrightText: 2026 Igor Santos
// SPDX-License-Identifier: MIT

//! Places the four bash safety guards at the `~/.claude/hooks/<name>.sh`
//! paths `init::wire` points `settings.json` at, so the component that names
//! a path is the component that puts the file there.
//!
//! Before this module, nothing in `playbook init` placed them. The only
//! thing that ever did was `install.sh`'s whole-tree copy into `~/.claude`,
//! a side effect of copying the repo wholesale, and WU-11 deletes that copy.
//! `shell/setup-local.sh` copies only three of the four: `precommit-check.sh`
//! has never been in its loop, so a machine set up through `/playbook:setup`
//! already ends up with a `settings.json` naming a guard that is not on disk
//! the moment `init` runs.
//!
//! A `settings.json` command pointing at a missing script fails open and
//! silent: the guard simply never fires, and nothing reports it. That is the
//! failure the `hook-rename-lockstep-settings` memory fact records, roughly
//! 110 silent errors over 28 hours on 2026-08-11, and it is the WU-8
//! guard-stub defect in a different costume. Both were invisible because the
//! wiring looked correct when read.
//!
//! Two properties keep it from coming back:
//!
//! 1. The guard list is DERIVED from `wire`'s own `GUARD_SPECS`, filtered on
//!    `!ported`, never copied into a second hardcoded list here. When WU-13
//!    ports a guard to Rust and flips its `ported` flag, `wire` stops writing
//!    a `.sh` command for it and this module stops copying it, in one edit.
//! 2. `place_guards` reports every guard it actually placed in `wired`, and
//!    `wire` writes a guard command only for a name in that list. A guard
//!    whose source is missing lands in `failures` instead, is left out of
//!    `wired`, and so is left out of `settings.json`, rather than becoming a
//!    dangling command. A caller can no longer mistake a partial success for
//!    a complete one, because `failures` being non-empty says so directly.
//!
//! WU-13 deletes this module once all four guards run inside the binary.

use crate::init::wire;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};

/// Everything that can stop a guard reaching its destination as an
/// executable file. `MissingSource` is called out separately from `Io`
/// because it means the shipped tree is incomplete, which is a packaging
/// bug the user cannot fix, while `NotExecutable` means the copy landed but
/// would not run, which is the condition that actually produces a silent
/// guard.
#[derive(Debug)]
pub enum GuardError {
    MissingSource { name: &'static str, path: PathBuf },
    NotExecutable(PathBuf),
    Io(io::Error),
}

impl std::fmt::Display for GuardError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            GuardError::MissingSource { name, path } => write!(
                f,
                "guard {name} is not shipped at {}, so it cannot be placed",
                path.display()
            ),
            GuardError::NotExecutable(path) => write!(
                f,
                "{} is not executable after placement, so the guard would never fire",
                path.display()
            ),
            GuardError::Io(err) => write!(f, "{err}"),
        }
    }
}

impl From<io::Error> for GuardError {
    fn from(err: io::Error) -> Self {
        GuardError::Io(err)
    }
}

/// Which guards `place_guards` wrote, which were already byte-identical and
/// executable at the destination, which are therefore safe for `wire` to
/// write a command for, and which could not be placed at all.
///
/// `placed` and `already_current` together are exactly the guards that
/// landed on disk; `wired` is that same set by name, in `GUARD_SPECS` order,
/// ready to hand straight to `wire`. `failures` holds a `GuardError` for
/// every guard that did NOT land, one entry per guard, so a caller can
/// render every failure rather than only the first.
pub struct GuardOutcome {
    pub placed: Vec<PathBuf>,
    pub already_current: Vec<PathBuf>,
    pub wired: Vec<&'static str>,
    pub failures: Vec<GuardError>,
}

impl GuardOutcome {
    /// Whether anything was written. Lets the caller distinguish a run that
    /// repaired a machine from one that found it already correct, the same
    /// split `init::shim` and `init::statusline` report.
    pub fn changed(&self) -> bool {
        !self.placed.is_empty()
    }
}

/// Copy every guard `wire` still points at a `.sh` path from
/// `self_root/hooks/` into `claude_home/hooks/`, then confirm each landed as
/// an executable file. Returns a `GuardOutcome` naming, among the guards
/// `GUARD_SPECS` still marks unported, which landed and which did not.
///
/// A total function: it never stops at the first guard that cannot be
/// placed. Each guard is independent, so a source missing for one must not
/// cost the three others their placement; the previous shape, which
/// returned `Err` on the first failure and discarded whatever had already
/// landed, meant a broken shipped tree cost every guard rather than just the
/// broken one. Every guard that lands, whether freshly copied or already
/// current, is added to both the matching `Vec<PathBuf>` and to `wired`;
/// every guard that cannot be placed is added only to `failures`. `run`
/// reports the step `Failed` when `failures` is non-empty, and always passes
/// `wired` to `wire`, so a broken shipped tree costs only the guard commands
/// for the guards that actually failed, never the eleven ported hooks and
/// never the guards that did land.
pub fn place_guards(self_root: &Path, claude_home: &Path) -> GuardOutcome {
    let mut outcome = GuardOutcome {
        placed: Vec::new(),
        already_current: Vec::new(),
        wired: Vec::new(),
        failures: Vec::new(),
    };

    for name in wire::unported_guard_names() {
        let source = self_root.join("hooks").join(format!("{name}.sh"));
        if !source.is_file() {
            outcome
                .failures
                .push(GuardError::MissingSource { name, path: source });
            continue;
        }
        let dest = claude_home.join("hooks").join(format!("{name}.sh"));

        if is_already_current(&source, &dest) {
            outcome.already_current.push(dest);
            outcome.wired.push(name);
            continue;
        }

        if let Err(err) = copy_guard_atomically(&source, &dest) {
            outcome.failures.push(GuardError::Io(err));
            continue;
        }
        if let Err(err) = verify_executable(&dest) {
            outcome.failures.push(err);
            continue;
        }
        outcome.placed.push(dest);
        outcome.wired.push(name);
    }

    outcome
}

/// Whether `dest` already holds exactly `src`'s bytes AND would run. Both
/// halves matter: comparing contents alone would call a guard current after
/// something stripped its executable bit, which is precisely the silent-guard
/// state this module exists to prevent.
fn is_already_current(src: &Path, dest: &Path) -> bool {
    let (Ok(shipped), Ok(placed)) = (fs::read(src), fs::read(dest)) else {
        return false;
    };
    shipped == placed && verify_executable(dest).is_ok()
}

/// Copy `src` to `dest` through a sibling temp file and a rename, the same
/// shape `init::statusline::copy_statusline_atomically` uses, so no reader
/// and no crash mid-copy ever observes a half-written guard.
///
/// `fs::copy` carries the source's permission bits, and all four guards ship
/// 0755, so the executable bit normally arrives for free. The explicit chmod
/// is still here because "normally" is doing real work in that sentence: a
/// checkout with a restrictive umask, an archive extracted without modes, or
/// a future guard added without the bit set would otherwise produce a file
/// that exists, matches byte for byte, and never runs. `verify_executable`
/// then confirms the result rather than trusting either step.
fn copy_guard_atomically(src: &Path, dest: &Path) -> io::Result<()> {
    let dir = dest
        .parent()
        .filter(|p| !p.as_os_str().is_empty())
        .unwrap_or_else(|| Path::new("."));
    fs::create_dir_all(dir)?;

    let file_name = dest.file_name().and_then(|n| n.to_str()).unwrap_or("guard");
    let tmp_path = dir.join(format!(".{file_name}-{}.tmp", std::process::id()));

    let result = fs::copy(src, &tmp_path)
        .and_then(|_| set_executable(&tmp_path))
        .and_then(|()| fs::rename(&tmp_path, dest));
    if result.is_err() {
        let _ = fs::remove_file(&tmp_path);
    }
    result
}

/// Force mode 0755 on unix. A no-op elsewhere: Windows has no mode bits, so
/// there is nothing to set and `verify_executable` does not look for one.
#[cfg(unix)]
fn set_executable(path: &Path) -> io::Result<()> {
    use std::os::unix::fs::PermissionsExt;
    fs::set_permissions(path, fs::Permissions::from_mode(0o755))
}

#[cfg(not(unix))]
fn set_executable(_path: &Path) -> io::Result<()> {
    Ok(())
}

/// Confirm `path` is a regular file that would actually execute. Stats it,
/// checks an execute bit is set for some class on unix, and then opens it,
/// because a stat can succeed on a file this process cannot read.
fn verify_executable(path: &Path) -> Result<(), GuardError> {
    let metadata = fs::metadata(path)?;
    if !metadata.is_file() {
        return Err(GuardError::NotExecutable(path.to_path_buf()));
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        if metadata.permissions().mode() & 0o111 == 0 {
            return Err(GuardError::NotExecutable(path.to_path_buf()));
        }
    }
    fs::File::open(path)?;
    Ok(())
}
