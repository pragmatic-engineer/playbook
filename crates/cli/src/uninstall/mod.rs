// SPDX-FileCopyrightText: 2026 Igor Santos
// SPDX-License-Identifier: Apache-2.0

//! `playbook uninstall`: the inverse of `playbook setup` and `install.sh`.
//!
//! It removes only what playbook placed: the hook entries and status line
//! `init` wrote into `settings.json`, the managed launcher block in the rc
//! files, the files `init` copies under `~/.config/playbook`, and the two
//! installer scripts in `~/.claude`, a collector service file an older setup left, and the
//! unedited skill folders `skills sync` wrote. The binary and its rc `PATH`
//! block go only on request. Memory, the data folder with every database, settings the
//! user owns, sessions, history and earlier backups are never touched.

pub mod style;

use crate::init::{migrate, shim, wire};
use std::fs;
use std::io;
use std::path::{Path, PathBuf};

/// What to remove, resolved once by the caller.
pub struct Options {
    pub home: PathBuf,
    /// Where the installer put the binary.
    pub bin_dir: PathBuf,
    pub remove_binary: bool,
    pub dry_run: bool,
}

/// One line per action, plus the steps that failed.
#[derive(Debug, Default)]
pub struct Report {
    pub lines: Vec<String>,
    pub errors: Vec<String>,
}

/// Files `init` copies under `~/.config/playbook`, relative to it, with the
/// migration key that tracks user edits where one exists.
const PLACED: &[(&str, Option<&str>)] = &[
    ("statusline.sh", Some("statusline")),
    ("prompts/SYSTEM_PROMPT.md", Some("system-prompt")),
    ("hooks/lib/config-hash.sh", None),
    ("plugin-seen", None),
];

/// Directories earlier installs filled, removed whole.
const LEGACY_DIRS: &[&str] = &["shell"];

/// Files the installer copies into `~/.claude`.
const CLAUDE_HOME_FILES: &[&str] = &["install.sh", "uninstall.sh"];

pub fn run(opts: &Options) -> Report {
    let mut report = Report::default();
    let verb = if opts.dry_run {
        "would remove"
    } else {
        "removed"
    };
    let claude_home = opts.home.join(".claude");
    let stamp = crate::common::time::now_epoch_secs();

    let settings = settings_step(opts, &claude_home, stamp, &mut report);
    rc_step(opts, stamp, &mut report);
    // The service runs the binary, so it goes before the binary can.
    match crate::init::collector_service::remove(&opts.home, opts.dry_run) {
        Ok(Some(line)) => report.lines.push(line),
        Ok(None) => {}
        Err(err) => report
            .errors
            .push(format!("collector service left in place: {err}")),
    }
    for foreign in crate::init::collector_service::foreign_files(&opts.home) {
        report.lines.push(format!(
            "{}; to remove it, run: {}",
            foreign.sentence(),
            foreign.fix
        ));
    }
    if !settings.ok {
        report.lines.push(
            "kept the placed files and the binary, since settings.json still refers to them"
                .to_string(),
        );
        remove_claude_home_files(&claude_home, verb, opts.dry_run, &mut report);
        manual_steps(&opts.home, &mut report);
        return report;
    }
    let status_line_in_use = !settings.statusline_removed
        && fs::read_to_string(claude_home.join("settings.json"))
            .is_ok_and(|text| text.contains("playbook/statusline.sh"));

    let config_dir = opts.home.join(".config").join("playbook");
    for (rel, key) in PLACED {
        let path = config_dir.join(rel);
        if !path.is_file() {
            continue;
        }
        if key.is_some_and(|k| !migrate::placed_unchanged(&opts.home, k, &path)) {
            report.lines.push(format!(
                "kept {} (edited, or not recorded as placed by playbook)",
                path.display()
            ));
            continue;
        }
        if *rel == "statusline.sh" && status_line_in_use {
            report.lines.push(format!(
                "kept {} (your status line still runs it)",
                path.display()
            ));
            continue;
        }
        remove_file(&path, verb, opts.dry_run, &mut report);
    }
    for dir in LEGACY_DIRS {
        let path = config_dir.join(dir);
        if path.is_dir() {
            remove_dir_all(&path, verb, opts.dry_run, &mut report);
        }
    }
    if !opts.dry_run {
        for rel in ["prompts", "hooks/lib", "hooks"] {
            let _ = fs::remove_dir(config_dir.join(rel));
        }
    }
    remove_claude_home_files(&claude_home, verb, opts.dry_run, &mut report);
    synced_skills_step(opts, &mut report);

    if opts.remove_binary {
        binary_step(opts, verb, &mut report);
    }
    manual_steps(&opts.home, &mut report);
    report
}

/// The skill folders `playbook skills sync` wrote for other agents. Only the
/// ones whose files still match their record go; the rest are listed as kept.
fn synced_skills_step(opts: &Options, report: &mut Report) {
    let root = crate::common::paths::playbook_root_from(&opts.home);
    let target_root = opts.home.join(crate::skills_sync::TARGET_DIR);
    let removed = crate::skills_sync::apply::remove_synced(&root, &target_root, opts.dry_run);
    report.lines.extend(removed.lines);
    report.errors.extend(removed.errors);
}

/// The steps playbook never runs for the user, because they change Claude
/// Code's own plugin state. Only the ones that still apply are listed.
fn manual_steps(home: &Path, report: &mut Report) {
    use crate::common::plugin_state::{
        installed_in, listed_in, marketplace_known_in, HOOKS_PLUGIN_KEY, MARKETPLACE, PLUGIN_KEY,
    };
    let mut steps = Vec::new();
    if installed_in(home) == Some(true) {
        steps.push(format!("claude plugin uninstall {PLUGIN_KEY}"));
    }
    if listed_in(home, HOOKS_PLUGIN_KEY) == Some(true) {
        steps.push(format!("claude plugin uninstall {HOOKS_PLUGIN_KEY}"));
    }
    if marketplace_known_in(home) {
        steps.push(format!("claude plugin marketplace remove {MARKETPLACE}"));
    }
    if steps.is_empty() {
        return;
    }
    report.lines.push(
        "remaining manual steps (playbook does not change Claude Code's plugins):".to_string(),
    );
    report
        .lines
        .extend(steps.into_iter().map(|s| format!("  {s}")));
}

fn remove_claude_home_files(claude_home: &Path, verb: &str, dry_run: bool, report: &mut Report) {
    for name in CLAUDE_HOME_FILES {
        let path = claude_home.join(name);
        if path.is_file() {
            remove_file(&path, verb, dry_run, report);
        }
    }
}

/// How the `settings.json` step ended.
struct SettingsResult {
    /// False when the file could not be edited, so it may still name placed files.
    ok: bool,
    statusline_removed: bool,
}

fn settings_step(
    opts: &Options,
    claude_home: &Path,
    stamp: u64,
    report: &mut Report,
) -> SettingsResult {
    let done = |statusline_removed| SettingsResult {
        ok: true,
        statusline_removed,
    };
    let path = claude_home.join("settings.json");
    if !path.is_file() {
        return done(false);
    }
    let verb = if opts.dry_run {
        "would remove"
    } else {
        "removed"
    };
    match crate::init::telemetry::remove(&opts.home, &path, stamp, opts.dry_run) {
        Ok(out) if out.keys > 0 => {
            report.lines.push(format!(
                "{verb} {} telemetry variables that `playbook setup` wrote from {}",
                out.keys,
                path.display()
            ));
            if let Some(backup) = out.backup {
                report.lines.push(format!("backup: {}", backup.display()));
            }
        }
        Ok(_) => {}
        Err(err) => report
            .errors
            .push(format!("telemetry variables left unchanged: {err}")),
    }
    // Setup may have written it, but the user may also want it: say so, keep it.
    if crate::init::attribution::state(&path) == crate::init::attribution::State::Off {
        let how = match crate::init::attribution::held_off(&path).is_empty() {
            true => "delete the key by hand".to_string(),
            false => format!(
                "run `{}` while playbook is still installed, or delete the key by hand",
                crate::init::attribution::ON_COMMAND
            ),
        };
        report.lines.push(format!(
            "kept the attribution key in {}; to get Claude Code's attribution back, {how}",
            path.display()
        ));
    }
    match wire::unwire_at(&path, &opts.home, stamp, opts.dry_run) {
        Ok(out) if out.hooks_removed == 0 && !out.statusline_removed => done(false),
        Ok(out) => {
            let verb = if opts.dry_run {
                "would remove"
            } else {
                "removed"
            };
            let mut what = format!("{} hook entries", out.hooks_removed);
            if out.statusline_removed {
                what.push_str(" and the status line");
            }
            report
                .lines
                .push(format!("{verb} {what} from {}", path.display()));
            if let Some(backup) = out.backup_path {
                report.lines.push(format!("backup: {}", backup.display()));
            }
            done(out.statusline_removed)
        }
        Err(err) => {
            report
                .errors
                .push(format!("settings.json left unchanged: {err}"));
            SettingsResult {
                ok: false,
                statusline_removed: false,
            }
        }
    }
}

fn rc_step(opts: &Options, stamp: u64, report: &mut Report) {
    let mut changes = shim::strip_rc_files(&opts.home, stamp, opts.remove_binary, opts.dry_run);
    if opts.remove_binary {
        changes.extend(crate::init::path::strip(&opts.home, stamp, opts.dry_run));
    }
    for change in changes {
        let name = change.rc_file.display();
        if change.unwritable {
            report.errors.push(format!(
                "{name} is read-only; remove the playbook lines by hand"
            ));
        } else if let Some(err) = change.error {
            report.errors.push(format!("{name} left unchanged: {err}"));
        } else if let Some(backup) = change.backup {
            report.lines.push(format!(
                "removed the playbook lines from {name} (backup: {})",
                backup.display()
            ));
        } else {
            report
                .lines
                .push(format!("would remove the playbook lines from {name}"));
        }
    }
}

/// Binaries the installer puts in the bin directory, each with the
/// `<name>.<version>.bak` copies an update leaves beside it.
const BINARIES: [&str; 2] = ["playbook", "pbdoc"];

fn binary_step(opts: &Options, verb: &str, report: &mut Report) {
    for name in BINARIES {
        let binary = opts.bin_dir.join(name);
        if binary.is_file() {
            remove_file(&binary, verb, opts.dry_run, report);
        }
        if name == "playbook" {
            remove_pb_link(&opts.bin_dir, &binary, verb, opts.dry_run, report);
        }
        remove_backups(&opts.bin_dir, name, verb, opts.dry_run, report);
    }
}

fn remove_backups(bin_dir: &Path, name: &str, verb: &str, dry_run: bool, report: &mut Report) {
    let Ok(entries) = fs::read_dir(bin_dir) else {
        return;
    };
    let prefix = format!("{name}.");
    let mut backups: Vec<PathBuf> = entries
        .filter_map(Result::ok)
        .map(|e| e.path())
        .filter(|p| {
            p.file_name()
                .and_then(|n| n.to_str())
                .is_some_and(|n| n.starts_with(&prefix) && n.ends_with(".bak"))
        })
        .collect();
    backups.sort();
    for backup in backups {
        remove_file(&backup, verb, dry_run, report);
    }
}

/// The `pb` short name the installer links next to the binary. It goes only
/// when it is a symlink to `playbook` (relative or absolute), so a `pb` of the
/// user's own stays.
fn remove_pb_link(bin_dir: &Path, binary: &Path, verb: &str, dry_run: bool, report: &mut Report) {
    let link = bin_dir.join("pb");
    let Ok(target) = fs::read_link(&link) else {
        return;
    };
    if target == Path::new("playbook") || target == binary {
        remove_file(&link, verb, dry_run, report);
    }
}

fn remove_file(path: &Path, verb: &str, dry_run: bool, report: &mut Report) {
    let result = if dry_run {
        Ok(())
    } else {
        fs::remove_file(path)
    };
    settle(path, verb, dry_run, report, result);
}

fn remove_dir_all(path: &Path, verb: &str, dry_run: bool, report: &mut Report) {
    let result = if dry_run {
        Ok(())
    } else {
        fs::remove_dir_all(path)
    };
    settle(path, verb, dry_run, report, result);
}

fn settle(path: &Path, verb: &str, dry_run: bool, report: &mut Report, result: io::Result<()>) {
    if dry_run {
        report.lines.push(format!("{verb} {}", path.display()));
        return;
    }
    match result {
        Ok(()) => report.lines.push(format!("{verb} {}", path.display())),
        Err(err) => report
            .errors
            .push(format!("could not remove {}: {err}", path.display())),
    }
}

/// The install directory from `$PLAYBOOK_BIN_DIR`, else `~/.local/bin`. Setup
/// links `pb` only here, so uninstall finds every link setup made.
pub fn install_dir(home: &Path) -> PathBuf {
    default_bin_dir(home, std::env::var("PLAYBOOK_BIN_DIR").ok().as_deref())
}

/// The default install directory, overridable like the installer does.
pub fn default_bin_dir(home: &Path, env: Option<&str>) -> PathBuf {
    match env {
        Some(dir) if !dir.is_empty() => PathBuf::from(dir),
        _ => home.join(".local").join("bin"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn bin_dir_with(files: &[&str], tag: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("pb-uninstall-{tag}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        for name in files {
            fs::write(dir.join(name), "x").unwrap();
        }
        dir
    }

    fn opts(bin_dir: &Path, dry_run: bool) -> Options {
        Options {
            home: bin_dir.to_path_buf(),
            bin_dir: bin_dir.to_path_buf(),
            remove_binary: true,
            dry_run,
        }
    }

    const INSTALLED: [&str; 4] = ["playbook", "pbdoc", "pbdoc.0.1.0.bak", "playbook.0.1.0.bak"];

    #[test]
    fn dry_run_lists_pbdoc_and_its_backups_without_removing_them() {
        // Arrange
        let dir = bin_dir_with(&INSTALLED, "dry");
        let mut report = Report::default();

        // Act
        binary_step(&opts(&dir, true), "would remove", &mut report);

        // Assert
        let listed = report.lines.join("\n");
        assert!(listed.contains("would remove") && listed.contains("pbdoc"));
        assert!(listed.contains("pbdoc.0.1.0.bak"));
        assert!(dir.join("pbdoc").exists() && dir.join("pbdoc.0.1.0.bak").exists());
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn real_run_removes_pbdoc_and_its_backups_and_keeps_other_files() {
        // Arrange
        let mut files = INSTALLED.to_vec();
        files.push("pbdocs-notes");
        let dir = bin_dir_with(&files, "real");
        let mut report = Report::default();

        // Act
        binary_step(&opts(&dir, false), "removed", &mut report);

        // Assert
        for name in INSTALLED {
            assert!(!dir.join(name).exists(), "{name} should be gone");
        }
        assert!(dir.join("pbdocs-notes").exists());
        assert!(report.errors.is_empty());
        let _ = fs::remove_dir_all(&dir);
    }
}
