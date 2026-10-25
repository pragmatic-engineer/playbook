// SPDX-FileCopyrightText: 2026 Igor Santos
// SPDX-License-Identifier: Apache-2.0

//! The scratch directory for tests that run the repo's real shell script
//! (`install.sh`). The other helpers are in `shell_exec.rs`, `shell_run.rs`,
//! `shell_bin_dir.rs` and `install_stubs.rs`, so a test file includes only
//! what it uses.

use std::fs;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};

static COUNTER: AtomicU64 = AtomicU64::new(0);

/// A scratch directory removed on drop.
pub struct Work(pub PathBuf);

impl Work {
    pub fn new(tag: &str) -> Work {
        let n = COUNTER.fetch_add(1, Ordering::Relaxed);
        let dir = std::env::temp_dir().join(format!(
            "pb-shell-{tag}-{}-{n}",
            playbook::testing::run_id()
        ));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).expect("scratch dir");
        Work(dir)
    }

    // Not every install test builds paths under the scratch dir.
    #[allow(dead_code)]
    pub fn path(&self, rel: &str) -> PathBuf {
        self.0.join(rel)
    }

    /// A new empty directory under the scratch dir.
    pub fn dir(&self, rel: &str) -> PathBuf {
        let p = self.0.join(rel);
        fs::create_dir_all(&p).expect("subdir");
        p
    }
}

impl Drop for Work {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}
