// SPDX-FileCopyrightText: 2026 Igor Santos
// SPDX-License-Identifier: Apache-2.0

//! Helpers for tests that run the repo's real shell scripts (`install.sh`,
//! `bin/playbook`) against stubbed `curl`, `gh`, `uname` and `claude`.

#![allow(dead_code)]

use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Output, Stdio};
use std::sync::atomic::{AtomicU64, Ordering};

static COUNTER: AtomicU64 = AtomicU64::new(0);

/// The repository root.
pub fn repo_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
}

/// A scratch directory removed on drop.
pub struct Work(pub PathBuf);

impl Work {
    pub fn new(tag: &str) -> Work {
        let n = COUNTER.fetch_add(1, Ordering::Relaxed);
        let dir = std::env::temp_dir().join(format!("pb-shell-{tag}-{}-{n}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).expect("scratch dir");
        Work(dir)
    }

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

/// Write `content` to `path` and make it executable.
pub fn write_exec(path: &Path, content: &str) {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).expect("parent dir");
    }
    fs::write(path, content).expect("write script");
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(path, fs::Permissions::from_mode(0o755)).expect("chmod");
    }
}

/// The tracked files of HEAD extracted into `dest`, so untracked build output
/// cannot leak into a run. `None` when this is not a git checkout.
pub fn archive_head(dest: &Path) -> Option<()> {
    let root = repo_root();
    let inside = Command::new("git")
        .args(["-C"])
        .arg(&root)
        .args(["rev-parse", "--git-dir"])
        .output()
        .ok()?;
    if !inside.status.success() {
        return None;
    }
    fs::create_dir_all(dest).ok()?;
    let archive = Command::new("git")
        .arg("-C")
        .arg(&root)
        .args(["archive", "HEAD"])
        .stdout(Stdio::piped())
        .spawn()
        .ok()?;
    let status = Command::new("tar")
        .arg("-x")
        .arg("-C")
        .arg(dest)
        .stdin(archive.stdout?)
        .status()
        .ok()?;
    status.success().then_some(())
}

/// Run `bash <script> <args>` with `envs`, capturing both streams.
pub fn bash(script: &Path, args: &[&str], envs: &[(&str, String)]) -> Output {
    let mut cmd = Command::new("bash");
    cmd.arg(script).args(args);
    for (k, v) in envs {
        cmd.env(k, v);
    }
    cmd.output().expect("bash should spawn")
}

pub fn stdout(out: &Output) -> String {
    String::from_utf8_lossy(&out.stdout).into_owned()
}

pub fn stderr(out: &Output) -> String {
    String::from_utf8_lossy(&out.stderr).into_owned()
}
