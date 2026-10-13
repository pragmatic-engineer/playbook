// SPDX-FileCopyrightText: 2026 Igor Santos
// SPDX-License-Identifier: Apache-2.0

//! The private scratch directory a conversion writes to. The directory is
//! created with mode 0700 and a random name, and removed when the guard is
//! dropped, which covers success, error, timeout and a panic. A startup sweep
//! removes directories a killed process left behind.

use sha2::{Digest, Sha256};
use std::fs;
use std::io::Read;
use std::os::unix::fs::{DirBuilderExt, MetadataExt, PermissionsExt};
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime};

const PREFIX: &str = "doc-";

/// A directory that is deleted, with everything in it, on drop.
pub struct TempDir {
    path: PathBuf,
}

impl TempDir {
    /// Create a fresh private directory under `root`. `root` itself is made
    /// 0700 and must be a real directory, never a symlink.
    pub fn create(root: &Path) -> std::io::Result<Self> {
        fs::DirBuilder::new()
            .recursive(true)
            .mode(0o700)
            .create(root)?;
        let meta = fs::symlink_metadata(root)?;
        if !meta.is_dir() {
            return Err(std::io::Error::other("scratch root is not a directory"));
        }
        // SAFETY: `geteuid` takes no arguments and cannot fail.
        if meta.uid() != unsafe { libc::geteuid() } {
            return Err(std::io::Error::other("scratch root is not owned by you"));
        }
        fs::set_permissions(root, fs::Permissions::from_mode(0o700))?;
        for _ in 0..8 {
            let path = root.join(format!("{PREFIX}{}", random_hex(16)));
            match fs::DirBuilder::new().mode(0o700).create(&path) {
                Ok(()) => return Ok(Self { path }),
                Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => continue,
                Err(e) => return Err(e),
            }
        }
        Err(std::io::Error::other(
            "could not create a scratch directory",
        ))
    }

    pub fn path(&self) -> &Path {
        &self.path
    }
}

impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.path);
    }
}

/// `len` random hex characters, from the OS random source, with a hashed
/// fallback when it cannot be read.
pub fn random_hex(len: usize) -> String {
    let mut bytes = vec![0u8; len.div_ceil(2)];
    let read = fs::File::open("/dev/urandom").and_then(|mut f| f.read_exact(&mut bytes));
    if read.is_err() {
        let seed = format!(
            "{}:{:?}:{:p}",
            std::process::id(),
            SystemTime::now().duration_since(SystemTime::UNIX_EPOCH),
            &bytes
        );
        let digest = Sha256::digest(seed.as_bytes());
        for (slot, byte) in bytes.iter_mut().zip(digest.iter()) {
            *slot = *byte;
        }
    }
    let mut out: String = bytes.iter().map(|b| format!("{b:02x}")).collect();
    out.truncate(len);
    out
}

/// Remove scratch directories under `root` that are older than `max_age`.
/// Only real directories with the scratch prefix are touched, never symlinks.
/// Returns how many were removed.
pub fn sweep_stale(root: &Path, max_age: Duration) -> usize {
    let Ok(entries) = fs::read_dir(root) else {
        return 0;
    };
    let mut removed = 0;
    for entry in entries.flatten() {
        let name = entry.file_name();
        if !name.to_string_lossy().starts_with(PREFIX) {
            continue;
        }
        let path = entry.path();
        let Ok(meta) = fs::symlink_metadata(&path) else {
            continue;
        };
        if !meta.is_dir() {
            continue;
        }
        let age = meta
            .modified()
            .ok()
            .and_then(|m| SystemTime::now().duration_since(m).ok())
            .unwrap_or_default();
        if age >= max_age && fs::remove_dir_all(&path).is_ok() {
            removed += 1;
        }
    }
    removed
}
