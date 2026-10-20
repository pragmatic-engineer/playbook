// SPDX-FileCopyrightText: 2026 Igor Santos
// SPDX-License-Identifier: Apache-2.0

//! Reading the `pbdoc` release tarball without trusting it: gunzip it, then
//! walk the tar blocks by hand and hand back the one regular file asked for.

/// Longest `gzip -dc` may take on the pbdoc tarball.
const GUNZIP_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(60);
/// The most a pbdoc binary may weigh once unpacked.
const MAX_PBDOC_BYTES: usize = 64 * 1024 * 1024;
/// The licence notice that ships beside `pbdoc` and is never installed.
const TAR_NOTICE: &str = "NOTICE-pbdoc";
const TAR_BLOCK: usize = 512;
const TAR_NAME_LEN: usize = 100;
const TAR_SIZE_FIELD: std::ops::Range<usize> = 124..136;
const TAR_TYPE_OFFSET: usize = 156;
const TAR_OCTAL: u32 = 8;
/// The type flags of a pax extended header, for one entry or for all.
const TAR_PAX_HEADERS: &[u8] = b"xg";
/// bsdtar on macOS stores a file's extended attributes in a `._name` entry.
const APPLE_DOUBLE_PREFIX: &str = "._";

pub fn gunzip(bytes: &[u8]) -> Result<Vec<u8>, String> {
    let mut gzip = std::process::Command::new("gzip");
    gzip.arg("-dc");
    let out = crate::common::proc::run_with_input(&mut gzip, bytes, GUNZIP_TIMEOUT)
        .ok_or("gzip did not run or took too long")?;
    if out.status.success() {
        Ok(out.stdout)
    } else {
        Err("it is not a valid gzip archive".into())
    }
}

/// The contents of the `name` entry in `tar`. Every entry must be a regular
/// file, and besides `name` only the release notice may appear. Metadata that
/// no file lands from is skipped: pax headers and macOS `._` entries.
pub fn extract_regular_file(tar: &[u8], name: &str) -> Result<Vec<u8>, String> {
    let mut found = None;
    let mut rest = tar;
    while let Some(head) = rest.get(..TAR_BLOCK).filter(|h| h.iter().any(|&b| b != 0)) {
        let entry = head[..TAR_NAME_LEN]
            .split(|&b| b == 0)
            .next()
            .unwrap_or(&[]);
        let entry = String::from_utf8_lossy(entry);
        let kind = head[TAR_TYPE_OFFSET];
        let skipped = TAR_PAX_HEADERS.contains(&kind);
        if !skipped && !matches!(kind, b'0' | 0) {
            return Err(format!("{entry} is not a regular file"));
        }
        let entry = entry.strip_prefix("./").unwrap_or(&entry);
        let size = std::str::from_utf8(&head[TAR_SIZE_FIELD])
            .ok()
            .and_then(|s| usize::from_str_radix(s.trim_matches(['\0', ' ']), TAR_OCTAL).ok())
            .filter(|&n| n <= MAX_PBDOC_BYTES)
            .ok_or_else(|| format!("{entry} has an unreadable or oversized length"))?;
        let data = rest
            .get(TAR_BLOCK..TAR_BLOCK + size)
            .ok_or("the archive is cut short")?;
        if skipped || entry.starts_with(APPLE_DOUBLE_PREFIX) {
            // Not installed, and not a reason to refuse the archive.
        } else if entry == name {
            if found.replace(data).is_some() {
                return Err(format!("it holds {name} more than once"));
            }
        } else if entry != TAR_NOTICE {
            return Err(format!("it holds {entry}, expected only {name}"));
        }
        rest = rest
            .get(TAR_BLOCK + size.next_multiple_of(TAR_BLOCK)..)
            .unwrap_or(&[]);
    }
    found
        .map(<[u8]>::to_vec)
        .ok_or_else(|| format!("it does not hold {name}"))
}

/// Builds the `pbdoc` release tarballs the update tests serve.
#[cfg(all(test, unix))]
pub(crate) mod pbdoc_fixture {
    use crate::update::download::fake::FakeFetcher;
    use crate::update::verify;

    pub const ASSET: &str = "pbdoc-0.17.0-x86_64-unknown-linux-musl.tar.gz";

    pub fn script(version: &str) -> Vec<u8> {
        format!("#!/bin/sh\necho 'pbdoc {version}'\n").into_bytes()
    }

    /// One ustar entry: `kind` is the type flag, `link` the link target.
    pub fn entry(name: &str, kind: u8, link: &str, data: &[u8]) -> Vec<u8> {
        let mut head = [0u8; 512];
        head[..name.len()].copy_from_slice(name.as_bytes());
        head[100..108].copy_from_slice(b"0000755\0");
        head[124..136].copy_from_slice(format!("{:011o}\0", data.len()).as_bytes());
        head[156] = kind;
        head[157..157 + link.len()].copy_from_slice(link.as_bytes());
        head[257..265].copy_from_slice(b"ustar\x0000");
        head[148..156].fill(b' ');
        let sum: u32 = head.iter().map(|&b| u32::from(b)).sum();
        head[148..156].copy_from_slice(format!("{sum:06o}\0 ").as_bytes());
        let mut out = head.to_vec();
        out.extend_from_slice(data);
        out.resize(512 + data.len().next_multiple_of(512), 0);
        out
    }

    /// The entries as a gzipped tar, ending with the two empty blocks.
    pub fn tarball(entries: &[Vec<u8>]) -> Vec<u8> {
        let mut tar = entries.concat();
        tar.extend_from_slice(&[0u8; 1024]);
        let mut gzip = std::process::Command::new("gzip");
        gzip.arg("-c");
        crate::common::proc::run_with_input(&mut gzip, &tar, std::time::Duration::from_secs(30))
            .expect("gzip ran")
            .stdout
    }

    pub fn good(version: &str) -> Vec<u8> {
        tarball(&[entry("pbdoc", b'0', "", &script(version))])
    }

    /// Drops the pbdoc asset and its SHA256SUMS line from the release.
    pub fn remove(f: &mut FakeFetcher, base: &str) {
        f.0.remove(&format!("{base}/{ASSET}"));
        let url = format!("{base}/SHA256SUMS");
        let sums = String::from_utf8(f.0[&url].clone()).unwrap();
        let kept: String = sums
            .lines()
            .filter(|line| !line.ends_with(ASSET))
            .map(|line| format!("{line}\n"))
            .collect();
        f.0.insert(url, kept.into_bytes());
    }

    /// Serves `tarball` under `base` and lists it in that release's
    /// SHA256SUMS with the hash of `hashed`, replacing any earlier one.
    pub fn serve(f: &mut FakeFetcher, base: &str, tarball: &[u8], hashed: &[u8]) {
        remove(f, base);
        f.0.insert(format!("{base}/{ASSET}"), tarball.to_vec());
        let url = format!("{base}/SHA256SUMS");
        let mut sums = String::from_utf8(f.0[&url].clone()).unwrap();
        sums.push_str(&format!("{}  {ASSET}\n", verify::sha256_hex(hashed)));
        f.0.insert(url, sums.into_bytes());
    }
}

#[cfg(all(test, unix))]
mod tests {
    use super::pbdoc_fixture::{entry, script, tarball};
    use super::*;
    use crate::common::test_support::scratch_dir;
    use std::process::Command;

    fn body() -> Vec<u8> {
        script("1.0.0")
    }

    #[test]
    fn an_apple_double_entry_before_pbdoc_is_skipped() {
        let tar = gunzip(&tarball(&[
            entry("._pbdoc", b'0', "", b"\x00\x05\x16\x07 apple"),
            entry("pbdoc", b'0', "", &body()),
            entry("NOTICE-pbdoc", b'0', "", b"notice"),
        ]))
        .unwrap();

        assert_eq!(extract_regular_file(&tar, "pbdoc").unwrap(), body());
    }

    #[test]
    fn a_pax_header_before_an_entry_is_skipped() {
        let tar = gunzip(&tarball(&[
            entry("PaxHeader/pbdoc", b'x', "", b"30 mtime=1700000000.1\n"),
            entry("pbdoc", b'0', "", &body()),
        ]))
        .unwrap();

        assert_eq!(extract_regular_file(&tar, "pbdoc").unwrap(), body());
    }

    #[test]
    fn an_apple_double_name_in_a_directory_is_still_refused() {
        let tar = gunzip(&tarball(&[
            entry("pbdoc", b'0', "", &body()),
            entry("dir/._x", b'0', "", b"x"),
        ]))
        .unwrap();

        let err = extract_regular_file(&tar, "pbdoc").unwrap_err();

        assert!(err.contains("expected only pbdoc"), "{err}");
    }

    /// The system tar, with an extended attribute on the file when the
    /// platform has the `xattr` tool: on macOS that adds the `._pbdoc` entry.
    #[test]
    fn a_tarball_made_by_the_system_tar_installs_pbdoc() {
        let dir = scratch_dir("tarball-system-tar");
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("pbdoc"), body()).unwrap();
        std::fs::write(dir.join(TAR_NOTICE), b"notice").unwrap();
        let _ = Command::new("xattr")
            .args(["-w", "com.apple.test", "x"])
            .arg(dir.join("pbdoc"))
            .output();
        let archive = dir.join("pbdoc.tar.gz");
        let made = Command::new("tar")
            .arg("-czf")
            .arg(&archive)
            .arg("-C")
            .arg(&dir)
            .args(["pbdoc", TAR_NOTICE])
            .status()
            .unwrap();
        assert!(made.success());

        let tar = gunzip(&std::fs::read(&archive).unwrap()).unwrap();

        assert_eq!(extract_regular_file(&tar, "pbdoc").unwrap(), body());
    }
}
