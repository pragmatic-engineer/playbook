// SPDX-FileCopyrightText: 2026 Igor Santos
// SPDX-License-Identifier: Apache-2.0

//! Pre-trusts a directory in `~/.claude.json` so Claude Code's first-launch
//! trust dialog never blocks `ccc`/`ccd`. The write swaps a tmp file onto the
//! RESOLVED target, so a symlinked file stays a symlink and a concurrent
//! reader never sees a torn file.

use crate::common::atomic::{acquire_dir_lock, remove_stale_lock_dir, STALE_LOCK_AGE};
use serde_json::{Map, Value};
use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::time::Duration;

const LOCK_RETRIES: u32 = 50;
const LOCK_DELAY: Duration = Duration::from_millis(10);
/// How often the merge restarts when the file changes under it before the
/// swap, so a concurrent writer's change is never overwritten with old data.
const MERGE_ATTEMPTS: u32 = 3;

/// CLI entry: trusts `path` in the real `~/.claude.json`. Never returns an
/// error, so a trust failure can never block the launch that called it.
pub fn run(path: &str) -> Result<(), String> {
    run_with_home(&crate::common::home_dir(), path)
}

/// Same as `run` with an explicit home, so tests never read the real `$HOME`.
pub fn run_with_home(home: &Path, path: &str) -> Result<(), String> {
    if home.as_os_str().is_empty() {
        return Ok(());
    }
    if !Path::new(path).is_absolute() {
        eprintln!("playbook trust: ignoring {path:?}: the path must be absolute");
        return Ok(());
    }
    if let Err(message) = write_trust_entry(&home.join(".claude.json"), path) {
        eprintln!("playbook trust: {message}");
    }
    Ok(())
}

/// Marks `project_path` as trusted in the file at `claude_json_path`.
/// A missing file is a silent no-op; this never originates Claude's state.
pub(crate) fn write_trust_entry(claude_json_path: &Path, project_path: &str) -> Result<(), String> {
    let Ok(real_path) = fs::canonicalize(claude_json_path) else {
        return Ok(());
    };
    let lock_path = lock_path_for(&real_path);
    remove_stale_lock_dir(&lock_path, STALE_LOCK_AGE);
    if !acquire_dir_lock(&lock_path, LOCK_RETRIES, LOCK_DELAY) {
        return Err(format!(
            "could not take the write lock beside {}; skipped rather than risk overwriting a concurrent change",
            real_path.display()
        ));
    }
    let result = merge_and_swap(&real_path, project_path, &mut || {});
    let _ = fs::remove_dir(&lock_path);
    result
}

fn lock_path_for(real_path: &Path) -> PathBuf {
    let name = real_path
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_default();
    real_path.with_file_name(format!("{name}.lock"))
}

/// Reads, merges, and swaps. `before_swap` runs after the merge is computed
/// and before the file is compared again, so a test can play the concurrent
/// writer; production passes a no-op.
fn merge_and_swap(
    real_path: &Path,
    project_path: &str,
    before_swap: &mut dyn FnMut(),
) -> Result<(), String> {
    for _ in 0..MERGE_ATTEMPTS {
        let original = fs::read_to_string(real_path)
            .map_err(|e| format!("could not read {}: {e}", real_path.display()))?;
        let mut root: Value = serde_json::from_str(&original)
            .map_err(|e| format!("could not parse {}: {e}", real_path.display()))?;

        if is_already_trusted(&root, project_path) {
            return Ok(());
        }
        set_trusted(&mut root, project_path)?;

        let mut contents = serde_json::to_string_pretty(&root)
            .map_err(|e| format!("could not serialize the updated document: {e}"))?;
        if original.ends_with('\n') {
            contents.push('\n');
        }
        before_swap();
        if fs::read_to_string(real_path).is_ok_and(|now| now == original) {
            return swap_in(real_path, &contents);
        }
    }
    Err(format!(
        "{} kept changing while it was being updated; skipped",
        real_path.display()
    ))
}

/// Whether `claude_json_path` already trusts `project_path`. A missing or
/// unreadable file reads as not trusted.
pub(crate) fn is_trusted(claude_json_path: &Path, project_path: &str) -> bool {
    fs::read_to_string(claude_json_path)
        .ok()
        .and_then(|text| serde_json::from_str::<Value>(&text).ok())
        .is_some_and(|root| is_already_trusted(&root, project_path))
}

fn is_already_trusted(root: &Value, project_path: &str) -> bool {
    root.get("projects")
        .and_then(|projects| projects.get(project_path))
        .and_then(|entry| entry.get("hasTrustDialogAccepted"))
        .and_then(Value::as_bool)
        == Some(true)
}

fn set_trusted(root: &mut Value, project_path: &str) -> Result<(), String> {
    let top = root
        .as_object_mut()
        .ok_or("the top-level value is not a JSON object")?;
    let projects = top
        .entry("projects")
        .or_insert_with(|| Value::Object(Map::new()))
        .as_object_mut()
        .ok_or("`projects` is not a JSON object")?;
    let entry = projects
        .entry(project_path)
        .or_insert_with(|| Value::Object(Map::new()))
        .as_object_mut()
        .ok_or("the project entry is not a JSON object")?;
    entry.insert("hasTrustDialogAccepted".to_string(), Value::Bool(true));
    Ok(())
}

/// Creates the tmp file exclusively and owner-only, so it never follows a
/// planted symlink and the OAuth data is never readable by others. A leftover
/// at the same name is removed first (`remove_file` unlinks a symlink itself).
fn create_private_tmp(path: &Path) -> std::io::Result<fs::File> {
    let mut options = fs::OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    match options.open(path) {
        Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => {
            fs::remove_file(path)?;
            options.open(path)
        }
        other => other,
    }
}

/// Writes `contents` to a tmp file beside `real_path`, then renames it over
/// `real_path`. Same directory keeps the rename atomic; the original mode is
/// set on the open handle before any data is written.
fn swap_in(real_path: &Path, contents: &str) -> Result<(), String> {
    let parent = real_path
        .parent()
        .ok_or_else(|| format!("{} has no parent directory", real_path.display()))?;
    let tmp_path = parent.join(format!(
        ".trust-tmp-{}-{:?}",
        std::process::id(),
        std::thread::current().id()
    ));
    let write_error = |e: std::io::Error| format!("could not write {}: {e}", tmp_path.display());

    let mut tmp_file = create_private_tmp(&tmp_path).map_err(write_error)?;
    let staged = fs::metadata(real_path)
        .and_then(|meta| tmp_file.set_permissions(meta.permissions()))
        .and_then(|()| tmp_file.write_all(contents.as_bytes()))
        .and_then(|()| tmp_file.sync_all());
    if let Err(e) = staged {
        let _ = fs::remove_file(&tmp_path);
        return Err(write_error(e));
    }
    if let Err(e) = fs::rename(&tmp_path, real_path) {
        let _ = fs::remove_file(&tmp_path);
        return Err(format!("could not replace {}: {e}", real_path.display()));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::common::test_support::scratch_dir;
    use serde_json::{json, Value};
    use std::fs;

    fn scratch(tag: &str) -> std::path::PathBuf {
        let dir = scratch_dir(tag);
        fs::create_dir_all(&dir).expect("scratch dir is creatable");
        dir
    }

    fn read_json(path: &Path) -> Value {
        serde_json::from_str(&fs::read_to_string(path).expect("file is readable"))
            .expect("file holds valid json")
    }

    #[test]
    fn missing_file_is_a_silent_noop_and_is_not_created() {
        // Arrange
        let dir = scratch("trust-missing");
        let file = dir.join(".claude.json");

        // Act
        let got = write_trust_entry(&file, "/work/a");

        // Assert
        assert_eq!(got, Ok(()));
        assert!(!file.exists(), "the file must never be originated");
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn empty_object_gains_exactly_the_trusted_project_entry() {
        // Arrange
        let dir = scratch("trust-empty");
        let file = dir.join(".claude.json");
        fs::write(&file, "{}").expect("fixture is writable");

        // Act
        let got = write_trust_entry(&file, "/work/a");

        // Assert
        assert_eq!(got, Ok(()));
        assert_eq!(
            read_json(&file),
            json!({"projects": {"/work/a": {"hasTrustDialogAccepted": true}}})
        );
        let _ = fs::remove_dir_all(&dir);
    }

    #[cfg(unix)]
    #[test]
    fn already_trusted_entry_skips_the_write_entirely() {
        use std::os::unix::fs::MetadataExt;

        // Arrange
        let dir = scratch("trust-short-circuit");
        let file = dir.join(".claude.json");
        fs::write(
            &file,
            r#"{"projects":{"/work/a":{"hasTrustDialogAccepted":true}}}"#,
        )
        .expect("fixture is writable");
        let before = fs::metadata(&file).expect("fixture has metadata");
        std::thread::sleep(std::time::Duration::from_millis(30));

        // Act
        let got = write_trust_entry(&file, "/work/a");

        // Assert
        let after = fs::metadata(&file).expect("file still has metadata");
        assert_eq!(got, Ok(()));
        assert_eq!(before.ino(), after.ino(), "a write would swap the inode");
        assert_eq!(before.modified().ok(), after.modified().ok());
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn untrusted_entry_flips_and_keeps_sibling_fields() {
        // Arrange
        let dir = scratch("trust-siblings");
        let file = dir.join(".claude.json");
        fs::write(
            &file,
            r#"{"projects":{"/work/a":{"hasTrustDialogAccepted":false,"lastCost":1.23}}}"#,
        )
        .expect("fixture is writable");

        // Act
        let got = write_trust_entry(&file, "/work/a");

        // Assert
        assert_eq!(got, Ok(()));
        assert_eq!(
            read_json(&file),
            json!({"projects": {"/work/a": {"hasTrustDialogAccepted": true, "lastCost": 1.23}}})
        );
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn unrelated_project_entry_and_top_level_keys_survive_unchanged() {
        // Arrange
        let dir = scratch("trust-unrelated");
        let file = dir.join(".claude.json");
        let original = json!({
            "numStartups": 7,
            "projects": {"/other": {"hasTrustDialogAccepted": false, "tools": ["a", "b"]}}
        });
        fs::write(&file, original.to_string()).expect("fixture is writable");

        // Act
        let got = write_trust_entry(&file, "/work/new");

        // Assert
        let after = read_json(&file);
        assert_eq!(got, Ok(()));
        assert_eq!(after["numStartups"], original["numStartups"]);
        assert_eq!(after["projects"]["/other"], original["projects"]["/other"]);
        assert_eq!(
            after["projects"]["/work/new"],
            json!({"hasTrustDialogAccepted": true})
        );
        let _ = fs::remove_dir_all(&dir);
    }

    #[cfg(unix)]
    #[test]
    fn symlinked_file_stays_a_symlink_and_the_target_gets_the_write() {
        use std::os::unix::fs::{symlink, MetadataExt};

        // Arrange
        let dir = scratch("trust-symlink");
        let real_dir = dir.join("dotfiles");
        fs::create_dir_all(&real_dir).expect("real dir is creatable");
        let target = real_dir.join("claude.json");
        fs::write(&target, "{}").expect("target is writable");
        let link = dir.join(".claude.json");
        symlink(&target, &link).expect("symlink is creatable");
        let link_target_before = fs::read_link(&link).expect("link is readable");
        let link_ino_before = fs::symlink_metadata(&link)
            .expect("link has metadata")
            .ino();
        let ino_before = fs::metadata(&target).expect("target has metadata").ino();

        // Act
        let got = write_trust_entry(&link, "/work/a");

        // Assert
        assert_eq!(got, Ok(()));
        assert_eq!(
            fs::read_link(&link).expect("still a link"),
            link_target_before
        );
        assert!(fs::symlink_metadata(&link)
            .expect("link has metadata")
            .file_type()
            .is_symlink());
        let link_ino_after = fs::symlink_metadata(&link)
            .expect("link has metadata")
            .ino();
        assert_eq!(
            link_ino_before, link_ino_after,
            "the symlink itself must never be replaced"
        );
        let after = fs::metadata(&target).expect("target has metadata");
        assert_ne!(
            ino_before,
            after.ino(),
            "the rename swaps a fresh inode onto the resolved target"
        );
        assert_eq!(
            read_json(&target),
            json!({"projects": {"/work/a": {"hasTrustDialogAccepted": true}}})
        );
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn malformed_json_is_an_error_and_leaves_the_file_untouched() {
        // Arrange
        let dir = scratch("trust-malformed");
        let file = dir.join(".claude.json");
        fs::write(&file, "{ not json").expect("fixture is writable");

        // Act
        let got = write_trust_entry(&file, "/work/a");

        // Assert
        let message = got.expect_err("malformed json must be an error");
        assert!(message.contains("parse"), "got: {message}");
        assert_eq!(fs::read_to_string(&file).expect("readable"), "{ not json");
        let _ = fs::remove_dir_all(&dir);
    }

    #[cfg(unix)]
    #[test]
    fn unwritable_parent_dir_is_an_error_and_leaves_the_file_untouched() {
        use std::os::unix::fs::PermissionsExt;

        // Arrange
        let dir = scratch("trust-readonly");
        let file = dir.join(".claude.json");
        fs::write(&file, "{}").expect("fixture is writable");
        fs::set_permissions(&dir, fs::Permissions::from_mode(0o555)).expect("chmod works");

        // Act
        let got = write_trust_entry(&file, "/work/a");

        // Assert
        fs::set_permissions(&dir, fs::Permissions::from_mode(0o755)).expect("restore works");
        let message = got.expect_err("an unwritable directory must be an error");
        assert!(message.contains("write"), "got: {message}");
        assert_eq!(fs::read_to_string(&file).expect("readable"), "{}");
        let _ = fs::remove_dir_all(&dir);
    }

    #[cfg(unix)]
    #[test]
    fn file_permissions_survive_the_swap() {
        use std::os::unix::fs::PermissionsExt;

        // Arrange
        let dir = scratch("trust-mode");
        let file = dir.join(".claude.json");
        fs::write(&file, "{}").expect("fixture is writable");
        // 0o640 differs from both a 022 and a 077 umask default, so dropping
        // the mode copy fails this test whatever the umask is.
        fs::set_permissions(&file, fs::Permissions::from_mode(0o640)).expect("chmod works");

        // Act
        let got = write_trust_entry(&file, "/work/a");

        // Assert
        let mode = fs::metadata(&file).expect("metadata").permissions().mode() & 0o777;
        assert_eq!(got, Ok(()));
        assert_eq!(mode, 0o640, "the original mode must survive the swap");
        let _ = fs::remove_dir_all(&dir);
    }

    #[cfg(unix)]
    #[test]
    fn the_tmp_file_never_follows_a_planted_symlink() {
        // Arrange: a symlink at the predictable tmp name pointing at a victim.
        let dir = scratch("trust-planted-link");
        let file = dir.join(".claude.json");
        let victim = dir.join("victim.txt");
        fs::write(&file, "{}").expect("fixture is writable");
        fs::write(&victim, "keep me").expect("victim is writable");
        let tmp_name = format!(
            ".trust-tmp-{}-{:?}",
            std::process::id(),
            std::thread::current().id()
        );
        std::os::unix::fs::symlink(&victim, dir.join(&tmp_name)).expect("symlink is creatable");

        // Act
        let got = write_trust_entry(&file, "/work/a");

        // Assert
        assert_eq!(got, Ok(()));
        assert_eq!(fs::read_to_string(&victim).unwrap(), "keep me");
        assert_eq!(
            read_json(&file),
            json!({"projects": {"/work/a": {"hasTrustDialogAccepted": true}}})
        );
        let _ = fs::remove_dir_all(&dir);
    }

    #[cfg(unix)]
    #[test]
    fn the_tmp_file_is_owner_only_while_it_holds_the_data() {
        use std::os::unix::fs::PermissionsExt;

        // Arrange
        let dir = scratch("trust-tmp-mode");
        let tmp = dir.join("tmp-file");

        // Act
        let handle = create_private_tmp(&tmp).expect("tmp is creatable");

        // Assert
        let mode = handle.metadata().unwrap().permissions().mode() & 0o777;
        assert_eq!(
            mode & 0o077,
            0,
            "group and other must have no access: {mode:o}"
        );
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_lock_that_cannot_be_taken_skips_the_write() {
        // Arrange: a fresh lock directory held by someone else.
        let dir = scratch("trust-lock-held");
        let file = dir.join(".claude.json");
        fs::write(&file, "{}").expect("fixture is writable");
        fs::create_dir(dir.join(".claude.json.lock")).expect("lock dir is creatable");

        // Act
        let got = write_trust_entry(&file, "/work/a");

        // Assert
        assert!(got.is_err(), "{got:?}");
        assert_eq!(read_json(&file), json!({}));
        let _ = fs::remove_dir_all(&dir);
    }

    #[cfg(unix)]
    #[test]
    fn a_stale_lock_directory_does_not_block_the_write() {
        // Arrange: a lock left behind by a killed holder, well past the limit.
        let dir = scratch("trust-lock-stale");
        let file = dir.join(".claude.json");
        let lock = dir.join(".claude.json.lock");
        fs::write(&file, "{}").expect("fixture is writable");
        fs::create_dir(&lock).expect("lock dir is creatable");
        let past = std::time::SystemTime::now() - crate::common::atomic::STALE_LOCK_AGE * 3;
        fs::File::open(&lock)
            .and_then(|h| h.set_modified(past))
            .expect("mtime is settable");
        let started = std::time::Instant::now();

        // Act
        let got = write_trust_entry(&file, "/work/a");

        // Assert
        assert_eq!(got, Ok(()));
        assert!(
            started.elapsed() < Duration::from_millis(400),
            "{:?}",
            started.elapsed()
        );
        assert!(!lock.exists(), "the lock must be released afterward");
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_change_made_during_the_merge_is_kept_not_overwritten() {
        // Arrange: another writer adds a project after our merge is computed.
        let dir = scratch("trust-concurrent");
        let file = dir.join(".claude.json");
        fs::write(&file, r#"{"projects":{}}"#).expect("fixture is writable");
        let mut interfered = false;
        let mut other_writer = || {
            if !interfered {
                interfered = true;
                fs::write(&file, r#"{"projects":{"/other":{"seen":1}}}"#).expect("writable");
            }
        };

        // Act
        let got = merge_and_swap(&file, "/work/a", &mut other_writer);

        // Assert
        assert_eq!(got, Ok(()));
        assert_eq!(
            read_json(&file),
            json!({"projects": {
                "/other": {"seen": 1},
                "/work/a": {"hasTrustDialogAccepted": true}
            }})
        );
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_file_that_never_stops_changing_is_skipped_with_an_error() {
        // Arrange: the other writer changes the file on every attempt.
        let dir = scratch("trust-churn");
        let file = dir.join(".claude.json");
        fs::write(&file, "{}").expect("fixture is writable");
        let mut counter = 0;
        let mut churn = || {
            counter += 1;
            fs::write(&file, format!(r#"{{"n":{counter}}}"#)).expect("writable");
        };

        // Act
        let got = merge_and_swap(&file, "/work/a", &mut churn);

        // Assert
        assert!(got.is_err(), "{got:?}");
        assert!(!fs::read_to_string(&file)
            .unwrap()
            .contains("hasTrustDialogAccepted"));
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_relative_or_home_relative_path_is_ignored_and_writes_nothing() {
        // Arrange
        let home = scratch("trust-run-relative");
        fs::write(home.join(".claude.json"), "{}").expect("fixture is writable");

        for bad in ["relative/dir", "~", "~/work", "."] {
            // Act
            let got = run_with_home(&home, bad);

            // Assert
            assert_eq!(got, Ok(()), "{bad}");
            assert_eq!(read_json(&home.join(".claude.json")), json!({}), "{bad}");
        }
        let _ = fs::remove_dir_all(&home);
    }

    #[test]
    fn run_with_home_is_ok_and_silent_when_no_claude_json_exists() {
        // Arrange
        let home = scratch("trust-run-missing");

        // Act
        let got = run_with_home(&home, "/work/a");

        // Assert
        assert_eq!(got, Ok(()));
        assert!(!home.join(".claude.json").exists());
        let _ = fs::remove_dir_all(&home);
    }

    #[test]
    fn run_with_home_never_errors_on_malformed_json() {
        // Arrange
        let home = scratch("trust-run-malformed");
        fs::write(home.join(".claude.json"), "{ not json").expect("fixture is writable");

        // Act
        let got = run_with_home(&home, "/work/a");

        // Assert
        assert_eq!(got, Ok(()));
        assert_eq!(
            fs::read_to_string(home.join(".claude.json")).expect("readable"),
            "{ not json"
        );
        let _ = fs::remove_dir_all(&home);
    }

    #[test]
    fn run_with_home_writes_the_entry_for_a_valid_file() {
        // Arrange
        let home = scratch("trust-run-valid");
        fs::write(home.join(".claude.json"), "{}").expect("fixture is writable");

        // Act
        let got = run_with_home(&home, "/work/a");

        // Assert
        assert_eq!(got, Ok(()));
        assert_eq!(
            read_json(&home.join(".claude.json")),
            json!({"projects": {"/work/a": {"hasTrustDialogAccepted": true}}})
        );
        let _ = fs::remove_dir_all(&home);
    }

    #[cfg(unix)]
    #[test]
    fn run_with_home_never_errors_when_the_file_cannot_be_written() {
        use std::os::unix::fs::PermissionsExt;

        // Arrange
        let home = scratch("trust-run-readonly");
        fs::write(home.join(".claude.json"), "{}").expect("fixture is writable");
        fs::set_permissions(&home, fs::Permissions::from_mode(0o555)).expect("chmod works");

        // Act
        let got = run_with_home(&home, "/work/a");

        // Assert
        fs::set_permissions(&home, fs::Permissions::from_mode(0o755)).expect("restore works");
        assert_eq!(got, Ok(()));
        assert_eq!(
            fs::read_to_string(home.join(".claude.json")).expect("readable"),
            "{}"
        );
        let _ = fs::remove_dir_all(&home);
    }

    #[test]
    fn run_with_home_ignores_an_empty_project_path() {
        // Arrange
        let home = scratch("trust-run-empty-path");
        fs::write(home.join(".claude.json"), "{}").expect("fixture is writable");

        // Act
        let got = run_with_home(&home, "");

        // Assert
        assert_eq!(got, Ok(()));
        assert_eq!(read_json(&home.join(".claude.json")), json!({}));
        let _ = fs::remove_dir_all(&home);
    }

    #[test]
    fn run_with_home_ignores_an_unresolved_home() {
        // Act
        let got = run_with_home(Path::new(""), "/work/a");

        // Assert
        assert_eq!(got, Ok(()));
    }

    #[test]
    fn a_concurrent_reader_never_observes_a_torn_file() {
        use std::sync::atomic::{AtomicBool, Ordering};
        use std::sync::Arc;

        // Arrange
        let dir = scratch("trust-concurrent");
        let file = dir.join(".claude.json");
        let mut projects = serde_json::Map::new();
        for i in 0..10000 {
            projects.insert(
                format!("/seed/{i}"),
                json!({"hasTrustDialogAccepted": true}),
            );
        }
        fs::write(&file, json!({"projects": projects}).to_string()).expect("fixture is writable");
        let stop = Arc::new(AtomicBool::new(false));
        let reader_stop = Arc::clone(&stop);
        let reader_file = file.clone();
        let reader = std::thread::spawn(move || {
            let mut bad_reads = 0u32;
            while !reader_stop.load(Ordering::Relaxed) {
                let text = fs::read_to_string(&reader_file).unwrap_or_default();
                if serde_json::from_str::<Value>(&text).is_err() {
                    bad_reads += 1;
                }
            }
            bad_reads
        });

        // Act
        for i in 0..60 {
            write_trust_entry(&file, &format!("/work/{i}")).expect("write succeeds");
        }
        stop.store(true, Ordering::Relaxed);
        let bad_reads = reader.join().expect("reader thread finishes");

        // Assert
        assert_eq!(bad_reads, 0, "a reader saw an empty or partial file");
        assert!(read_json(&file)["projects"]["/work/59"]["hasTrustDialogAccepted"] == true);
        let _ = fs::remove_dir_all(&dir);
    }
}
