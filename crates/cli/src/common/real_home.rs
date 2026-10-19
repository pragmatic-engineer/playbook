// SPDX-FileCopyrightText: 2026 Igor Santos
// SPDX-License-Identifier: Apache-2.0

//! A safety net for tests: a debug build that cargo started does not run
//! migrations, the upgrade heal or the real service manager against the
//! real home of the account. A test that starts `playbook` without a
//! scratch `HOME` would otherwise write the real migration record and run
//! the real `launchctl`. Other writes, such as the session hook's own
//! files, are not covered; the scratch home from `tests/support/isolated.rs`
//! is the first line. Release builds are unchanged.

use std::path::{Path, PathBuf};

/// Set to `1` to let a debug build that cargo started change the real home.
pub const ALLOW_ENV: &str = "PLAYBOOK_ALLOW_REAL_HOME";

/// The note printed when the net holds.
pub const NOTE: &str = "a debug build started by cargo does not change the real home of your account; set PLAYBOOK_ALLOW_REAL_HOME=1 to allow it";

#[cfg(test)]
thread_local! {
    /// A scratch home a unit test treats as the account's home.
    static PRETEND: std::cell::RefCell<Option<PathBuf>> = const { std::cell::RefCell::new(None) };
}

/// Makes `home` count as the account's home on this thread until the guard
/// drops, so a unit test can check the net without the real home.
#[cfg(test)]
pub fn pretend_account_home(home: &Path) -> impl Drop {
    struct Reset;
    impl Drop for Reset {
        fn drop(&mut self) {
            PRETEND.with(|p| *p.borrow_mut() = None);
        }
    }
    PRETEND.with(|p| *p.borrow_mut() = Some(home.to_path_buf()));
    Reset
}

/// The home the account database lists for this user.
fn account_home() -> Option<PathBuf> {
    #[cfg(test)]
    if let Some(home) = PRETEND.with(|p| p.borrow().clone()) {
        return Some(home);
    }
    crate::service_manager::account_home()
}

/// Whether `home` is the home the account database lists for this user. A
/// home that does not exist never is.
pub fn is_account_home(home: &Path) -> bool {
    let canonical = |p: &Path| std::fs::canonicalize(p).unwrap_or_else(|_| p.to_path_buf());
    account_home().is_some_and(|real| home.exists() && canonical(&real) == canonical(home))
}

/// Whether cargo or nextest started this process or one of its parents.
/// cargo sets `CARGO_MANIFEST_DIR` for the tests it runs and nextest sets
/// `NEXTEST_RUN_ID`, and a child inherits either.
pub(crate) fn started_by_cargo() -> bool {
    ["CARGO_MANIFEST_DIR", "NEXTEST_RUN_ID"]
        .iter()
        .any(|k| std::env::var_os(k).is_some())
}

/// Whether the net holds for `home`: a debug build that cargo started, with
/// `home` the real home, and no [`ALLOW_ENV`].
pub fn refused(home: &Path) -> bool {
    let allowed = std::env::var_os(ALLOW_ENV).is_some_and(|v| v == "1");
    decide(
        cfg!(debug_assertions),
        started_by_cargo(),
        is_account_home(home),
        allowed,
    )
}

fn decide(debug: bool, cargo: bool, real_home: bool, allowed: bool) -> bool {
    debug && cargo && real_home && !allowed
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_a_debug_build_started_by_cargo_on_the_real_home_is_refused() {
        assert!(decide(true, true, true, false));
        assert!(!decide(true, true, true, true), "the opt-in lets it run");
        assert!(!decide(false, true, true, false), "a release build runs");
        assert!(
            !decide(true, false, true, false),
            "a person running a dev build"
        );
        assert!(!decide(true, true, false, false), "a scratch home");
    }

    #[test]
    fn a_home_that_counts_as_the_account_home_is_refused_and_another_is_not() {
        let home = crate::common::test_support::scratch_dir("real-home");
        let other = crate::common::test_support::scratch_dir("real-home-other");
        let missing = home.join("gone");
        std::fs::create_dir_all(&home).unwrap();
        std::fs::create_dir_all(&other).unwrap();

        let before = refused(&home);
        let _pretend = pretend_account_home(&home);

        assert!(!before, "a scratch home is not the account home");
        assert_eq!(refused(&home), std::env::var_os(ALLOW_ENV).is_none());
        assert!(!refused(&other));
        assert!(!is_account_home(&missing));
    }
}
