// SPDX-FileCopyrightText: 2026 Igor Santos
// SPDX-License-Identifier: MIT

//! Orchestrates `playbook init`: composes `merge`, `wire`, `shim` and
//! `statusline` into one idempotent repair, backing `Command::Init`.
//!
//! Steps run independently and best-effort: a failure in one (a malformed
//! `settings.json`, say) does not stop the others from making whatever
//! progress they can, since each targets a different file on disk and a
//! user running `init` to repair a broken machine is better served by
//! partial progress plus a clear failure line than by an all-or-nothing
//! abort. `InitOutcome::ok` still reports overall failure if any step
//! failed, so a caller chaining on the exit code sees it.
//!
//! Ordering is not just cosmetic: `settings` seeds or three-way-merges
//! `settings.json` from the shipped template BEFORE `hooks` upserts into it.
//! `merge` only reconciles whole top-level keys (see `init::merge`'s doc
//! comment: a key the user customised is kept or dropped as a whole), while
//! `wire` reconciles individual hook entries inside `.hooks`. Running `wire`
//! first would leave it nothing to upsert into on a fresh machine; running
//! the merge after `wire` would risk the merge's coarser per-key policy
//! discarding an entry `wire` just added. `statusline` runs last because it
//! depends on `settings.json` already naming a destination.

use crate::init::merge;
use crate::init::shim::{self, ShellKind};
use crate::init::statusline;
use crate::init::wire;
use std::fs;
use std::path::{Path, PathBuf};

/// Everything `run` needs to locate and repair a machine's Claude Code
/// configuration, resolved once by the caller (`main.rs`) so this module
/// never reads the environment itself and stays trivial to test against a
/// scratch directory, the same split `init::shim` and `init::statusline`
/// already draw between resolving paths and acting on them.
pub struct InitPaths {
    /// Where the shipped template, shell runtime and `statusline.sh` live.
    /// `None` when `CLAUDE_PLUGIN_ROOT` is unset, in which case every step
    /// that needs it is skipped rather than guessing a path.
    pub self_root: Option<PathBuf>,
    /// `$HOME/.claude`, where `settings.json` and the launcher runtime live.
    pub claude_home: PathBuf,
    /// The user's home directory, for the rc file `shim` wires.
    pub home: PathBuf,
    /// `None` when `$SHELL` names neither bash nor zsh, in which case the
    /// shim step is skipped with instructions to source it manually.
    pub shell_kind: Option<ShellKind>,
}

/// How one step of `run` landed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StepStatus {
    /// The step wrote a change.
    Wired,
    /// Nothing needed to change; already in the target shape.
    AlreadyCorrect,
    /// The step could not run, for a reason that is not itself a failure
    /// (missing `CLAUDE_PLUGIN_ROOT`, an unrecognised `$SHELL`).
    Skipped,
    /// The step tried and failed.
    Failed,
}

/// One step's result, ready to render as a report line.
pub struct StepReport {
    pub name: &'static str,
    pub status: StepStatus,
    pub detail: String,
}

impl StepReport {
    fn wired(name: &'static str, detail: impl Into<String>) -> Self {
        Self {
            name,
            status: StepStatus::Wired,
            detail: detail.into(),
        }
    }

    fn already_correct(name: &'static str, detail: impl Into<String>) -> Self {
        Self {
            name,
            status: StepStatus::AlreadyCorrect,
            detail: detail.into(),
        }
    }

    fn skipped(name: &'static str, detail: impl Into<String>) -> Self {
        Self {
            name,
            status: StepStatus::Skipped,
            detail: detail.into(),
        }
    }

    fn failed(name: &'static str, detail: impl Into<String>) -> Self {
        Self {
            name,
            status: StepStatus::Failed,
            detail: detail.into(),
        }
    }

    /// One terse line for a human, e.g. `hooks: wired - ...`.
    pub fn render(&self) -> String {
        let verb = match self.status {
            StepStatus::Wired => "wired",
            StepStatus::AlreadyCorrect => "ok",
            StepStatus::Skipped => "skipped",
            StepStatus::Failed => "FAILED",
        };
        format!("{}: {verb} - {}", self.name, self.detail)
    }
}

/// Every step's outcome, in run order.
pub struct InitOutcome {
    pub steps: Vec<StepReport>,
}

impl InitOutcome {
    /// Whether every step that attempted a change succeeded. A step that
    /// was deliberately skipped does not count against this.
    pub fn ok(&self) -> bool {
        !self.steps.iter().any(|s| s.status == StepStatus::Failed)
    }
}

/// Run every `init` step against `paths`. See the module doc comment for
/// step order and why one step's failure does not stop the others.
pub fn run(paths: &InitPaths) -> InitOutcome {
    let settings_path = paths.claude_home.join("settings.json");
    let self_root = paths.self_root.as_deref();

    let steps = vec![
        seed_or_merge_settings(self_root, &paths.claude_home, &settings_path),
        wire_hooks(&settings_path),
        install_shim_step(self_root, &paths.claude_home, &paths.home, paths.shell_kind),
        place_statusline_step(self_root, &settings_path, &paths.home),
    ];

    InitOutcome { steps }
}

/// Step 1: seed a fresh `settings.json` from the shipped template, or
/// three-way-merge an existing one, always through `merge::merge`. The
/// merge's BASE lives at `claude_home/.settings.base.json` and is refreshed
/// on every run regardless of whether `settings.json` itself changed, the
/// same way `init::merge::merge` already refreshes NEWBASE_OUT unconditionally.
///
/// A missing `settings.json` is treated as a user with zero customisations:
/// it is seeded as an empty object first, so `merge::merge` loads it validly
/// and adopts every template key through its ordinary "user never touched
/// this key" branch. `setup-local.sh` instead special-cases a fresh install
/// as a verbatim template copy, which is byte-for-byte faithful to the
/// template's own key order on that one run, but `merge::merge` serialises
/// its OWN output with keys in sorted order (matching the python it ports),
/// so the very next `init` run would silently reorder the file and report a
/// spurious change. Going through `merge::merge` from the first run avoids
/// that: the file is in its final, stable shape immediately.
fn seed_or_merge_settings(
    self_root: Option<&Path>,
    claude_home: &Path,
    settings_path: &Path,
) -> StepReport {
    let Some(self_root) = self_root else {
        return StepReport::skipped(
            "settings",
            "CLAUDE_PLUGIN_ROOT is not set, no template to seed from",
        );
    };
    let template_path = self_root.join("settings.shared.json");
    if !template_path.is_file() {
        return StepReport::skipped(
            "settings",
            format!("no template shipped at {}", template_path.display()),
        );
    }

    if !settings_path.is_file() {
        let init_empty =
            fs::create_dir_all(claude_home).and_then(|()| fs::write(settings_path, "{}\n"));
        if let Err(err) = init_empty {
            return StepReport::failed(
                "settings",
                format!("could not initialise {}: {err}", settings_path.display()),
            );
        }
    }

    let base_path = claude_home.join(".settings.base.json");
    match merge::merge(&base_path, &template_path, settings_path, &base_path, None) {
        Ok(outcome) => finish_merge(settings_path, &outcome),
        Err(merge::MergeError::Validation(err)) => StepReport::failed("settings", err.to_string()),
        Err(merge::MergeError::Io(err)) => StepReport::failed("settings", err.to_string()),
    }
}

/// Turn a completed merge into a `StepReport`, comparing its rendered output
/// against what is already on disk before writing anything: a no-op merge
/// (the common case on a re-run) neither takes a backup nor rewrites the
/// file, mirroring `wire::wire`'s own idempotence check.
fn finish_merge(settings_path: &Path, outcome: &merge::MergeOutcome) -> StepReport {
    let rendered = format!("{}\n", outcome.stdout);
    let existing = fs::read_to_string(settings_path).unwrap_or_default();
    if rendered == existing {
        return StepReport::already_correct("settings", "already matches the template");
    }
    match backup_then_write(settings_path, &rendered) {
        Ok(()) => StepReport::wired(
            "settings",
            format!(
                "merged the template in ({} customisation(s) preserved)",
                outcome.skipped.len()
            ),
        ),
        Err(err) => StepReport::failed(
            "settings",
            format!("could not write {}: {err}", settings_path.display()),
        ),
    }
}

/// Copy `path` to a timestamped sibling before overwriting it with
/// `content`, the same safety net `init::wire::wire` gives its own
/// `settings.json` changes. Duplicated rather than shared: `wire`'s
/// equivalent helpers are private to that module, and this is the only
/// other place in the crate that rewrites `settings.json` wholesale, so
/// promoting them to `pub(crate)` would widen that module's surface for one
/// caller.
fn backup_then_write(path: &Path, content: &str) -> std::io::Result<()> {
    if path.is_file() {
        let epoch = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_secs())
            .unwrap_or(0);
        let file_name = path
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_else(|| "settings.json".to_string());
        fs::copy(
            path,
            path.with_file_name(format!("{file_name}.bak.{epoch}")),
        )?;
    }
    let dir = path
        .parent()
        .filter(|p| !p.as_os_str().is_empty())
        .unwrap_or_else(|| Path::new("."));
    fs::create_dir_all(dir)?;
    let tmp_path = dir.join(format!(".init-settings-{}.tmp", std::process::id()));
    if let Err(err) = fs::write(&tmp_path, content) {
        let _ = fs::remove_file(&tmp_path);
        return Err(err);
    }
    if let Err(err) = fs::rename(&tmp_path, path) {
        let _ = fs::remove_file(&tmp_path);
        return Err(err);
    }
    Ok(())
}

/// Step 2: upsert the ported hooks and the four guards into `settings.json`.
fn wire_hooks(settings_path: &Path) -> StepReport {
    match wire::wire(settings_path) {
        Ok(outcome) if outcome.changed => {
            StepReport::wired("hooks", "wired the ported hooks into settings.json")
        }
        Ok(_) => StepReport::already_correct("hooks", "all hooks already wired"),
        Err(err) => StepReport::failed("hooks", err.to_string()),
    }
}

/// Step 3: install the launcher runtime and wire the rc file, skipped
/// cleanly when either input is missing rather than guessed.
fn install_shim_step(
    self_root: Option<&Path>,
    claude_home: &Path,
    home: &Path,
    shell_kind: Option<ShellKind>,
) -> StepReport {
    let Some(self_root) = self_root else {
        return StepReport::skipped(
            "shim",
            "CLAUDE_PLUGIN_ROOT is not set, no launcher runtime to install",
        );
    };
    let Some(shell_kind) = shell_kind else {
        return StepReport::skipped(
            "shim",
            "$SHELL is neither bash nor zsh; source shell/bash/cc.sh or shell/zsh/cc.zsh manually",
        );
    };
    match shim::install_shim(self_root, claude_home, home, shell_kind) {
        Ok(outcome) if outcome.appended => StepReport::wired(
            "shim",
            format!(
                "added the launcher source line to {}",
                outcome.rc_file.display()
            ),
        ),
        Ok(outcome) => StepReport::already_correct(
            "shim",
            format!("{} already sources the launcher", outcome.rc_file.display()),
        ),
        Err(err) => StepReport::failed("shim", err.to_string()),
    }
}

/// Step 4: place `statusline.sh` at the path `settings.json` names.
///
/// `statusline::place_statusline` always copies unconditionally, so whether
/// this is a real change has to be decided BEFORE calling it, by comparing
/// the shipped script against whatever is already at the resolved
/// destination; deciding afterwards would find them equal on every run,
/// wired or not, since the copy already landed.
fn place_statusline_step(
    self_root: Option<&Path>,
    settings_path: &Path,
    home: &Path,
) -> StepReport {
    let Some(self_root) = self_root else {
        return StepReport::skipped(
            "statusline",
            "CLAUDE_PLUGIN_ROOT is not set, no statusline.sh to place",
        );
    };
    let source = self_root.join("statusline.sh");
    let already_current = statusline::resolve_statusline_path(settings_path, home)
        .ok()
        .and_then(|dest| {
            let shipped = fs::read(&source).ok()?;
            let placed = fs::read(&dest).ok()?;
            Some(shipped == placed)
        })
        .unwrap_or(false);

    match statusline::place_statusline(self_root, settings_path, home) {
        Ok(dest) if already_current => StepReport::already_correct(
            "statusline",
            format!("already up to date at {}", dest.display()),
        ),
        Ok(dest) => StepReport::wired("statusline", format!("placed at {}", dest.display())),
        Err(err) => StepReport::failed("statusline", err.to_string()),
    }
}
