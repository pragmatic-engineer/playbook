// SPDX-FileCopyrightText: 2026 Igor Santos
// SPDX-License-Identifier: Apache-2.0

//! FNV-1a 64-bit content hashing, shared by `record.rs` and `check.rs` so
//! both hash their staleness inputs the same way instead of duplicating it.

use std::io::Read;

/// FNV-1a 64-bit hash of `bytes`, rendered as lowercase 16-hex-char output.
pub fn hash_hex(bytes: &[u8]) -> String {
    let mut hash: u64 = 0xcbf29ce484222325;
    for byte in bytes {
        hash ^= *byte as u64;
        hash = hash.wrapping_mul(0x100000001b3);
    }
    format!("{hash:016x}")
}

/// Read `source` (`"-"` for stdin, else a file path) and hash its content.
pub fn read_and_hash(source: &str) -> Result<String, String> {
    let bytes = if source == "-" {
        let mut buf = Vec::new();
        std::io::stdin()
            .read_to_end(&mut buf)
            .map_err(|e| format!("failed to read stdin: {e}"))?;
        buf
    } else {
        std::fs::read(source).map_err(|e| format!("failed to read {source}: {e}"))?
    };
    Ok(hash_hex(&bytes))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_input_matches_fnv_offset_basis() {
        // Arrange
        let bytes: &[u8] = &[];

        // Act
        let result = hash_hex(bytes);

        // Assert
        assert_eq!(result, format!("{:016x}", 0xcbf29ce484222325u64));
    }

    #[test]
    fn single_byte_a_matches_canonical_fnv1a64_vector() {
        // Arrange
        let bytes: &[u8] = b"a";

        // Act
        let result = hash_hex(bytes);

        // Assert
        assert_eq!(result, format!("{:016x}", 0xaf63dc4c8601ec8cu64));
    }

    #[test]
    fn same_content_hashed_twice_is_equal() {
        // Arrange
        let bytes: &[u8] = b"identical content";

        // Act
        let (first, second) = (hash_hex(bytes), hash_hex(bytes));

        // Assert
        assert_eq!(first, second);
    }

    #[test]
    fn different_content_hashes_differ() {
        // Arrange
        let (left, right): (&[u8], &[u8]) = (b"content one", b"content two");

        // Act
        let (hash_left, hash_right) = (hash_hex(left), hash_hex(right));

        // Assert
        assert_ne!(hash_left, hash_right);
    }

    #[test]
    fn read_and_hash_nonexistent_file_returns_err() {
        // Arrange
        let source = "/nonexistent/path/that/should/not/exist/hash-wu1.txt";

        // Act
        let result = read_and_hash(source);

        // Assert
        assert!(result.is_err());
    }
}
