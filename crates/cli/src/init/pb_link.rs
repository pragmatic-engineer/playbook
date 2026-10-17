// SPDX-FileCopyrightText: 2026 Igor Santos
// SPDX-License-Identifier: Apache-2.0

//! The `short-name` part of `playbook setup`: a `pb -> playbook` symlink in
//! the install folder, the same link `install.sh` makes (ADR 0044). It never
//! replaces a `pb` that is not this binary, and never writes outside the
//! folder `playbook uninstall --remove-binary` cleans.

use crate::init::run::StepReport;
use std::fs;
use std::path::{Path, PathBuf};

/// The config key that records the answer.
pub const KEY: &str = "launcher.shortName";

const PART: &str = "short-name";

/// What setup does with the part, decided from the flags and the answer.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Choice {
    /// Not asked and no flag: nothing changes and nothing is recorded.
    Skip,
    /// The user said no: the answer is recorded and no link is touched.
    Declined,
    /// Make sure `pb` runs this binary.
    Link,
}

/// Where the link may go.
#[derive(Debug, Clone)]
pub struct Target {
    /// The running binary.
    pub exe: PathBuf,
    /// The install folder `playbook uninstall` knows: `$PLAYBOOK_BIN_DIR`,
    /// else `~/.local/bin`.
    pub install_dir: PathBuf,
    /// The `PATH` to search for another `pb`.
    pub path_var: std::ffi::OsString,
}

/// Whether the user said no before, read from playbook's own config.
pub fn declined(home: &Path) -> bool {
    matches!(
        crate::config::resolve(KEY, home, None),
        Ok((
            serde_json::Value::Bool(false),
            crate::config::Source::Global
        ))
    )
}

/// Whether a yes is stored, so a later no leaves it alone (ADR 0044: a no
/// removes nothing).
fn stored_yes(home: &Path) -> bool {
    matches!(
        crate::config::resolve(KEY, home, None),
        Ok((serde_json::Value::Bool(true), _))
    )
}

fn record(home: &Path, yes: bool) -> Result<(), String> {
    crate::config::write::set(
        crate::config::write::Tier::Global,
        KEY,
        serde_json::Value::Bool(yes),
        home,
        None,
    )
    .map_err(|err| err.to_string())
}

/// The note a step adds when the answer could not be stored.
fn record_note(home: &Path, yes: bool) -> String {
    record(home, yes)
        .err()
        .map(|err| format!("; could not record the answer: {err}"))
        .unwrap_or_default()
}

/// What `plan` found: nothing to write, or the link to create.
enum Plan {
    Done(StepReport),
    Create(PathBuf),
}

fn same_file(a: &Path, b: &Path) -> bool {
    match (fs::canonicalize(a), fs::canonicalize(b)) {
        (Ok(a), Ok(b)) => a == b,
        _ => false,
    }
}

/// Run the part.
pub fn step(home: &Path, choice: Choice, target: Option<&Target>) -> StepReport {
    match choice {
        Choice::Skip => {
            return StepReport::skipped(PART, "not requested; pass --short-name to opt in")
        }
        Choice::Declined if stored_yes(home) => {
            return StepReport::skipped(
                PART,
                "you said no, so no `pb` link is added; launcher.shortName stays on, so an rc file that loads `playbook shell-init` keeps its `pb` function. To turn that off, run: playbook config set launcher.shortName false",
            );
        }
        Choice::Declined => {
            let note = record_note(home, false);
            return StepReport::skipped(
                PART,
                format!("you said no, so no `pb` link is added; to add it later, run: playbook setup --only short-name{note}"),
            );
        }
        Choice::Link => {}
    }
    let note = record_note(home, true);
    let Some(target) = target else {
        return StepReport::skipped(
            PART,
            format!(
                "the location of the playbook binary is unknown, so no `pb` link was added{note}"
            ),
        );
    };
    let mut report = link(target);
    report.detail.push_str(&note);
    report
}

fn link(target: &Target) -> StepReport {
    let link = match plan(target) {
        Plan::Done(done) => return done,
        Plan::Create(link) => link,
    };
    let on_path = std::env::split_paths(&target.path_var).any(|dir| dir == target.install_dir);
    let hint = if on_path {
        "so `pb cc` works".to_string()
    } else {
        format!("add {} to PATH to use it", target.install_dir.display())
    };
    match make_link(&link) {
        Ok(()) => StepReport::wired(
            PART,
            format!("linked {} to playbook; {hint}", link.display()),
        ),
        Err(err) => StepReport::failed(PART, format!("could not create {}: {err}", link.display())),
    }
}

/// What `--dry-run` shows for the part. Writes nothing.
pub fn preview(choice: Choice, target: Option<&Target>) -> String {
    match (choice, target) {
        (Choice::Skip, _) => "not requested".to_string(),
        (Choice::Declined, _) => "records your no; no link is touched".to_string(),
        (Choice::Link, None) => "the location of the playbook binary is unknown".to_string(),
        (Choice::Link, Some(target)) => match plan(target) {
            Plan::Done(report) => report.detail,
            Plan::Create(link) => format!("would link {} to playbook", link.display()),
        },
    }
}

/// The report when nothing needs writing, else the link to create.
fn plan(target: &Target) -> Plan {
    if let Some(found) = crate::init::path::command_on_path("pb", &target.path_var) {
        if same_file(&found, &target.exe) {
            return Plan::Done(StepReport::already_correct(
                PART,
                format!("`pb` at {} already runs playbook", found.display()),
            ));
        }
        return Plan::Done(StepReport::skipped(
            PART,
            format!(
                "`pb` is already a command at {}; left alone, use `playbook` in full",
                found.display()
            ),
        ));
    }
    let link = target.install_dir.join("pb");
    if fs::symlink_metadata(&link).is_ok() {
        if same_file(&link, &target.exe) {
            return Plan::Done(StepReport::already_correct(
                PART,
                format!(
                    "{} already runs playbook; add {} to PATH to use it",
                    link.display(),
                    target.install_dir.display()
                ),
            ));
        }
        return Plan::Done(StepReport::skipped(
            PART,
            format!("{} is not the playbook link; left alone", link.display()),
        ));
    }
    if !same_file(&target.install_dir.join("playbook"), &target.exe) {
        return Plan::Done(StepReport::skipped(
            PART,
            format!(
                "playbook is not installed in {}, so no link was added; to add one, run: ln -s {} <a folder on your PATH>/pb",
                target.install_dir.display(),
                target.exe.display()
            ),
        ));
    }
    Plan::Create(link)
}

#[cfg(unix)]
fn make_link(link: &Path) -> std::io::Result<()> {
    std::os::unix::fs::symlink("playbook", link)
}

#[cfg(not(unix))]
fn make_link(_link: &Path) -> std::io::Result<()> {
    Err(std::io::Error::new(
        std::io::ErrorKind::Unsupported,
        "symlinks are not supported here",
    ))
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;
    use crate::common::test_support::scratch_dir;
    use crate::init::run::StepStatus;
    use std::os::unix::fs::PermissionsExt;

    fn script(dir: &Path, name: &str) -> PathBuf {
        fs::create_dir_all(dir).unwrap();
        let path = dir.join(name);
        fs::write(&path, "#!/bin/sh\n").unwrap();
        fs::set_permissions(&path, fs::Permissions::from_mode(0o755)).unwrap();
        path
    }

    fn exe_in(dir: &Path) -> PathBuf {
        script(dir, "playbook")
    }

    #[test]
    fn a_foreign_pb_in_the_install_folder_is_left_alone() {
        let home = scratch_dir("pb-link-foreign");
        let bin = home.join(".local/bin");
        let exe = exe_in(&bin);
        let theirs = script(&bin, "pb");
        let target = Target {
            exe,
            install_dir: bin.clone(),
            path_var: Default::default(),
        };

        let got = step(&home, Choice::Link, Some(&target));

        assert_eq!(got.status, StepStatus::Skipped);
        assert!(
            got.detail.contains("not the playbook link"),
            "{}",
            got.detail
        );
        assert_eq!(fs::read_to_string(&theirs).unwrap(), "#!/bin/sh\n");
        assert!(fs::symlink_metadata(&theirs).unwrap().file_type().is_file());
    }

    #[test]
    fn a_no_keeps_a_stored_yes() {
        let home = scratch_dir("pb-link-keep-yes");
        fs::create_dir_all(&home).unwrap();
        record(&home, true).unwrap();

        let got = step(&home, Choice::Declined, None);

        assert_eq!(got.status, StepStatus::Skipped);
        assert!(
            got.detail.contains("config set launcher.shortName false"),
            "{}",
            got.detail
        );
        assert!(!declined(&home));
    }

    #[test]
    fn links_pb_in_the_install_folder_and_then_passes() {
        let home = scratch_dir("pb-link-home");
        let bin = home.join(".local/bin");
        let exe = exe_in(&bin);
        let target = Target {
            exe: exe.clone(),
            install_dir: bin.clone(),
            path_var: Default::default(),
        };

        let first = step(&home, Choice::Link, Some(&target));
        let second = step(&home, Choice::Link, Some(&target));

        assert_eq!(first.status, StepStatus::Wired, "{}", first.detail);
        assert_eq!(
            fs::read_link(bin.join("pb")).unwrap(),
            Path::new("playbook")
        );
        assert_eq!(
            second.status,
            StepStatus::AlreadyCorrect,
            "{}",
            second.detail
        );
        assert!(!declined(&home));
    }

    #[test]
    fn the_preview_names_the_link_and_writes_nothing() {
        let home = scratch_dir("pb-link-preview");
        let bin = home.join(".local/bin");
        let exe = exe_in(&bin);
        let target = Target {
            exe,
            install_dir: bin.clone(),
            path_var: Default::default(),
        };

        let text = preview(Choice::Link, Some(&target));

        assert!(text.starts_with("would link"), "{text}");
        assert!(fs::symlink_metadata(bin.join("pb")).is_err());
        assert!(!declined(&home));
    }

    #[test]
    fn another_pb_on_path_is_left_alone() {
        let home = scratch_dir("pb-link-taken");
        let bin = home.join(".local/bin");
        let exe = exe_in(&bin);
        let other = home.join("other");
        fs::create_dir_all(&other).unwrap();
        let theirs = other.join("pb");
        fs::write(&theirs, "#!/bin/sh\n").unwrap();
        fs::set_permissions(&theirs, fs::Permissions::from_mode(0o755)).unwrap();
        let target = Target {
            exe,
            install_dir: bin.clone(),
            path_var: other.clone().into_os_string(),
        };

        let got = step(&home, Choice::Link, Some(&target));

        assert_eq!(got.status, StepStatus::Skipped);
        assert!(got.detail.contains("already a command"), "{}", got.detail);
        assert!(fs::symlink_metadata(bin.join("pb")).is_err());
    }

    #[test]
    fn a_pb_on_path_that_runs_this_binary_passes() {
        let home = scratch_dir("pb-link-brew");
        let brew = home.join("brew/bin");
        let exe = exe_in(&brew);
        std::os::unix::fs::symlink("playbook", brew.join("pb")).unwrap();
        let target = Target {
            exe,
            install_dir: home.join(".local/bin"),
            path_var: brew.clone().into_os_string(),
        };

        let got = step(&home, Choice::Link, Some(&target));

        assert_eq!(got.status, StepStatus::AlreadyCorrect, "{}", got.detail);
        assert!(fs::symlink_metadata(home.join(".local/bin/pb")).is_err());
    }

    #[test]
    fn a_binary_outside_the_install_folder_gets_the_command_to_run() {
        let home = scratch_dir("pb-link-elsewhere");
        let exe = exe_in(&home.join("downloads"));
        let target = Target {
            exe,
            install_dir: home.join(".local/bin"),
            path_var: Default::default(),
        };

        let got = step(&home, Choice::Link, Some(&target));

        assert_eq!(got.status, StepStatus::Skipped);
        assert!(got.detail.contains("ln -s"), "{}", got.detail);
    }

    #[test]
    fn a_no_is_recorded_and_touches_nothing() {
        let home = scratch_dir("pb-link-no");
        let bin = home.join(".local/bin");
        let exe = exe_in(&bin);
        let target = Target {
            exe,
            install_dir: bin.clone(),
            path_var: Default::default(),
        };

        let got = step(&home, Choice::Declined, Some(&target));
        let skip = step(&home, Choice::Skip, Some(&target));

        assert_eq!(got.status, StepStatus::Skipped);
        assert!(declined(&home));
        assert_eq!(skip.status, StepStatus::Skipped);
        assert!(fs::symlink_metadata(bin.join("pb")).is_err());
    }
}
