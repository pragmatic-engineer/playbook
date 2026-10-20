// SPDX-FileCopyrightText: 2026 Igor Santos
// SPDX-License-Identifier: Apache-2.0

//! Kills the collector that holds a test home's lock, so no test leaves one
//! running. Shared by the tests that can start a collector.

#![allow(dead_code)]

use playbook::usage::daemon::{lock_state, LockState};
use playbook::usage::run::Paths;
use std::time::{Duration, Instant};

/// How long a killed collector may take to let go of the lock.
pub const GONE_BOUND: Duration = Duration::from_secs(5);
/// How long to watch for a late collector after the lock freed.
pub const RECHECK: Duration = Duration::from_millis(500);
const POLL: Duration = Duration::from_millis(50);

/// Polls `done` every `POLL` until `bound`, checking it before the deadline
/// so a condition met on the last poll still counts.
pub fn within(bound: Duration, mut done: impl FnMut() -> bool) -> bool {
    let deadline = Instant::now() + bound;
    loop {
        if done() {
            return true;
        }
        if Instant::now() >= deadline {
            return false;
        }
        std::thread::sleep(POLL);
    }
}

/// Sends `SIGKILL` to the pid the lock names; true when the lock was held.
pub fn kill_holder(paths: &Paths) -> bool {
    match lock_state(paths) {
        LockState::Held(h) => {
            if let Some(pid) = h.pid.and_then(|p| libc::pid_t::try_from(p).ok()) {
                // SAFETY: plain kill(2) on the pid the lock names.
                unsafe { libc::kill(pid, libc::SIGKILL) };
            }
            true
        }
        _ => false,
    }
}

/// Kills holders until the lock frees, then watches `RECHECK` for a late
/// one. A collector still starting writes its pid a moment later, and a
/// late spawn can take the lock after the first one goes. True when no
/// collector holds the lock at the end.
pub fn kill_all(paths: &Paths) -> bool {
    let free = within(GONE_BOUND, || {
        kill_holder(paths);
        lock_state(paths) == LockState::Free
    });
    free && !within(RECHECK, || kill_holder(paths))
}
