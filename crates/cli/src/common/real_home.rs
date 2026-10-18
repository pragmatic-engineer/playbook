// SPDX-FileCopyrightText: 2026 Igor Santos
// SPDX-License-Identifier: Apache-2.0

//! A safety net for tests: a debug build that cargo started never runs
//! migrations or the upgrade heal against the real home of the account. A
//! test that starts `playbook` without a scratch `HOME` would otherwise
//! write the real migration record and run the real service manager.
//! Release builds are unchanged.

use std::path::{Path, PathBuf};

/// Set to `1` to let a debug build that cargo started change the real home.
pub const ALLOW_ENV: &str = "PLAYBOOK_ALLOW_REAL_HOME";

/// The note printed when the net holds.
pub const NOTE: &str = "a debug build started by cargo does not change the real home of your account; set PLAYBOOK_ALLOW_REAL_HOME=1 to allow it";

/// Whether `home` is the home the account database lists for this user.
pub fn is_account_home(home: &Path) -> bool {
    let canonical = |p: &Path| std::fs::canonicalize(p).unwrap_or_else(|_| p.to_path_buf());
    crate::service_manager::account_home()
        .is_some_and(|real| home.exists() && canonical(&real) == canonical(home))
}

/// Whether cargo or nextest started this process or one of its parents.
/// Both set `CARGO_MANIFEST_DIR` for the tests they run, and a child
/// inherits it.
fn started_by_cargo() -> bool {
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

/// The real home, for tests that check the net holds.
pub fn account_home() -> Option<PathBuf> {
    crate::service_manager::account_home()
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
    fn a_unit_test_on_the_real_home_is_refused_and_a_scratch_home_is_not() {
        let scratch = std::env::temp_dir().join(format!("pb-real-home-{}", std::process::id()));
        std::fs::create_dir_all(&scratch).unwrap();

        assert!(!refused(&scratch));
        if let Some(real) = account_home() {
            if std::env::var_os(ALLOW_ENV).is_none() {
                assert!(refused(&real));
            }
        }
    }
}
