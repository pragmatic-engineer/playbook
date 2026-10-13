// SPDX-FileCopyrightText: 2026 Igor Santos
// SPDX-License-Identifier: Apache-2.0

//! SessionStart hook the plugin registers in `hooks/hooks.json`. It detects a
//! `settings.json` that `playbook init` never wired (a plugin update with no
//! matching `playbook init` run) and tells the user to run /playbook:setup.
//!
//! It runs through the plugin's `bin/playbook` shim, which prints the same
//! warning when no binary is installed. Silent once settings.json carries the
//! ported hooks. Never fails the session.

use crate::common::{emit_pre_context, home_dir, Payload};
use std::fs;
use std::path::Path;

/// What a wired `settings.json` contains.
const MARKER: &str = "playbook hook session-init";

/// The warning text. `bin/playbook` carries a copy for the no-binary case, and
/// a test keeps the two equal.
const MESSAGE: &str =
    "playbook is installed as a plugin only. Run /playbook:setup to install the binary and guards.";

/// Hook entry point.
pub fn run(_payload: &Payload) {
    if let Some(message) = warning(&home_dir()) {
        emit_pre_context("SessionStart", message);
    }
}

/// The warning to show, or `None` when `settings.json` under `home` is wired.
/// A missing or unreadable file counts as unwired.
pub fn warning(home: &Path) -> Option<&'static str> {
    (!is_wired(&home.join(".claude/settings.json"))).then_some(MESSAGE)
}

/// True when the `settings.json` at `path` carries the ported hooks.
pub fn is_wired(path: &Path) -> bool {
    fs::read_to_string(path).is_ok_and(|text| text.contains(MARKER))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::common::test_support::scratch_dir;

    fn home(settings: Option<&str>) -> std::path::PathBuf {
        let h = scratch_dir("migcheck");
        fs::create_dir_all(h.join(".claude")).unwrap();
        if let Some(text) = settings {
            fs::write(h.join(".claude/settings.json"), text).unwrap();
        }
        h
    }

    #[test]
    fn wired_settings_stay_silent() {
        let h = home(Some(r#"{"command":"playbook hook session-init"}"#));
        assert_eq!(warning(&h), None);
    }

    #[test]
    fn unwired_missing_or_malformed_settings_warn() {
        assert!(warning(&home(Some(r#"{"hooks":{}}"#))).is_some());
        assert!(warning(&home(None)).is_some());
        assert!(warning(&home(Some("not json {"))).is_some());
    }
}
