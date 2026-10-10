// SPDX-FileCopyrightText: 2026 Igor Santos
// SPDX-License-Identifier: Apache-2.0

//! Helpers for the integration tests under `tests/`. Not part of the public
//! interface.

use std::sync::OnceLock;

/// An id that is unique to this process run, for naming scratch directories.
///
/// The process id alone is not: ids are reused, and scratch directories from
/// earlier runs are never removed, so a new process with a reused id walks
/// into a stale repository or config left by an old one (a fresh `git remote
/// add origin` then fails because the old run already added it). The start
/// time in nanoseconds makes the id differ between runs.
pub fn run_id() -> &'static str {
    static ID: OnceLock<String> = OnceLock::new();
    ID.get_or_init(|| {
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_or(0, |d| d.as_nanos());
        format!("{}-{nanos}", std::process::id())
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_id_is_stable_within_a_run_and_carries_the_process_id() {
        assert_eq!(run_id(), run_id());
        assert!(run_id().starts_with(&format!("{}-", std::process::id())));
    }
}
