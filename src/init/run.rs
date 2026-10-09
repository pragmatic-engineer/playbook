// SPDX-FileCopyrightText: 2026 Igor Santos
// SPDX-License-Identifier: Apache-2.0

//! Orchestrates `playbook init`: composes `merge`, `wire` and `shim`
//! into one idempotent repair, backing `Command::Init`.

use crate::init::merge;
use crate::init::migrate;
use crate::init::path;
use crate::init::shim::{self, ShellKind};
use crate::init::system_prompt;
use crate::init::wire;
use std::fs;
use std::path::{Path, PathBuf};

/// Everything `run` needs to locate and repair a machine's Claude Code
/// configuration, resolved once by the caller (`main.rs`) so this module
/// never reads the environment itself and stays trivial to test against a
/// scratch directory, the same split `init::shim`
/// already draw between resolving paths and acting on them.
pub struct InitPaths {
    /// Where the shipped template, shell runtime and system prompt live.
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
    /// Whether the user asked for `prompts/SYSTEM_PROMPT.md` via
    /// `--system-prompt`. False still refreshes an already-installed copy;
    /// see `init::system_prompt` for why installing one unasked would be a
    /// behaviour change rather than a port.
    pub system_prompt: bool,
    /// Whether the user asked for the shell launcher shim via `--aliases`,
    /// which `/playbook:setup` forwards. False
    /// skips the `shim` step entirely: unlike `system_prompt`, there is no
    /// "refresh an existing copy" case here, since a launcher a user never
    /// asked for should not be touched at all.
    pub aliases: bool,
    /// Whether to wire the hook entries into settings.json.
    pub hooks: bool,
    /// Whether to merge the shared settings template into settings.json.
    pub settings: bool,
    /// Where the binary lives and the shell to wire it for. `None` skips the
    /// `path` step.
    pub path_setup: Option<path::Setup>,
    /// `(repo_root, dest_base)` when init runs inside a repo with a resolvable slug.
    pub repo: Option<(PathBuf, PathBuf)>,
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
    pub(crate) fn wired(name: &'static str, detail: impl Into<String>) -> Self {
        Self {
            name,
            status: StepStatus::Wired,
            detail: detail.into(),
        }
    }

    pub(crate) fn already_correct(name: &'static str, detail: impl Into<String>) -> Self {
        Self {
            name,
            status: StepStatus::AlreadyCorrect,
            detail: detail.into(),
        }
    }

    pub(crate) fn skipped(name: &'static str, detail: impl Into<String>) -> Self {
        Self {
            name,
            status: StepStatus::Skipped,
            detail: detail.into(),
        }
    }

    pub(crate) fn failed(name: &'static str, detail: impl Into<String>) -> Self {
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
    /// Manual migration findings, for stderr.
    pub warnings: Vec<String>,
}

impl InitOutcome {
    /// Whether every step that attempted a change succeeded. A step that
    /// was deliberately skipped does not count against this.
    pub fn ok(&self) -> bool {
        !self.steps.iter().any(|s| s.status == StepStatus::Failed)
    }
}

/// Run every `init` step against `paths`. Copy steps run first; `settings`
/// and `shim` only rewrite their file once their copy step is confirmed.
pub fn run(paths: &InitPaths) -> InitOutcome {
    let settings_path = paths.claude_home.join("settings.json");
    let self_root = paths.self_root.as_deref();
    // One stamp for every backup of this run, so a second boundary cannot split them.
    let epoch = crate::common::time::now_epoch_secs();

    let ctx = migrate::Ctx {
        home: paths.home.clone(),
        claude_home: paths.claude_home.clone(),
        self_root: paths.self_root.clone(),
        repo: paths.repo.clone(),
    };
    let migrated = migrate::run_pending(&ctx);

    let system_prompt_step = place_system_prompt_step(self_root, &paths.home, paths.system_prompt);
    if step_confirmed(&system_prompt_step) {
        migrate::record_system_prompt(&paths.home);
    }
    if let Some(root) = self_root {
        migrate::record_skills(&paths.home, root);
    }

    let settings_step = if paths.settings {
        seed_or_merge_settings(
            self_root,
            &paths.claude_home,
            &settings_path,
            epoch,
            paths.hooks,
        )
    } else {
        StepReport::skipped("settings", "not requested; pass --settings to opt in")
    };
    let hooks_step = if paths.hooks {
        wire_hooks(&settings_path, epoch)
    } else {
        StepReport::skipped("hooks", "not requested; pass --hooks to opt in")
    };

    let path_step = path_step(&paths.home, paths.path_setup.as_ref());
    let shim_step = rewire_rc_file_step(&paths.home, paths.shell_kind, paths.aliases);

    let trust_step = trust_config_dir_step(&paths.home);

    let mut steps = vec![
        system_prompt_step,
        settings_step,
        hooks_step,
        path_step,
        shim_step,
    ];
    steps.extend(migrated.steps);
    steps.push(trust_step);
    InitOutcome {
        steps,
        warnings: migrated.warnings,
    }
}

/// Puts the binary's directory on PATH for every shell start. A failure is
/// reported, never fatal: the binary still runs by absolute path.
fn path_step(home: &Path, setup: Option<&path::Setup>) -> StepReport {
    let Some(setup) = setup else {
        return StepReport::skipped("path", "not requested, or the binary location is unknown");
    };
    match path::ensure(home, setup) {
        Ok(out) if !out.changed.is_empty() => {
            let files: Vec<String> = out
                .changed
                .iter()
                .map(|f| f.display().to_string())
                .collect();
            StepReport::wired(
                "path",
                format!(
                    "added {} to PATH in {}. Open a new terminal to pick it up.",
                    setup.bin_dir.display(),
                    files.join(", ")
                ),
            )
        }
        Ok(out) if !out.skipped.is_empty() => {
            let (file, why) = &out.skipped[0];
            StepReport::skipped(
                "path",
                format!(
                    "{}: {why}. Add {} to PATH by hand.",
                    file.display(),
                    setup.bin_dir.display()
                ),
            )
        }
        Ok(_) => StepReport::already_correct(
            "path",
            format!(
                "{} is already on PATH for new shells",
                setup.bin_dir.display()
            ),
        ),
        Err(err) => StepReport::failed("path", err.to_string()),
    }
}

/// Marks `~/.config/playbook` as trusted in `~/.claude.json`, so a session
/// started in that folder skips Claude Code's trust dialog. Best-effort: a
/// missing file or any failure is reported as skipped, never as a failure.
fn trust_config_dir_step(home: &Path) -> StepReport {
    let claude_json = home.join(".claude.json");
    let config_dir = home.join(".config").join("playbook");
    let dir = config_dir.to_string_lossy();
    if !claude_json.exists() {
        return StepReport::skipped("trust", "no ~/.claude.json yet; nothing to update");
    }
    if crate::trust::is_trusted(&claude_json, &dir) {
        return StepReport::already_correct("trust", "config dir already trusted");
    }
    match crate::trust::write_trust_entry(&claude_json, &dir) {
        Ok(()) => StepReport::wired("trust", format!("trusted {dir}")),
        Err(err) => StepReport::skipped("trust", format!("could not update ~/.claude.json: {err}")),
    }
}

/// Whether a copy step actually landed: `Skipped` does not count, since it
/// means the destination's state is unconfirmed, not verified complete.
fn step_confirmed(step: &StepReport) -> bool {
    matches!(step.status, StepStatus::Wired | StepStatus::AlreadyCorrect)
}

/// Step 6: place `prompts/SYSTEM_PROMPT.md`, which is opt-in. See
/// `init::system_prompt` for why `init` refreshes an existing copy but never
/// installs one the user did not ask for.
fn place_system_prompt_step(self_root: Option<&Path>, home: &Path, opt_in: bool) -> StepReport {
    let Some(self_root) = self_root else {
        return StepReport::skipped(
            "system-prompt",
            "CLAUDE_PLUGIN_ROOT is not set, no prompt to place",
        );
    };
    match system_prompt::place_system_prompt(self_root, home, opt_in) {
        Ok(system_prompt::Placement::Placed(dest)) => {
            StepReport::wired("system-prompt", format!("placed at {}", dest.display()))
        }
        Ok(system_prompt::Placement::AlreadyCurrent(dest)) => StepReport::already_correct(
            "system-prompt",
            format!("already up to date at {}", dest.display()),
        ),
        Ok(system_prompt::Placement::UserEdited(dest)) => StepReport::skipped(
            "system-prompt",
            format!("left your edited copy at {}", dest.display()),
        ),
        Ok(system_prompt::Placement::NotShipped(source)) => StepReport::skipped(
            "system-prompt",
            format!("not shipped at {}", source.display()),
        ),
        Ok(system_prompt::Placement::NotOptedIn) => StepReport::skipped(
            "system-prompt",
            "not installed; pass --system-prompt to opt in",
        ),
        Err(err) => StepReport::failed("system-prompt", err.to_string()),
    }
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
/// this key" branch. The retired `setup-local.sh` instead special-cased a fresh install
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
    epoch: u64,
    with_hooks: bool,
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
    if !with_hooks {
        return merge_without_hooks(
            claude_home,
            &template_path,
            settings_path,
            &base_path,
            epoch,
        );
    }
    match merge::merge(&base_path, &template_path, settings_path, &base_path, None) {
        Ok(outcome) => finish_merge(settings_path, &outcome, epoch),
        Err(merge::MergeError::Validation(err)) => StepReport::failed("settings", err.to_string()),
        Err(merge::MergeError::Io(err)) => StepReport::failed("settings", err.to_string()),
    }
}

/// The settings merge for `--no-hooks`: the template's `hooks` block (the
/// `PreToolUse` guards) is left out, and whatever hooks the user's file
/// already has are kept as they are. The merge runs against a hook-free copy
/// of the template and a scratch base, so the recorded base still describes
/// the full template and a later run with hooks on merges normally.
fn merge_without_hooks(
    claude_home: &Path,
    template_path: &Path,
    settings_path: &Path,
    base_path: &Path,
    epoch: u64,
) -> StepReport {
    let scratch = |name: &str| claude_home.join(format!(".settings.{name}.{epoch}.tmp"));
    let (tpl_tmp, base_tmp) = (scratch("template-nohooks"), scratch("base-nohooks"));
    let result = (|| -> Result<merge::MergeOutcome, String> {
        let mut template: serde_json::Value =
            serde_json::from_str(&fs::read_to_string(template_path).map_err(|e| e.to_string())?)
                .map_err(|e| format!("{}: {e}", template_path.display()))?;
        if let Some(map) = template.as_object_mut() {
            map.remove("hooks");
        }
        fs::write(&tpl_tmp, template.to_string()).map_err(|e| e.to_string())?;
        if base_path.is_file() {
            fs::copy(base_path, &base_tmp).map_err(|e| e.to_string())?;
        }
        let mut outcome = merge::merge(&base_tmp, &tpl_tmp, settings_path, &base_tmp, None)
            .map_err(|e| match e {
                merge::MergeError::Validation(err) => err.to_string(),
                merge::MergeError::Io(err) => err.to_string(),
            })?;
        let user: serde_json::Value = fs::read_to_string(settings_path)
            .ok()
            .and_then(|t| serde_json::from_str(&t).ok())
            .unwrap_or(serde_json::Value::Null);
        let mut merged: serde_json::Value =
            serde_json::from_str(&outcome.stdout).map_err(|e| e.to_string())?;
        if let Some(map) = merged.as_object_mut() {
            match user.get("hooks") {
                Some(hooks) => {
                    map.insert("hooks".to_string(), hooks.clone());
                }
                None => {
                    map.remove("hooks");
                }
            }
        }
        outcome.stdout = serde_json::to_string_pretty(&merged).map_err(|e| e.to_string())?;
        Ok(outcome)
    })();
    let _ = fs::remove_file(&tpl_tmp);
    let _ = fs::remove_file(&base_tmp);
    match result {
        Ok(outcome) => finish_merge(settings_path, &outcome, epoch),
        Err(err) => StepReport::failed("settings", err),
    }
}

/// Turn a completed merge into a `StepReport`, comparing its rendered output
/// against what is already on disk before writing anything: a no-op merge
/// (the common case on a re-run) neither takes a backup nor rewrites the
/// file, mirroring `wire::wire`'s own idempotence check.
fn finish_merge(settings_path: &Path, outcome: &merge::MergeOutcome, epoch: u64) -> StepReport {
    let rendered = format!("{}\n", outcome.stdout);
    let existing = fs::read_to_string(settings_path).unwrap_or_default();
    if rendered == existing {
        return StepReport::already_correct("settings", "already matches the template");
    }
    match backup_then_write(settings_path, &rendered, &outcome.skipped, epoch) {
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
///
/// Also writes `skipped` beside the backup, as
/// `settings-merge-skipped.<epoch>.json`, using the SAME epoch as the backup
/// so the two files one real write produces are easy to pair up by eye;
/// written only when `skipped` is non-empty, since an idempotent re-run
/// never reaches this function at all (`finish_merge` returns before calling
/// it), and a real write that withheld nothing has no report worth keeping.
/// Reuses `merge::render_skip_report` rather than re-deriving the shape, so
/// this stays byte-for-byte the same shape `merge::merge`'s own SKIP_OUT
/// would have written, without going through `merge::merge`'s `skip_out`
/// parameter itself: that parameter writes unconditionally whenever `Some`,
/// even an empty array (N3, pinned by `tests/init_merge.rs`'s
/// `n3_zero_withheld_keys_writes_empty_skip_array`), and its target path
/// would have to be decided before `merge::merge` runs, before this
/// function's epoch even exists. `outcome.skipped` is already in memory by
/// the time `finish_merge` calls this, so no second call or disk round-trip
/// is needed to get it.
///
/// Both file families this function can produce are unbounded without
/// pruning, so after writing, `prune_family` retains only the 5
/// newest-epoch files in each: the `.bak.<epoch>` backups and the
/// `settings-merge-skipped.<epoch>.json` reports. This runs only on this,
/// the real-write path; `finish_merge`'s idempotent short-circuit means an
/// idempotent re-run never prunes either family, matching the same "nothing
/// changed, nothing happens" rule the write itself already follows.
///
/// A backup already present under `epoch` is kept, so the first write of a
/// run (or of a second) leaves the pre-change copy and the later ones add none.
fn backup_then_write(
    path: &Path,
    content: &str,
    skipped: &[merge::SkippedEntry],
    epoch: u64,
) -> std::io::Result<()> {
    let dir = path
        .parent()
        .filter(|p| !p.as_os_str().is_empty())
        .unwrap_or_else(|| Path::new("."));

    if path.is_file() {
        let file_name = path
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_else(|| "settings.json".to_string());
        let backup = path.with_file_name(format!("{file_name}.bak.{epoch}"));
        // The first backup of a run holds the pre-run state; keep it.
        if !backup.exists() {
            fs::copy(path, &backup)?;
        }

        if !skipped.is_empty() {
            fs::write(
                path.with_file_name(format!("settings-merge-skipped.{epoch}.json")),
                merge::render_skip_report(skipped),
            )?;
        }

        prune_family(dir, &format!("{file_name}.bak."), "");
        prune_family(dir, "settings-merge-skipped.", ".json");
    }

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

/// The retain-5 pruning policy for both of `backup_then_write`'s file
/// families: keep only the 5 files directly in `dir` named
/// `{prefix}<epoch>{suffix}` with the highest embedded epoch, deleting the
/// rest. Parses the epoch back out of each matching file name rather than
/// trusting file modification times, since a fabricated or copied file's
/// mtime need not agree with the epoch its own name claims, and the epoch in
/// the name is what both families are already keyed by everywhere else. A
/// name matching `{prefix}...{suffix}` whose middle segment does not parse
/// as a `u64` is left alone rather than guessed about; this only ever runs
/// against files this crate itself named, so an unparsable match is not
/// expected in practice. Best-effort like the rest of this module's writes:
/// a `read_dir` or `remove_file` failure is swallowed rather than turned
/// into a step failure, since a pruning miss leaves stale files behind
/// rather than losing data.
fn prune_family(dir: &Path, prefix: &str, suffix: &str) {
    let Ok(read_dir) = fs::read_dir(dir) else {
        return;
    };
    let mut entries: Vec<(u64, PathBuf)> = read_dir
        .flatten()
        .filter_map(|entry| {
            let name = entry.file_name().to_string_lossy().into_owned();
            let epoch_str = name.strip_prefix(prefix)?.strip_suffix(suffix)?;
            let epoch: u64 = epoch_str.parse().ok()?;
            Some((epoch, entry.path()))
        })
        .collect();
    if entries.len() <= 5 {
        return;
    }
    entries.sort_unstable_by_key(|(epoch, _)| *epoch);
    let excess = entries.len() - 5;
    for (_, stale_path) in entries.into_iter().take(excess) {
        let _ = fs::remove_file(stale_path);
    }
}

/// Step 2: upsert the ported hooks and guards into `settings.json`. Every
/// `GUARD_SPECS` entry is wired (WU-13), so `wire` wires every hook
/// and guard unconditionally; WU-14 dropped the `placed_guards` and
/// `claude_home` parameters `wire::wire` used to take, since the gate they
/// fed had become permanently unreachable.
fn wire_hooks(settings_path: &Path, epoch: u64) -> StepReport {
    match wire::wire_at(settings_path, epoch) {
        Ok(outcome) if outcome.changed => {
            StepReport::wired("hooks", "wired the ported hooks into settings.json")
        }
        Ok(_) => StepReport::already_correct("hooks", "all hooks already wired"),
        Err(err) => StepReport::failed("hooks", err.to_string()),
    }
}

/// Patch the rc file. Gated on `aliases`, then `$SHELL`.
fn rewire_rc_file_step(home: &Path, shell_kind: Option<ShellKind>, aliases: bool) -> StepReport {
    if !aliases {
        return StepReport::skipped("shim", "not installed; pass --aliases to opt in");
    }
    let Some(shell_kind) = shell_kind else {
        return StepReport::skipped(
            "shim",
            "$SHELL is neither bash nor zsh; add `eval \"$(playbook shell-init)\"` to your rc file by hand",
        );
    };
    match shim::rewire_rc_file(home, shell_kind) {
        Ok(outcome) if outcome.unwritable => StepReport::skipped(
            "shim",
            format!(
                "{} is not writable; left unchanged, source the launcher there by hand",
                outcome.rc_file.display()
            ),
        ),
        Ok(outcome) if outcome.appended => StepReport::wired(
            "shim",
            format!(
                "wired `playbook shell-init` in {}",
                outcome.rc_file.display()
            ),
        ),
        Ok(outcome) => StepReport::already_correct(
            "shim",
            format!("{} already loads the launcher", outcome.rc_file.display()),
        ),
        Err(err) => StepReport::failed("shim", err.to_string()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::common::test_support::scratch_dir;

    #[test]
    fn a_backup_under_the_same_stamp_is_kept_and_the_write_still_lands() {
        let dir = scratch_dir("init-backup-kept");
        fs::create_dir_all(&dir).unwrap();
        let path = dir.join("settings.json");
        fs::write(&path, "current").unwrap();
        let backup = dir.join("settings.json.bak.1700000000");
        fs::write(&backup, "pre-run original").unwrap();

        backup_then_write(&path, "new", &[], 1_700_000_000).unwrap();

        assert_eq!(fs::read_to_string(&backup).unwrap(), "pre-run original");
        assert_eq!(fs::read_to_string(&path).unwrap(), "new");
    }
}
