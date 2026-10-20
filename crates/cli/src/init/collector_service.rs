// SPDX-FileCopyrightText: 2026 Igor Santos
// SPDX-License-Identifier: Apache-2.0

//! Removes what the retired `collector-service` part of `playbook setup`
//! (ADR 0043) left: the service files and the stored answers. `playbook
//! uninstall` and the `collector-service-removed` migration call it. The
//! files are found by [`crate::usage::service`].

use crate::common::account::uid;
use crate::service_manager::{self as manager, Manager};
use crate::usage::service::{self, Kind};
use std::path::{Path, PathBuf};

/// The state key that recorded a no from the retired setup part. Only
/// [`forget_answers`] and [`declined`] still name it.
pub(crate) const DECLINED_KEY: &str = "collector-service/declined";

/// The state key the retired `collector-service` migration wrote after its
/// one reminder. Only `forget_answers` still names it.
pub(crate) const REMINDED_KEY: &str = "collector-service/reminded";

fn root(home: &Path) -> PathBuf {
    crate::common::paths::playbook_root_from(home)
}

fn has_key(home: &Path, key: &str) -> bool {
    crate::state::list_read_only(&root(home), key).is_ok_and(|rows| !rows.is_empty())
}

/// Whether the user said no, read without creating or changing the store.
pub fn declined(home: &Path) -> bool {
    has_key(home, DECLINED_KEY)
}

/// A file at one of playbook's service paths that playbook did not
/// write, which playbook never touches itself.
#[derive(Debug, PartialEq, Eq)]
pub struct Foreign {
    /// The service and the path, as for a file playbook wrote.
    pub describe: String,
    /// The commands that stop the service and remove the file.
    pub fix: String,
}

impl Foreign {
    /// One sentence for `playbook migrate` and `playbook doctor`.
    pub fn sentence(&self) -> String {
        format!("{} was not written by playbook, so it stays", self.describe)
    }

    fn new(describe: String, unload: String, file: &Path) -> Self {
        let fix = format!(
            "{unload}; rm {}",
            crate::common::shell::word(&file.to_string_lossy())
        );
        Foreign { describe, fix }
    }
}

/// The files at playbook's service paths under `home` that playbook did not
/// write. It reads only the files and runs nothing.
pub fn foreign_files(home: &Path) -> Vec<Foreign> {
    let kinds = KINDS
        .iter()
        .filter_map(|kind| service::find_kind(*kind, home))
        .filter(|found| !found.ours)
        .map(|found| {
            let unload = match found.kind {
                Kind::Launchd => format!("launchctl bootout gui/{}/{}", uid(), found.kind.id()),
                Kind::Systemd => format!("systemctl --user disable --now {}", found.kind.id()),
            };
            Foreign::new(found.describe(), unload, &found.file)
        });
    let old = service::find_old(home).filter(|o| !o.ours).map(|old| {
        let unload = format!("launchctl bootout gui/{}/{}", uid(), service::OLD_LABEL);
        Foreign::new(old.describe(), unload, &old.file)
    });
    kinds.chain(old).collect()
}

/// The service managers playbook ever wrote a file for. Removal checks each
/// one on any system, so a file a synced home carried over goes too.
const KINDS: [Kind; 2] = [Kind::Launchd, Kind::Systemd];

/// The service files playbook wrote under `home`, in words, or `None` when
/// there are none. It reads only the files and runs nothing.
pub fn installed_names(home: &Path) -> Option<String> {
    let names: Vec<String> = KINDS
        .iter()
        .filter_map(|kind| service::find_kind(*kind, home))
        .filter(|found| found.ours)
        .map(|found| found.describe())
        .chain(
            service::find_old(home)
                .filter(|o| o.ours)
                .map(|o| o.describe()),
        )
        .collect();
    (!names.is_empty()).then(|| names.join(" and the "))
}

/// The step `playbook uninstall` and the `collector-service-removed`
/// migration run: stops and deletes every service file playbook wrote,
/// including the agent under the label 0.37.0 used. `Ok(None)` when there
/// is none.
pub fn remove(home: &Path, dry_run: bool) -> Result<Option<String>, String> {
    if dry_run {
        return Ok(installed_names(home).map(|names| format!("would stop and remove the {names}")));
    }
    let manager = manager::manager_for(home);
    remove_with(home, manager.as_ref())
}

/// [`remove`] with the service manager passed in, so tests use a fake one.
pub fn remove_with(home: &Path, manager: &dyn Manager) -> Result<Option<String>, String> {
    let Some(names) = installed_names(home) else {
        return Ok(None);
    };
    for kind in KINDS {
        manager::remove(kind, home, manager, uid())?;
    }
    manager::remove_old(home, manager, uid())?;
    Ok(Some(format!("stopped and removed the {names}")))
}

/// Deletes the answers the retired setup part stored. A key that stays is
/// read by nothing that changes behavior, so a failure is dropped and the
/// next run of the migration tries again.
pub fn forget_answers(home: &Path) {
    for key in [DECLINED_KEY, REMINDED_KEY] {
        if has_key(home, key) {
            let _ = crate::state::delete(&root(home), key);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::common::test_support::scratch_dir;
    use crate::service_manager::fake::Launchd;
    use std::fs;

    const PROGRAM: &str = "/p";

    /// Writes the file the retired service wrote for `kind` into `home`.
    fn put(kind: Kind, home: &Path) {
        let file = kind.file(home);
        fs::create_dir_all(file.parent().unwrap()).unwrap();
        let log = service::log_path(home);
        fs::write(&file, service::render(kind, Path::new(PROGRAM), &log)).unwrap();
    }

    #[test]
    fn a_foreign_or_non_utf8_file_is_kept_and_reported() {
        let home = scratch_dir("collector-service-foreign");
        let own = Kind::Launchd.file(&home);
        let theirs = Kind::Systemd.file(&home);
        put(Kind::Launchd, &home);
        fs::create_dir_all(theirs.parent().unwrap()).unwrap();
        fs::write(&theirs, [0xff, 0xfe, 0x00]).unwrap();

        let done = remove_with(&home, &Launchd::default()).unwrap().unwrap();
        let foreign = foreign_files(&home);

        assert!(!own.exists(), "playbook's own file goes");
        assert!(theirs.is_file(), "the foreign file stays");
        assert!(!done.contains("systemd"), "{done}");
        assert_eq!(foreign.len(), 1);
        assert!(foreign[0].describe.starts_with("systemd user unit "));
        assert!(foreign[0].fix.contains("disable --now") && foreign[0].fix.contains("rm "));
        assert_eq!(installed_names(&home), None);
    }

    /// Writes the agent 0.37.0 wrote into `home`.
    fn put_old(home: &Path) {
        let file = service::old_file(home);
        fs::create_dir_all(file.parent().unwrap()).unwrap();
        let log = service::log_path(home);
        fs::write(&file, service::render_old(Path::new(PROGRAM), &log)).unwrap();
    }

    #[test]
    fn a_dry_run_only_names_the_files_and_a_removal_deletes_them() {
        let home = scratch_dir("collector-service-remove");
        put(Kind::Systemd, &home);

        let dry = remove(&home, true).unwrap().unwrap();
        let still_there = Kind::Systemd.file(&home).exists();
        let done = remove_with(&home, &Launchd::default()).unwrap().unwrap();

        assert!(dry.starts_with("would stop and remove"), "{dry}");
        assert!(still_there);
        assert!(done.starts_with("stopped and removed"), "{done}");
        assert!(!Kind::Systemd.file(&home).exists());
        assert_eq!(remove_with(&home, &Launchd::default()).unwrap(), None);
    }

    #[test]
    fn removal_takes_both_agents_and_leaves_a_foreign_old_file() {
        let both = scratch_dir("collector-service-remove-both");
        put(Kind::Launchd, &both);
        put_old(&both);
        let foreign = scratch_dir("collector-service-remove-foreign");
        let file = service::old_file(&foreign);
        fs::create_dir_all(file.parent().unwrap()).unwrap();
        fs::write(&file, "<plist/>").unwrap();
        let fake = Launchd::with(&[service::LABEL, service::OLD_LABEL]);

        let done = remove_with(&both, &fake).unwrap().unwrap();
        let none = remove_with(&foreign, &fake).unwrap();

        assert!(
            done.contains(service::LABEL) && done.contains(service::OLD_LABEL),
            "{done}"
        );
        assert!(!Kind::Launchd.file(&both).exists() && !service::old_file(&both).exists());
        assert!(fake.loaded.borrow().is_empty(), "{:?}", fake.calls.borrow());
        assert_eq!(none, None);
        assert!(file.is_file(), "a file playbook did not write stays");
    }

    #[test]
    fn removal_keeps_a_service_file_that_is_not_text() {
        let home = scratch_dir("collector-service-remove-binary");
        let file = Kind::Systemd.file(&home);
        fs::create_dir_all(file.parent().unwrap()).unwrap();
        fs::write(&file, [0xff_u8, 0xfe, 0x00]).unwrap();

        let done = remove_with(&home, &Launchd::default()).unwrap();

        assert_eq!(done, None);
        assert!(file.is_file());
    }
}
