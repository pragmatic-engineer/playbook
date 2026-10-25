// SPDX-FileCopyrightText: 2026 Igor Santos
// SPDX-License-Identifier: Apache-2.0

//! Input admission and zip central-directory limits (see ADR 0051).

use std::fs::{self, File};
use std::path::Path;

use zip::ZipArchive;

pub(crate) const HARD_MAX_INPUT_BYTES: u64 = 104_857_600;
pub(crate) const MAX_ZIP_ENTRIES: usize = 10_000;
pub(crate) const MAX_ZIP_UNCOMPRESSED: u64 = 268_435_456;
pub(crate) const MAX_ZIP_RATIO: u64 = 100;
/// Entries below this size skip the ratio check, so tiny files that compress well pass.
pub(crate) const ZIP_RATIO_FLOOR: u64 = 1 << 20;

/// A refused input, with the plain-language reason.
#[derive(Debug)]
pub(crate) struct Refusal(pub String);

/// Why [`admit`] rejected an input.
#[derive(Debug)]
pub(crate) enum AdmitError {
    /// The input is missing, unreadable or not a regular file: a failed conversion.
    Unreadable(String),
    /// The input is over a size limit.
    TooLarge(Refusal),
}

/// Stats `path` and rejects a non-file or one over `min(max_bytes, HARD_MAX_INPUT_BYTES)`.
/// Only metadata is read, never the content.
pub(crate) fn admit(path: &Path, max_bytes: u64) -> Result<(), AdmitError> {
    let meta = fs::metadata(path)
        .map_err(|err| AdmitError::Unreadable(format!("cannot read input: {}", err.kind())))?;
    if !meta.is_file() {
        return Err(AdmitError::Unreadable("input is not a file".into()));
    }
    let limit = max_bytes.min(HARD_MAX_INPUT_BYTES);
    if meta.len() > limit {
        return Err(AdmitError::TooLarge(Refusal(format!(
            "input is too large ({} bytes, limit {limit})",
            meta.len()
        ))));
    }
    Ok(())
}

/// Checks the declared (compressed, uncompressed) sizes of a zip's entries.
pub(crate) fn scan(entries: &[(u64, u64)]) -> Result<(), Refusal> {
    if entries.len() > MAX_ZIP_ENTRIES {
        return Err(too_many_entries(entries.len()));
    }
    let mut total: u64 = 0;
    for &(compressed, uncompressed) in entries {
        if uncompressed >= ZIP_RATIO_FLOOR
            && uncompressed as u128 > compressed as u128 * MAX_ZIP_RATIO as u128
        {
            return Err(Refusal(format!(
                "zip entry compression ratio is too large (limit {MAX_ZIP_RATIO})"
            )));
        }
        total = total.saturating_add(uncompressed);
    }
    if total > MAX_ZIP_UNCOMPRESSED {
        return Err(Refusal(format!(
            "zip uncompressed size is too large (limit {MAX_ZIP_UNCOMPRESSED} bytes)"
        )));
    }
    Ok(())
}

fn too_many_entries(count: usize) -> Refusal {
    Refusal(format!(
        "zip has too many entries ({count}, limit {MAX_ZIP_ENTRIES})"
    ))
}

/// Reads only the zip central directory of `path` and applies [`scan`].
/// A file that is not a readable zip passes: the converter reports it.
pub(crate) fn prescan_zip(path: &Path) -> Result<(), Refusal> {
    let Ok(file) = File::open(path) else {
        return Ok(());
    };
    let Ok(mut archive) = ZipArchive::new(file) else {
        return Ok(());
    };
    if archive.len() > MAX_ZIP_ENTRIES {
        return Err(too_many_entries(archive.len()));
    }
    let mut sizes = Vec::with_capacity(archive.len());
    for index in 0..archive.len() {
        let Ok(entry) = archive.by_index_raw(index) else {
            return Ok(());
        };
        sizes.push((entry.compressed_size(), entry.size()));
    }
    scan(&sizes)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs::File;
    use std::os::unix::fs::MetadataExt;

    const ONE_KB: u64 = 1_000;
    const ONE_MIB: u64 = 1 << 20;
    const SPARSE_LIMIT_BYTES: u64 = ONE_MIB;
    const ENTRY_BYTES: u64 = 10;
    // Just over 1 MiB and divisible by both 100 and 101.
    const RATIO_ENTRY_BYTES: u64 = 100 * 101 * 105;
    const COMPRESSED_AT_RATIO_LIMIT: u64 = RATIO_ENTRY_BYTES / MAX_ZIP_RATIO;
    const COMPRESSED_AT_RATIO_101: u64 = RATIO_ENTRY_BYTES / (MAX_ZIP_RATIO + 1);
    const BELOW_FLOOR_RATIO: u64 = 1_000;

    fn scratch(tag: &str) -> std::path::PathBuf {
        let dir = std::env::temp_dir().join(format!("pbdoc-limits-{}-{tag}", std::process::id()));
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    /// Creates a sparse file, or None (with a printed reason) when the
    /// filesystem allocated real blocks for it.
    fn sparse(tag: &str, len: u64) -> Option<std::path::PathBuf> {
        let path = scratch(tag).join("f");
        let file = File::create(&path).unwrap();
        file.set_len(len).unwrap();
        if fs::metadata(&path).unwrap().blocks() * 512 > SPARSE_LIMIT_BYTES {
            eprintln!("skipped: filesystem does not make sparse files");
            return None;
        }
        Some(path)
    }

    fn message(err: AdmitError) -> String {
        match err {
            AdmitError::Unreadable(text) | AdmitError::TooLarge(Refusal(text)) => text,
        }
    }

    fn entries(count: usize) -> Vec<(u64, u64)> {
        vec![(ENTRY_BYTES, ENTRY_BYTES); count]
    }

    #[test]
    fn admit_refuses_a_file_one_byte_over_the_hard_cap() {
        // Arrange
        let Some(path) = sparse("over", HARD_MAX_INPUT_BYTES + 1) else {
            return;
        };
        // Act
        let result = admit(&path, u64::MAX);
        // Assert
        assert!(message(result.unwrap_err()).contains("too large"));
    }

    #[test]
    fn admit_accepts_a_file_exactly_at_the_hard_cap() {
        // Arrange
        let Some(path) = sparse("at", HARD_MAX_INPUT_BYTES) else {
            return;
        };
        // Act
        let result = admit(&path, u64::MAX);
        // Assert
        assert!(result.is_ok());
    }

    #[test]
    fn admit_refuses_a_file_one_byte_over_a_lower_flag_limit() {
        // Arrange
        let Some(path) = sparse("flag-over", ONE_KB + 1) else {
            return;
        };
        // Act
        let result = admit(&path, ONE_KB);
        // Assert
        assert!(message(result.unwrap_err()).contains("too large"));
    }

    #[test]
    fn admit_accepts_a_file_exactly_at_a_lower_flag_limit() {
        // Arrange
        let Some(path) = sparse("flag-at", ONE_KB) else {
            return;
        };
        // Act
        let result = admit(&path, ONE_KB);
        // Assert
        assert!(result.is_ok());
    }

    #[test]
    fn admit_reports_a_missing_path_as_unreadable() {
        // Arrange
        let path = scratch("missing").join("absent");
        // Act
        let result = admit(&path, HARD_MAX_INPUT_BYTES);
        // Assert
        assert!(matches!(result, Err(AdmitError::Unreadable(_))));
    }

    #[test]
    fn admit_refuses_a_directory_as_not_a_file() {
        // Arrange
        let dir = scratch("dir");
        // Act
        let result = admit(&dir, HARD_MAX_INPUT_BYTES);
        // Assert
        assert!(message(result.unwrap_err()).contains("not a file"));
    }

    #[test]
    fn admit_refuses_a_fifo_as_not_a_file() {
        // Arrange
        let fifo = scratch("fifo").join("pipe");
        let c_path = std::ffi::CString::new(fifo.to_str().unwrap()).unwrap();
        // SAFETY: c_path is a valid NUL-terminated path that outlives the call.
        assert_eq!(unsafe { libc::mkfifo(c_path.as_ptr(), 0o600) }, 0);
        // Act
        let result = admit(&fifo, HARD_MAX_INPUT_BYTES);
        // Assert
        assert!(message(result.unwrap_err()).contains("not a file"));
    }

    #[test]
    fn scan_accepts_exactly_the_entry_limit() {
        // Arrange
        let list = entries(MAX_ZIP_ENTRIES);
        // Act
        let result = scan(&list);
        // Assert
        assert!(result.is_ok());
    }

    #[test]
    fn scan_refuses_one_entry_over_the_limit() {
        // Arrange
        let list = entries(MAX_ZIP_ENTRIES + 1);
        // Act
        let result = scan(&list);
        // Assert
        assert!(result.unwrap_err().0.contains("entries"));
    }

    #[test]
    fn scan_accepts_a_total_exactly_at_the_uncompressed_limit() {
        // Arrange
        let half = MAX_ZIP_UNCOMPRESSED / 2;
        let list = [(half, half), (half, half)];
        // Act
        let result = scan(&list);
        // Assert
        assert!(result.is_ok());
    }

    #[test]
    fn scan_refuses_a_total_one_byte_over_the_uncompressed_limit() {
        // Arrange
        let half = MAX_ZIP_UNCOMPRESSED / 2;
        let list = [(half, half), (half, half + 1)];
        // Act
        let result = scan(&list);
        // Assert
        assert!(result.unwrap_err().0.contains("uncompressed"));
    }

    #[test]
    fn scan_accepts_an_entry_at_the_ratio_limit() {
        // Arrange
        let list = [(COMPRESSED_AT_RATIO_LIMIT, RATIO_ENTRY_BYTES)];
        // Act
        let result = scan(&list);
        // Assert
        assert!(result.is_ok());
    }

    #[test]
    fn scan_refuses_an_entry_at_ratio_101() {
        // Arrange
        let list = [(COMPRESSED_AT_RATIO_101, RATIO_ENTRY_BYTES)];
        // Act
        let result = scan(&list);
        // Assert
        assert!(result.unwrap_err().0.contains("ratio"));
    }

    #[test]
    fn scan_accepts_a_high_ratio_entry_below_the_ratio_floor() {
        // Arrange
        let size = ZIP_RATIO_FLOOR - 1;
        let list = [(size / BELOW_FLOOR_RATIO, size)];
        // Act
        let result = scan(&list);
        // Assert
        assert!(result.is_ok());
    }
}
