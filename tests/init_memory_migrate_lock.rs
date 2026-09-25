// SPDX-FileCopyrightText: 2026 Igor Santos
// SPDX-License-Identifier: MIT

//! Alone in its own integration test binary so that setting
//! `PLAYBOOK_TEST_COPY_DELAY_MS` here can never race a sibling test in
//! `tests/init_memory_migrate.rs` reading the same process-wide env var:
//! cargo compiles and runs each integration test file as a separate process,
//! so there is no shared address space or shared env to race.

use playbook::init::memory_migrate::migrate_memory;
use playbook::init::run::StepStatus;
use std::env;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::time::Duration;

static SCRATCH_COUNTER: AtomicU64 = AtomicU64::new(0);

/// A fresh scratch directory standing in for `$HOME`, unique per call.
fn scratch_home(tag: &str) -> PathBuf {
    let n = SCRATCH_COUNTER.fetch_add(1, Ordering::Relaxed);
    let home = env::temp_dir().join(format!(
        "playbook-init-memory-migrate-lock-{}-{tag}-{n}",
        std::process::id()
    ));
    fs::create_dir_all(&home).expect("scratch home should be creatable");
    home
}

fn claude_home_of(home: &Path) -> PathBuf {
    home.join(".claude")
}

fn mem_dir_of(claude_home: &Path) -> PathBuf {
    claude_home.join("memory")
}

fn new_root_of(home: &Path) -> PathBuf {
    home.join(".config").join("playbook").join("memory")
}

#[test]
fn lock_directory_is_held_for_the_duration_of_an_in_flight_copy() {
    // Arrange: at least two files, and an empty pre-existing new root so the
    // move takes the per-file copy path (the fast rename never takes a
    // lock). PLAYBOOK_TEST_COPY_DELAY_MS makes the total copy time bounded
    // and predictable: file count times delay.
    let home = scratch_home("lock-held-during-copy");
    let claude_home = claude_home_of(&home);
    let old_root = mem_dir_of(&claude_home);
    fs::create_dir_all(&old_root).unwrap();
    fs::write(old_root.join("fact-a.md"), "fact a content").unwrap();
    fs::write(old_root.join("fact-b.md"), "fact b content").unwrap();
    let new_root = new_root_of(&home);
    fs::create_dir_all(&new_root).unwrap();
    let lock_path = new_root.join("memory.graph.json.lock");

    let delay_ms: u64 = 50;
    env::set_var("PLAYBOOK_TEST_COPY_DELAY_MS", delay_ms.to_string());

    let observed = std::sync::Arc::new(AtomicBool::new(false));
    let poller = {
        let observed = observed.clone();
        let lock_path = lock_path.clone();
        // A retry loop, not a single check, so the poll window reliably
        // overlaps the in-flight copy despite scheduling jitter: roughly
        // 10x the expected total copy time (2 files * 50ms), 5ms apart.
        std::thread::spawn(move || {
            for _ in 0..200 {
                if lock_path.is_dir() {
                    observed.store(true, Ordering::Relaxed);
                    break;
                }
                std::thread::sleep(Duration::from_millis(5));
            }
        })
    };

    // Act
    let report = migrate_memory(&home, &claude_home);
    poller.join().unwrap();
    env::remove_var("PLAYBOOK_TEST_COPY_DELAY_MS");

    // Assert
    assert_eq!(report.status, StepStatus::Wired, "{}", report.detail);
    assert!(
        observed.load(Ordering::Relaxed),
        "the poller never observed the lock directory during the in-flight copy"
    );
    assert!(
        !lock_path.is_dir(),
        "the lock directory must be removed once migration completes"
    );

    let _ = fs::remove_dir_all(&home);
}
