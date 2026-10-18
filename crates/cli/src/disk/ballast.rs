// SPDX-FileCopyrightText: 2026 Igor Santos
// SPDX-License-Identifier: Apache-2.0

//! The ballast file (ADR 0047 decision 4): 1 GB of real blocks kept in
//! reserve. Deleting it is one `unlink`, which needs no free space, so it
//! gives back room for a shell and a cleanup on a disk that is full.

use std::fs;
use std::path::{Path, PathBuf};

const FILE_NAME: &str = "ballast.bin";

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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn size_reads_only_a_regular_file() {
        let home =
            std::env::temp_dir().join(format!("pb-disk-ballast-{}", crate::testing::run_id()));
        assert_eq!(size(&home), None);
        fs::create_dir_all(dir(&home)).unwrap();
        fs::write(file(&home), vec![0u8; 100]).unwrap();
        assert_eq!(size(&home), Some(100));
        fs::remove_dir_all(&home).unwrap();
    }
}
