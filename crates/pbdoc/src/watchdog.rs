// SPDX-FileCopyrightText: 2026 Igor Santos
// SPDX-License-Identifier: Apache-2.0

//! Self watchdog that exits 4 when the helper's resident size passes the cap (see ADR 0051).

use std::time::Duration;
use std::{io, thread};

use crate::alloc::exit_over_memory_limit;

const POLL_INTERVAL: Duration = Duration::from_millis(20);

/// Starts the watchdog thread for a cap of `cap_bytes`.
pub fn spawn(cap_bytes: usize) -> io::Result<()> {
    thread::Builder::new()
        .name("pbdoc-watchdog".into())
        .spawn(move || loop {
            if resident_bytes().is_some_and(|rss| rss > cap_bytes) {
                exit_over_memory_limit();
            }
            thread::sleep(POLL_INTERVAL);
        })
        .map(drop)
}

#[cfg(target_os = "linux")]
fn resident_bytes() -> Option<usize> {
    let statm = std::fs::read_to_string("/proc/self/statm").ok()?;
    let pages: usize = statm.split_whitespace().nth(1)?.parse().ok()?;
    // SAFETY: sysconf with a valid name has no memory-safety preconditions.
    let page_size = unsafe { libc::sysconf(libc::_SC_PAGESIZE) };
    usize::try_from(page_size).ok()?.checked_mul(pages)
}

#[cfg(target_os = "macos")]
#[expect(
    deprecated,
    reason = "libc marks the mach task_info items deprecated in favour of the mach2 crate"
)]
fn resident_bytes() -> Option<usize> {
    let mut info = std::mem::MaybeUninit::<libc::mach_task_basic_info>::zeroed();
    let mut count = libc::MACH_TASK_BASIC_INFO_COUNT;
    // SAFETY: `info` and `count` are valid for writes and `count` matches the
    // size of mach_task_basic_info, as task_info requires for this flavor.
    let status = unsafe {
        libc::task_info(
            libc::mach_task_self(),
            libc::MACH_TASK_BASIC_INFO,
            info.as_mut_ptr().cast(),
            &mut count,
        )
    };
    if status != libc::KERN_SUCCESS {
        return None;
    }
    // SAFETY: task_info returned success, so it filled `info`.
    let info = unsafe { info.assume_init() };
    // Copy out of the packed struct before using the field.
    let resident = { info.resident_size };
    usize::try_from(resident).ok()
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
