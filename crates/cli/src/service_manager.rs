// SPDX-FileCopyrightText: 2026 Igor Santos
// SPDX-License-Identifier: Apache-2.0

//! Removes the files of the retired background service for the usage
//! collector (ADR 0043), for `playbook uninstall` and the
//! `collector-service-removed` migration. The files are found by
//! [`crate::usage::service`].
//!
//! Every call to `launchctl` or `systemctl` goes through [`Manager`]. Tests
//! pass a fake, and [`manager_for`] hands out the real one only when `home`
//! is the home of the account, so a run with a scratch `HOME` never loads a
//! real service.

use crate::usage::service::{find_old, Kind, LABEL, OLD_LABEL, UNIT};
use std::fs;
use std::path::{Path, PathBuf};

/// Runs the service manager's command line tool.
pub trait Manager {
    /// Runs `program` with `args` and returns what it printed. `Err` holds
    /// what it printed on a non-zero exit, or why it could not run.
    fn run(&self, program: &str, args: &[String]) -> Result<String, String>;
}

/// The real tools on this machine.
pub struct System;

impl Manager for System {
    fn run(&self, program: &str, args: &[String]) -> Result<String, String> {
        let out = std::process::Command::new(program)
            .args(args)
            .stdin(std::process::Stdio::null())
            .output()
            .map_err(|e| format!("could not run {program}: {e}"))?;
        if out.status.success() {
            return Ok(String::from_utf8_lossy(&out.stdout).into_owned());
        }
        let text = String::from_utf8_lossy(&out.stderr);
        let text = text.trim();
        Err(if text.is_empty() {
            format!("{program} {} exited with {}", args.join(" "), out.status)
        } else {
            format!("{program} {}: {text}", args.join(" "))
        })
    }
}

/// A manager that runs nothing and says why: `HOME` is not the home of the
/// account, so a real service would run against other files, or this is a
/// debug build that cargo started (`common::real_home`).
pub struct Refuses(pub &'static str);

impl Manager for Refuses {
    fn run(&self, program: &str, _: &[String]) -> Result<String, String> {
        Err(format!("{program} not run, because {}", self.0))
    }
}

/// Why the real tools may not run for `home`, or `None` when they may.
pub fn refusal(home: &Path) -> Option<&'static str> {
    use crate::common::real_home;
    if !real_home::is_account_home(home) {
        Some("HOME is not the home of your account")
    } else if real_home::refused(home) {
        Some(real_home::NOTE)
    } else {
        None
    }
}

/// The real manager when [`refusal`] allows it, else one that runs nothing.
pub fn manager_for(home: &Path) -> Box<dyn Manager> {
    match refusal(home) {
        None => Box::new(System),
        Some(why) => Box::new(Refuses(why)),
    }
}

fn words(list: &[&str]) -> Vec<String> {
    list.iter().map(|s| (*s).to_string()).collect()
}

/// The `launchctl bootout` command for the agent `label`.
fn bootout_command(label: &str, uid: u32) -> (&'static str, Vec<String>) {
    (
        "launchctl",
        vec!["bootout".into(), format!("gui/{uid}/{label}")],
    )
}

/// The commands that stop the service and keep it from starting again.
fn unload_commands(kind: Kind, uid: u32) -> Vec<(&'static str, Vec<String>)> {
    match kind {
        Kind::Launchd => vec![bootout_command(LABEL, uid)],
        Kind::Systemd => vec![("systemctl", words(&["--user", "disable", "--now", UNIT]))],
    }
}

/// `Err` with the note when this build may not change the real home
/// (`common::real_home`), so files there change only where launchctl may
/// run.
fn off_limits(home: &Path) -> Result<(), String> {
    match crate::common::real_home::refused(home) {
        true => Err(crate::common::real_home::NOTE.to_string()),
        false => Ok(()),
    }
}

/// Stops the service and deletes its file. `Ok(false)` when there was no file.
pub fn remove(kind: Kind, home: &Path, manager: &dyn Manager, uid: u32) -> Result<bool, String> {
    let file = kind.file(home);
    if !file.exists() {
        return Ok(false);
    }
    off_limits(home)?;
    for (program, args) in unload_commands(kind, uid) {
        // Not loaded is fine: the file goes either way.
        let _ = manager.run(program, &args);
    }
    fs::remove_file(&file).map_err(|e| format!("could not remove {}: {e}", file.display()))?;
    if kind == Kind::Systemd {
        let _ = manager.run("systemctl", &words(&["--user", "daemon-reload"]));
    }
    Ok(true)
}

/// Stops the agent playbook wrote under [`OLD_LABEL`] and deletes its file.
/// `Ok(None)` when there is no such file, or it is not one playbook wrote,
/// which stays.
pub fn remove_old(home: &Path, manager: &dyn Manager, uid: u32) -> Result<Option<PathBuf>, String> {
    let Some(old) = find_old(home).filter(|o| o.ours) else {
        return Ok(None);
    };
    off_limits(home)?;
    let (program, args) = bootout_command(OLD_LABEL, uid);
    // Not loaded is fine: the file goes either way.
    let _ = manager.run(program, &args);
    fs::remove_file(&old.file)
        .map_err(|e| format!("could not remove {}: {e}", old.file.display()))?;
    Ok(Some(old.file))
}

/// A fake launchd for the tests of this module and of the `collector-service-removed` migration.
#[cfg(test)]
pub(crate) mod fake {
    use super::Manager;
    use std::cell::RefCell;

    /// launchd as a set of loaded labels. `bootout` unloads one and fails
    /// for one not loaded, as launchctl does. Every call is recorded.
    #[derive(Default)]
    pub(crate) struct Launchd {
        pub(crate) loaded: RefCell<Vec<String>>,
        pub(crate) calls: RefCell<Vec<String>>,
    }

    impl Launchd {
        pub(crate) fn with(labels: &[&str]) -> Launchd {
            Launchd {
                loaded: RefCell::new(labels.iter().map(|l| (*l).to_string()).collect()),
                ..Launchd::default()
            }
        }

        pub(crate) fn has(&self, label: &str) -> bool {
            self.loaded.borrow().iter().any(|l| l == label)
        }
    }

    impl Manager for Launchd {
        fn run(&self, program: &str, args: &[String]) -> Result<String, String> {
            self.calls
                .borrow_mut()
                .push(format!("{program} {}", args.join(" ")));
            if args.first().map(String::as_str) != Some("bootout") {
                return Ok(String::new());
            }
            let gone = args[1].rsplit('/').next().unwrap_or_default().to_string();
            let had = self.has(&gone);
            self.loaded.borrow_mut().retain(|l| *l != gone);
            match had {
                true => Ok(String::new()),
                false => Err("No such process".into()),
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::fake::Launchd;
    use super::*;
    use crate::common::test_support::scratch_dir;
    use crate::usage::service::{log_path, old_file, render, render_old};

    const PROGRAM: &str = "/opt/homebrew/bin/playbook";
    const UID: u32 = 7;

    /// Writes the file the retired service wrote for `kind` under `home`.
    fn put(kind: Kind, home: &Path) {
        let file = kind.file(home);
        fs::create_dir_all(file.parent().unwrap()).unwrap();
        fs::write(&file, render(kind, Path::new(PROGRAM), &log_path(home))).unwrap();
    }

    /// A scratch home with the agent 0.37.0 wrote.
    fn old_home(tag: &str) -> PathBuf {
        let home = scratch_dir(tag);
        let file = old_file(&home);
        fs::create_dir_all(file.parent().unwrap()).unwrap();
        fs::write(&file, render_old(Path::new(PROGRAM), &log_path(&home))).unwrap();
        home
    }

    #[test]
    fn remove_stops_the_service_and_deletes_only_its_file() {
        let home = scratch_dir("service-remove");
        put(Kind::Systemd, &home);
        let log = log_path(&home);
        fs::create_dir_all(log.parent().unwrap()).unwrap();
        fs::write(&log, "").unwrap();
        let fake = Launchd::default();

        let removed = remove(Kind::Systemd, &home, &fake, UID).unwrap();
        let again = remove(Kind::Systemd, &home, &fake, UID).unwrap();

        assert!(removed && !again);
        assert!(!Kind::Systemd.file(&home).exists());
        assert!(log_path(&home).exists(), "the log stays");
        assert_eq!(
            *fake.calls.borrow(),
            [
                format!("systemctl --user disable --now {UNIT}"),
                "systemctl --user daemon-reload".to_string(),
            ]
        );
    }

    #[test]
    fn a_scratch_home_never_gets_the_real_manager() {
        let home = scratch_dir("service-scratch");
        fs::create_dir_all(&home).unwrap();

        let err = manager_for(&home).run("launchctl", &[]).unwrap_err();

        assert!(err.contains("not the home of your account"), "{err}");
    }

    #[test]
    fn a_file_playbook_did_not_write_is_left_alone() {
        let home = scratch_dir("service-remove-foreign");
        let file = old_file(&home);
        fs::create_dir_all(file.parent().unwrap()).unwrap();
        fs::write(&file, "<plist>mine</plist>").unwrap();
        let fake = Launchd::with(&[OLD_LABEL]);

        let removed = remove_old(&home, &fake, UID).unwrap();

        assert_eq!(removed, None);
        assert!(file.is_file());
        assert!(fake.calls.borrow().is_empty());
    }

    #[test]
    fn the_real_home_net_stops_every_file_change() {
        let home = old_home("service-remove-net");
        put(Kind::Launchd, &home);
        let _pretend = crate::common::real_home::pretend_account_home(&home);
        let fake = Launchd::with(&[OLD_LABEL, LABEL]);

        let removed_old = remove_old(&home, &fake, UID);
        let removed = remove(Kind::Launchd, &home, &fake, UID);

        if std::env::var_os(crate::common::real_home::ALLOW_ENV).is_none() {
            for err in [removed_old.unwrap_err(), removed.unwrap_err()] {
                assert!(err.contains("PLAYBOOK_ALLOW_REAL_HOME"), "{err}");
            }
            assert!(old_file(&home).is_file() && Kind::Launchd.file(&home).is_file());
            assert!(fake.calls.borrow().is_empty());
        }
    }

    #[test]
    fn remove_old_boots_out_and_deletes_only_the_old_file() {
        let home = old_home("service-remove-old");
        put(Kind::Launchd, &home);
        let fake = Launchd::with(&[OLD_LABEL, LABEL]);

        let removed = remove_old(&home, &fake, UID).unwrap();
        let again = remove_old(&home, &fake, UID).unwrap();

        assert_eq!(removed, Some(old_file(&home)));
        assert_eq!(again, None);
        assert!(!fake.has(OLD_LABEL));
        assert!(Kind::Launchd.file(&home).is_file(), "the new agent stays");
        assert!(fake.has(LABEL), "and stays loaded");
    }
}
