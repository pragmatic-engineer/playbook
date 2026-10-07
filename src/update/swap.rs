// SPDX-FileCopyrightText: 2026 Igor Santos
// SPDX-License-Identifier: MIT

//! Replacing the binary: smoke test the staged file, back up the old one
//! (`playbook.<old>.bak`, newest 3, same rule as install.sh), rename into
//! place, smoke test again, and restore the backup if that fails.

use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

const KEEP_BACKUPS: usize = 3;

/// What a successful swap left behind.
#[derive(Debug)]
pub struct Installed {
    pub backup: Option<PathBuf>,
    pub warnings: Vec<String>,
}

/// Runs `<bin> --version` and returns its stdout.
pub fn run_version(bin: &Path) -> Result<String, String> {
    let output = Command::new(bin)
        .arg("--version")
        .stdin(Stdio::null())
        .output()
        .map_err(|err| format!("{} did not run: {err}", bin.display()))?;
    if !output.status.success() {
        return Err(format!(
            "{} --version exited with {}",
            bin.display(),
            output.status
        ));
    }
    Ok(String::from_utf8_lossy(&output.stdout).trim().to_string())
}

#[cfg(unix)]
pub fn make_executable(path: &Path) -> std::io::Result<()> {
    use std::os::unix::fs::PermissionsExt;
    fs::set_permissions(path, fs::Permissions::from_mode(0o755))
}

#[cfg(not(unix))]
pub fn make_executable(_: &Path) -> std::io::Result<()> {
    Ok(())
}

fn smoke_ok(
    smoke: &dyn Fn(&Path) -> Result<String, String>,
    bin: &Path,
    version: &str,
) -> Result<(), String> {
    let out = smoke(bin)?;
    if out.contains(version) {
        Ok(())
    } else {
        Err(format!(
            "version mismatch: the binary reports '{out}', expected {version}"
        ))
    }
}

/// A version becomes part of a file name, so only a plain token is kept.
fn backup_token(old_version: &str) -> String {
    let plain = !old_version.is_empty()
        && old_version
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || "._+-".contains(c));
    if plain {
        old_version.to_string()
    } else {
        "unknown".to_string()
    }
}

fn prune_backups(dir: &Path, stem: &str) {
    let prefix = format!("{stem}.");
    let mut backups: Vec<_> = fs::read_dir(dir)
        .into_iter()
        .flatten()
        .flatten()
        .filter(|e| {
            let name = e.file_name().to_string_lossy().into_owned();
            name.starts_with(&prefix) && name.ends_with(".bak")
        })
        .filter_map(|e| Some((e.metadata().ok()?.modified().ok()?, e.path())))
        .collect();
    backups.sort_by_key(|b| std::cmp::Reverse(b.0));
    for (_, old) in backups.into_iter().skip(KEEP_BACKUPS) {
        let _ = fs::remove_file(old);
    }
}

/// Copies `target` to `playbook.<old>.bak` beside it unless that backup
/// exists or the bytes are unchanged. Not fatal to the caller.
fn backup(target: &Path, old_version: &str, new_bytes: &[u8]) -> Result<Option<PathBuf>, String> {
    let Ok(current) = fs::read(target) else {
        return Ok(None);
    };
    if current == new_bytes {
        return Ok(None);
    }
    let dir = target.parent().ok_or("binary has no parent directory")?;
    let stem = target
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or("playbook");
    let bak = dir.join(format!("{stem}.{}.bak", backup_token(old_version)));
    if bak.exists() {
        return Ok(Some(bak));
    }
    let tmp = dir.join(format!(".{stem}.bak.{}", std::process::id()));
    fs::write(&tmp, &current).map_err(|e| e.to_string())?;
    make_executable(&tmp).map_err(|e| e.to_string())?;
    fs::rename(&tmp, &bak).map_err(|e| {
        let _ = fs::remove_file(&tmp);
        e.to_string()
    })?;
    prune_backups(dir, stem);
    Ok(Some(bak))
}

/// Restores `bak` over `target` through a same-directory temp file.
fn restore(bak: &Path, target: &Path) -> Result<(), String> {
    let tmp = target.with_extension(format!("restore.{}", std::process::id()));
    fs::copy(bak, &tmp).map_err(|e| e.to_string())?;
    make_executable(&tmp).map_err(|e| {
        let _ = fs::remove_file(&tmp);
        e.to_string()
    })?;
    fs::rename(&tmp, target).map_err(|e| {
        let _ = fs::remove_file(&tmp);
        e.to_string()
    })
}

/// Writes `bytes` to a fresh owner-only file beside the binary. `create_new`
/// refuses an existing path, so a planted symlink is never followed.
pub fn write_stage(dir: &Path, bytes: &[u8]) -> std::io::Result<PathBuf> {
    use std::io::Write;
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.subsec_nanos())
        .unwrap_or(0);
    let stage = dir.join(format!(".playbook.update.{}.{nanos}", std::process::id()));
    let mut options = fs::OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o700);
    }
    let result = options.open(&stage).and_then(|mut f| f.write_all(bytes));
    if let Err(err) = result {
        let _ = fs::remove_file(&stage);
        return Err(err);
    }
    Ok(stage)
}

/// Installs the already-written `stage` file (same directory as `target`).
/// A bad staged binary leaves `target` untouched; a bad installed one is
/// rolled back from the backup.
pub fn install(
    stage: &Path,
    target: &Path,
    old_version: &str,
    new_version: &str,
    smoke: &dyn Fn(&Path) -> Result<String, String>,
) -> Result<Installed, String> {
    let result = install_staged(stage, target, old_version, new_version, smoke);
    if result.is_err() {
        let _ = fs::remove_file(stage);
    }
    result
}

fn install_staged(
    stage: &Path,
    target: &Path,
    old_version: &str,
    new_version: &str,
    smoke: &dyn Fn(&Path) -> Result<String, String>,
) -> Result<Installed, String> {
    make_executable(stage).map_err(|e| e.to_string())?;
    if let Err(err) = smoke_ok(smoke, stage, new_version) {
        return Err(format!(
            "the downloaded binary failed its smoke test, nothing was changed: {err}"
        ));
    }
    let mut warnings = Vec::new();
    let new_bytes = fs::read(stage).map_err(|e| e.to_string())?;
    let bak = match backup(target, old_version, &new_bytes) {
        Ok(bak) => bak,
        Err(err) => {
            warnings.push(format!(
                "could not back up the previous binary ({err}); continuing without one"
            ));
            None
        }
    };
    fs::rename(stage, target)
        .map_err(|e| format!("could not replace {}: {e}", target.display()))?;
    if let Err(err) = smoke_ok(smoke, target, new_version) {
        return Err(match bak {
            Some(bak) => match restore(&bak, target) {
                Ok(()) => format!("the installed binary failed its smoke test; rolled back to {}: {err}", bak.display()),
                Err(re) => format!("the installed binary failed its smoke test ({err}) and the rollback failed ({re}); restore it by hand: mv -f {} {}", bak.display(), target.display()),
            },
            None => format!("the installed binary failed its smoke test and there is no backup to restore: {err}"),
        });
    }
    Ok(Installed {
        backup: bak,
        warnings,
    })
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;
    use crate::common::test_support::scratch_dir;
    use std::cell::Cell;

    fn script(path: &Path, version: &str) {
        fs::write(path, format!("#!/bin/sh\necho 'playbook {version}'\n")).unwrap();
        make_executable(path).unwrap();
    }

    fn setup(tag: &str) -> (PathBuf, PathBuf, PathBuf) {
        let dir = scratch_dir(tag);
        fs::create_dir_all(&dir).unwrap();
        let target = dir.join("playbook");
        script(&target, "0.16.0");
        let stage = dir.join(".playbook.new");
        script(&stage, "0.17.0");
        (dir, target, stage)
    }

    #[test]
    fn a_good_binary_is_swapped_in_and_the_old_one_is_kept() {
        let (dir, target, stage) = setup("swap-ok");
        let done = install(&stage, &target, "0.16.0", "0.17.0", &run_version).unwrap();
        assert_eq!(run_version(&target).unwrap(), "playbook 0.17.0");
        let bak = done.backup.unwrap();
        assert_eq!(bak, dir.join("playbook.0.16.0.bak"));
        assert_eq!(run_version(&bak).unwrap(), "playbook 0.16.0");
        assert!(!stage.exists());
    }

    #[test]
    fn a_bad_staged_binary_aborts_and_leaves_the_original() {
        let (_dir, target, stage) = setup("swap-bad-stage");
        fs::write(&stage, "#!/bin/sh\nexit 1\n").unwrap();
        let err = install(&stage, &target, "0.16.0", "0.17.0", &run_version).unwrap_err();
        assert!(err.contains("nothing was changed"), "{err}");
        assert_eq!(run_version(&target).unwrap(), "playbook 0.16.0");
    }

    #[test]
    fn a_failed_post_swap_smoke_test_rolls_back() {
        let (_dir, target, stage) = setup("swap-rollback");
        let calls = Cell::new(0);
        let smoke = |bin: &Path| {
            calls.set(calls.get() + 1);
            if calls.get() == 1 {
                run_version(bin)
            } else {
                Err("boom".to_string())
            }
        };
        let err = install(&stage, &target, "0.16.0", "0.17.0", &smoke).unwrap_err();
        assert!(err.contains("rolled back"), "{err}");
        assert_eq!(run_version(&target).unwrap(), "playbook 0.16.0");
    }

    #[test]
    fn only_the_newest_three_backups_survive() {
        let (dir, target, stage) = setup("swap-prune");
        for (i, v) in ["0.10.0", "0.11.0", "0.12.0"].iter().enumerate() {
            let bak = dir.join(format!("playbook.{v}.bak"));
            script(&bak, v);
            let when =
                std::time::SystemTime::now() - std::time::Duration::from_secs(1000 - i as u64);
            fs::File::options()
                .write(true)
                .open(&bak)
                .unwrap()
                .set_modified(when)
                .unwrap();
        }
        install(&stage, &target, "0.16.0", "0.17.0", &run_version).unwrap();
        assert!(!dir.join("playbook.0.10.0.bak").exists());
        assert!(dir.join("playbook.0.11.0.bak").exists());
        assert!(dir.join("playbook.0.16.0.bak").exists());
    }

    #[test]
    fn an_odd_version_string_cannot_escape_the_file_name() {
        assert_eq!(backup_token("../evil"), "unknown");
        assert_eq!(backup_token("0.16.0-rc.1"), "0.16.0-rc.1");
    }
}
