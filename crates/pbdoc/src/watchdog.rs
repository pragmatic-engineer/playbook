// SPDX-FileCopyrightText: 2026 Igor Santos
// SPDX-License-Identifier: Apache-2.0

//! Self watchdog that exits 4 when the helper's resident size passes the cap (see ADR 0051).

use std::io::Write;
use std::time::Duration;
use std::{io, thread};

/// Linux reads procfs, which is cheap. macOS spawns `ps`, so it polls less often.
#[cfg(target_os = "macos")]
const POLL_INTERVAL: Duration = Duration::from_millis(100);
#[cfg(not(target_os = "macos"))]
const POLL_INTERVAL: Duration = Duration::from_millis(20);
const MEMORY_LIMIT_MESSAGE: &[u8] = b"memory limit reached\n";

/// Writes the limit message and exits 4.
fn exit_over_memory_limit() -> ! {
    let _ = io::stderr().write_all(MEMORY_LIMIT_MESSAGE);
    std::process::exit(i32::from(crate::EXIT_RESOURCE_LIMIT))
}

/// Exits 4 now when the helper's resident size is over `cap_bytes`. The poll
/// thread can miss a spike shorter than its interval, so the converter calls
/// this after each large step.
pub(crate) fn exit_if_over(cap_bytes: usize) {
    if resident_bytes().is_some_and(|rss| rss > cap_bytes) {
        exit_over_memory_limit();
    }
}

/// Starts the watchdog thread for a cap of `cap_bytes`.
pub(crate) fn spawn(cap_bytes: usize) -> io::Result<()> {
    thread::Builder::new()
        .name("pbdoc-watchdog".into())
        .spawn(move || loop {
            exit_if_over(cap_bytes);
            thread::sleep(POLL_INTERVAL);
        })
        .map(drop)
}

/// The resident set in bytes, from the `VmRSS` line the kernel keeps in procfs.
#[cfg(target_os = "linux")]
fn resident_bytes() -> Option<usize> {
    let status = std::fs::read_to_string("/proc/self/status").ok()?;
    let line = status.lines().find_map(|l| l.strip_prefix("VmRSS:"))?;
    let kib: usize = line.trim().strip_suffix("kB")?.trim().parse().ok()?;
    kib.checked_mul(1024)
}

/// The resident set in bytes, from `ps`. macOS has no procfs and std has no
/// call for the size, so each reading costs a process spawn.
#[cfg(target_os = "macos")]
fn resident_bytes() -> Option<usize> {
    let out = std::process::Command::new("/bin/ps")
        .args(["-o", "rss=", "-p", &std::process::id().to_string()])
        .stdin(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .output()
        .ok()?;
    let kib: usize = String::from_utf8(out.stdout).ok()?.trim().parse().ok()?;
    kib.checked_mul(1024)
}

#[cfg(not(any(target_os = "linux", target_os = "macos")))]
fn resident_bytes() -> Option<usize> {
    None
}

#[cfg(all(test, any(target_os = "linux", target_os = "macos")))]
mod tests {
    use super::*;

    #[test]
    fn resident_bytes_reports_a_positive_size_for_this_process() {
        // Act
        let resident = resident_bytes();
        // Assert
        assert!(resident.is_some_and(|bytes| bytes > 0));
    }
}
