// SPDX-FileCopyrightText: 2026 Igor Santos
// SPDX-License-Identifier: Apache-2.0

//! The ballast file (ADR 0047 decision 4): 1 GB of real blocks kept in
//! reserve. Deleting it is one `unlink`, which needs no free space, so it
//! gives back room for a shell and a cleanup on a disk that is full.

use super::probe::{self, GIB};
use std::fs;
use std::io::{self, Write};
use std::path::{Path, PathBuf};

/// The size `playbook disk reserve` writes.
pub const BALLAST_BYTES: u64 = GIB;

const FILE_NAME: &str = "ballast.bin";

/// Marks the folder as a cache, which backup tools such as borg, restic and
/// `tar --exclude-caches` skip. See <https://bford.info/cachedir/>.
const CACHEDIR_TAG: &str = "Signature: 8a477f597d28d172789f06886806bc55\n# This file marks playbook's disk ballast as a cache. Backups can skip it.\n";

/// The folder that holds the ballast.
pub fn dir(home: &Path) -> PathBuf {
    home.join(".cache/playbook/ballast")
}

/// The ballast file.
pub fn file(home: &Path) -> PathBuf {
    dir(home).join(FILE_NAME)
}

/// The ballast's size in bytes, or `None` when there is none.
pub fn size(home: &Path) -> Option<u64> {
    fs::symlink_metadata(file(home))
        .ok()
        .filter(fs::Metadata::is_file)
        .map(|m| m.len())
}

/// Writes the ballast of `bytes` under `home`, unless one exists. Refuses
/// when fewer than `keep_free` bytes would be left free afterwards.
pub fn reserve(home: &Path, bytes: u64, keep_free: u64) -> Result<String, String> {
    if let Some(existing) = size(home) {
        return Ok(format!(
            "disk reserve: the ballast already exists ({}) at {}",
            super::size::gb(existing),
            file(home).display()
        ));
    }
    let folder = dir(home);
    let free = probe::space(&folder)
        .map_err(|e| format!("disk reserve: cannot read free space: {e}"))?
        .free;
    if free < bytes.saturating_add(keep_free) {
        return Err(format!(
            "disk reserve: refused, {} free would leave less than {} after the ballast. Free some space first, for example with `playbook disk clean`",
            super::size::gb(free),
            super::size::gb(keep_free)
        ));
    }
    fs::create_dir_all(&folder).map_err(|e| format!("disk reserve: {e}"))?;
    let _ = fs::write(folder.join("CACHEDIR.TAG"), CACHEDIR_TAG);
    let temp = folder.join(format!("{FILE_NAME}.tmp"));
    if let Err(e) = write_zeros(&temp, bytes) {
        let _ = fs::remove_file(&temp);
        return Err(format!("disk reserve: {e}"));
    }
    fs::rename(&temp, file(home)).map_err(|e| format!("disk reserve: {e}"))?;
    exclude_from_backups(&folder);
    probe::forget();
    Ok(format!(
        "disk reserve: wrote {} of ballast at {}. `playbook disk release` deletes it",
        super::size::gb(bytes),
        file(home).display()
    ))
}

/// Writes `bytes` zero bytes and syncs them, so the blocks are allocated.
/// A sparse file or a preallocation that the file system can drop would not
/// hold space in reserve.
fn write_zeros(path: &Path, bytes: u64) -> io::Result<()> {
    const CHUNK: usize = 1024 * 1024;
    let mut out = fs::File::create(path)?;
    let zeros = vec![0u8; CHUNK];
    let mut left = bytes;
    while left > 0 {
        let n = left.min(CHUNK as u64) as usize;
        out.write_all(&zeros[..n])?;
        left -= n as u64;
    }
    out.sync_all()
}

/// Keeps the folder out of Time Machine on macOS. A failure is ignored: the
/// `CACHEDIR.TAG` still marks it for other tools.
fn exclude_from_backups(folder: &Path) {
    if cfg!(target_os = "macos") {
        let _ = std::process::Command::new("tmutil")
            .arg("addexclusion")
            .arg(folder)
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .status();
    }
}

/// Deletes the ballast. Returns the bytes freed, or `None` when there was
/// none.
pub fn release(home: &Path) -> io::Result<Option<u64>> {
    let Some(bytes) = size(home) else {
        return Ok(None);
    };
    fs::remove_file(file(home))?;
    probe::forget();
    Ok(Some(bytes))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn scratch_home(tag: &str) -> PathBuf {
        let home = std::env::temp_dir().join(format!(
            "pb-disk-ballast-{}-{tag}",
            crate::testing::run_id()
        ));
        fs::create_dir_all(&home).unwrap();
        home
    }

    #[test]
    fn reserve_then_release_round_trip() {
        let home = scratch_home("round-trip");
        let bytes = 4 * 1024 * 1024;
        let said = reserve(&home, bytes, 0).unwrap();
        assert!(said.contains("wrote"), "{said}");
        assert_eq!(size(&home), Some(bytes));
        assert!(dir(&home).join("CACHEDIR.TAG").is_file());
        assert!(!dir(&home).join("ballast.bin.tmp").exists());
        let again = reserve(&home, bytes, 0).unwrap();
        assert!(again.contains("already exists"), "{again}");
        assert_eq!(release(&home).unwrap(), Some(bytes));
        assert_eq!(size(&home), None);
        assert_eq!(release(&home).unwrap(), None);
        fs::remove_dir_all(&home).unwrap();
    }

    #[test]
    fn reserve_refuses_when_too_little_would_stay_free() {
        let home = scratch_home("refuse");
        let error = reserve(&home, GIB, u64::MAX / 2).unwrap_err();
        assert!(error.contains("refused"), "{error}");
        assert_eq!(size(&home), None);
        fs::remove_dir_all(&home).unwrap();
    }

    #[cfg(unix)]
    #[test]
    fn the_blocks_are_real() {
        use std::os::unix::fs::MetadataExt;
        let home = scratch_home("blocks");
        reserve(&home, 2 * 1024 * 1024, 0).unwrap();
        let meta = fs::metadata(file(&home)).unwrap();
        assert!(meta.blocks() * 512 >= 2 * 1024 * 1024, "the file is sparse");
        fs::remove_dir_all(&home).unwrap();
    }
}
