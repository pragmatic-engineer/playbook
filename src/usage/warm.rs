// SPDX-FileCopyrightText: 2026 Igor Santos
// SPDX-License-Identifier: Apache-2.0

//! Warms the usage store in the background so `playbook usage` opens with
//! data. The `usage-warm` hook calls `maybe_spawn`, which returns in
//! milliseconds: it checks the config, a stamp and a lock, then starts a
//! detached `playbook usage warm` and does not wait for it.
//!
//! The child (`run_warm`) takes a lock so one ingest runs at a time, lowers
//! its priority, runs the same resume-safe batched ingest the usage view
//! uses, and stops after a time limit. A run that finished writes the stamp,
//! and a later hook skips while the stamp is younger than
//! `usage.autoIngestMinutes`. A run that hit the limit leaves no stamp, and
//! its committed batches stay, so the next run resumes. It reads transcripts
//! only as the ingest already does, writes nothing to them, prints nothing and
//! logs no content.

use super::run::{ingest_into, Paths};
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime};

const STAMP: &str = "auto-ingest.stamp";
const LOCK: &str = "auto-ingest.lock";
/// A lock older than this is from a run that died.
const LOCK_STALE: Duration = Duration::from_secs(20 * 60);
/// The most a warm run works before it stops and lets the next one resume.
const TIME_LIMIT: Duration = Duration::from_secs(120);

fn age(path: &Path, now: SystemTime) -> Option<Duration> {
    let modified = std::fs::metadata(path).ok()?.modified().ok()?;
    Some(now.duration_since(modified).unwrap_or_default())
}

/// Whether a warm run should start: the last finished run is at least
/// `minutes` ago and no live run holds the lock.
pub fn due(dir: &Path, minutes: u64, now: SystemTime) -> bool {
    if age(&dir.join(LOCK), now).is_some_and(|a| a < LOCK_STALE) {
        return false;
    }
    match age(&dir.join(STAMP), now) {
        Some(a) => a >= Duration::from_secs(minutes.saturating_mul(60)),
        None => true,
    }
}

/// The lock of a running warm, released on drop.
pub struct Lock(PathBuf);

impl Drop for Lock {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.0);
    }
}

/// Takes the lock, replacing a stale one. `None` when a live run holds it.
pub fn try_lock(dir: &Path, now: SystemTime) -> Option<Lock> {
    let path = dir.join(LOCK);
    for _ in 0..2 {
        match std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&path)
        {
            Ok(_) => return Some(Lock(path)),
            Err(_) if age(&path, now).is_some_and(|a| a < LOCK_STALE) => return None,
            Err(_) => {
                let _ = std::fs::remove_file(&path);
            }
        }
    }
    None
}

fn touch_stamp(dir: &Path) {
    let _ = std::fs::write(dir.join(STAMP), b"");
}

#[cfg(unix)]
fn lower_priority() {
    // SAFETY: setpriority on the calling process takes plain integers.
    unsafe {
        libc::setpriority(libc::PRIO_PROCESS, 0, 19);
    }
}

#[cfg(not(unix))]
fn lower_priority() {}

/// The warm run: lock, low priority, batched ingest within the time limit,
/// stamp when it finished. Returns quietly when another run holds the lock.
pub fn run_warm(paths: &Paths) -> Result<(), String> {
    let dir = paths
        .db
        .parent()
        .ok_or_else(|| "no usage directory".to_string())?;
    std::fs::create_dir_all(dir).map_err(|e| e.to_string())?;
    let Some(_lock) = try_lock(dir, SystemTime::now()) else {
        return Ok(());
    };
    lower_priority();
    let conn = super::db::open_db(&paths.db)?;
    let started = std::time::Instant::now();
    let mut stopped = false;
    ingest_into(paths, &conn, &mut |_, _| {
        let go = started.elapsed() < TIME_LIMIT;
        stopped |= !go;
        go
    })?;
    if !stopped {
        touch_stamp(dir);
    }
    Ok(())
}

/// What the hook does: starts a detached warm run when the config allows it
/// and one is due. Never waits for it, never prints.
pub fn maybe_spawn(home: &Path, paths: &Paths, exe: &Path) {
    if !enabled(home) {
        return;
    }
    let Some(dir) = paths.db.parent() else {
        return;
    };
    if !due(dir, minutes(home), SystemTime::now()) {
        return;
    }
    let mut cmd = std::process::Command::new(exe);
    cmd.args(["usage", "warm"])
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null());
    #[cfg(unix)]
    std::os::unix::process::CommandExt::process_group(&mut cmd, 0);
    let _ = cmd.spawn();
}

fn value(home: &Path, key: &str) -> Option<serde_json::Value> {
    crate::config::resolve_valid(key, home, None)
        .ok()
        .map(|(v, _, _)| v)
}

/// `usage.autoIngest`, on unless the config says false.
pub fn enabled(home: &Path) -> bool {
    value(home, "usage.autoIngest").and_then(|v| v.as_bool()) != Some(false)
}

/// `usage.autoIngestMinutes`, 10 when unset.
pub fn minutes(home: &Path) -> u64 {
    value(home, "usage.autoIngestMinutes")
        .and_then(|v| v.as_u64())
        .unwrap_or(10)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::usage::util::scratch_dir;

    fn dir(name: &str) -> PathBuf {
        let d = scratch_dir(name);
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    fn aged(path: &Path, secs: u64) {
        let f = std::fs::File::create(path).unwrap();
        f.set_modified(SystemTime::now() - Duration::from_secs(secs))
            .unwrap();
    }

    fn config(home: &Path, json: &str) {
        let cfg = home.join(".config/playbook");
        std::fs::create_dir_all(&cfg).unwrap();
        std::fs::write(cfg.join("config.json"), json).unwrap();
    }

    #[test]
    fn with_no_stamp_a_run_is_due() {
        assert!(due(&dir("warm-nostamp"), 10, SystemTime::now()));
    }

    #[test]
    fn a_run_finished_under_the_limit_ago_is_not_due() {
        let d = dir("warm-recent");
        aged(&d.join(STAMP), 5 * 60);
        assert!(!due(&d, 10, SystemTime::now()));
        aged(&d.join(STAMP), 11 * 60);
        assert!(due(&d, 10, SystemTime::now()));
    }

    #[test]
    fn zero_minutes_means_every_start() {
        let d = dir("warm-zero");
        aged(&d.join(STAMP), 1);
        assert!(due(&d, 0, SystemTime::now()));
    }

    #[test]
    fn a_live_lock_blocks_and_a_stale_lock_does_not() {
        let d = dir("warm-lock");
        aged(&d.join(LOCK), 60);
        assert!(!due(&d, 0, SystemTime::now()));
        assert!(try_lock(&d, SystemTime::now()).is_none());
        aged(&d.join(LOCK), 60 * 60);
        assert!(due(&d, 0, SystemTime::now()));
        assert!(try_lock(&d, SystemTime::now()).is_some());
    }

    #[test]
    fn only_one_holder_at_a_time_and_the_lock_is_released_on_drop() {
        let d = dir("warm-one");
        let first = try_lock(&d, SystemTime::now()).unwrap();
        assert!(try_lock(&d, SystemTime::now()).is_none());
        drop(first);
        assert!(try_lock(&d, SystemTime::now()).is_some());
    }

    #[test]
    fn a_finished_run_writes_the_stamp_and_frees_the_lock() {
        let home = dir("warm-run");
        let paths = Paths::from_home(&home);
        run_warm(&paths).unwrap();
        let usage = paths.db.parent().unwrap();
        assert!(usage.join(STAMP).is_file());
        assert!(!usage.join(LOCK).exists());
        assert!(paths.db.is_file());
    }

    #[test]
    fn a_run_that_finds_the_lock_taken_does_nothing() {
        let home = dir("warm-busy");
        let paths = Paths::from_home(&home);
        let usage = paths.db.parent().unwrap().to_path_buf();
        std::fs::create_dir_all(&usage).unwrap();
        let _held = try_lock(&usage, SystemTime::now()).unwrap();
        run_warm(&paths).unwrap();
        assert!(!usage.join(STAMP).exists());
    }

    #[test]
    fn the_defaults_are_on_and_ten_minutes() {
        let home = dir("warm-defaults");
        assert!(enabled(&home));
        assert_eq!(minutes(&home), 10);
    }

    #[test]
    fn the_config_keys_turn_it_off_and_set_the_minutes() {
        let home = dir("warm-config");
        config(
            &home,
            r#"{"usage":{"autoIngest":false,"autoIngestMinutes":3}}"#,
        );
        assert!(!enabled(&home));
        assert_eq!(minutes(&home), 3);
    }

    #[test]
    fn off_in_the_config_spawns_nothing() {
        let home = dir("warm-off");
        config(&home, r#"{"usage":{"autoIngest":false}}"#);
        let paths = Paths::from_home(&home);
        let marker = home.join("ran");
        let script = home.join("fake");
        std::fs::write(
            &script,
            format!("#!/bin/sh\ntouch '{}'\n", marker.display()),
        )
        .unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&script, std::fs::Permissions::from_mode(0o755)).unwrap();
        }
        maybe_spawn(&home, &paths, &script);
        std::thread::sleep(Duration::from_millis(300));
        assert!(!marker.exists());
    }
}
