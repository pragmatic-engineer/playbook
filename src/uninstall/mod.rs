// SPDX-FileCopyrightText: 2026 Igor Santos
// SPDX-License-Identifier: Apache-2.0

//! `playbook uninstall`: the inverse of `playbook init` and `install.sh`.
//!
//! It removes only what playbook placed: the hook entries and status line
//! `init` wrote into `settings.json`, the managed launcher block in the rc
//! files, the files `init` copies under `~/.config/playbook`, and the two
//! installer scripts in `~/.claude`. The binary and its rc `PATH` block go
//! only on request. Memory, settings the user owns, sessions, history and
//! earlier backups are never touched.

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
    if !settings.ok {
        report.lines.push(
            "kept the placed files and the binary, since settings.json still refers to them"
                .to_string(),
        );
        remove_claude_home_files(&claude_home, verb, opts.dry_run, &mut report);
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

    if opts.remove_binary {
        binary_step(opts, verb, &mut report);
    }
    report
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

fn binary_step(opts: &Options, verb: &str, report: &mut Report) {
    let binary = opts.bin_dir.join("playbook");
    if binary.is_file() {
        remove_file(&binary, verb, opts.dry_run, report);
    }
    let Ok(entries) = fs::read_dir(&opts.bin_dir) else {
        return;
    };
    let mut backups: Vec<PathBuf> = entries
        .filter_map(Result::ok)
        .map(|e| e.path())
        .filter(|p| {
            p.file_name()
                .and_then(|n| n.to_str())
                .is_some_and(|n| n.starts_with("playbook.") && n.ends_with(".bak"))
        })
        .collect();
    backups.sort();
    for backup in backups {
        remove_file(&backup, verb, opts.dry_run, report);
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

/// The default install directory, overridable like the installer does.
pub fn default_bin_dir(home: &Path, env: Option<&str>) -> PathBuf {
    match env {
        Some(dir) if !dir.is_empty() => PathBuf::from(dir),
        _ => home.join(".local").join("bin"),
    }
}
