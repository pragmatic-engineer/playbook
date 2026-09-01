// SPDX-FileCopyrightText: 2026 Igor Santos
// SPDX-License-Identifier: MIT

//! One module per hook name, matching hooks.json's `HookName` set exactly.
//! Every module is a stub for now: `run` takes the parsed payload and does
//! nothing, so this Work Unit's binary never panics and always exits 0. A
//! later Work Unit fills in one module's body with real behaviour, touching
//! only that file; `dispatch` below is already exhaustive over `HookName`
//! and does not need to change when a stub becomes real.

use crate::common::payload::Payload;
use crate::HookName;

pub mod auto_model_detect;
pub mod bg_await_guard;
pub mod memory_anchors;
pub mod memory_capture;
pub mod memory_signals;
pub mod no_slop_guard;
pub mod post_edit_track;
pub mod precommit_check;
pub mod precompact_warn;
pub mod preread_edit_check;
pub mod preread_size_check;
pub mod rebuild_memory_graph;
pub mod rm_workspace_guard;
pub mod search_counter;
pub mod session_clean_exit;
pub mod session_init;
pub mod staleness;

/// Dispatch a parsed hook payload to the named hook's entry point.
/// Exhaustive over `HookName`, so adding a new variant fails the build here
/// instead of silently doing nothing at runtime.
pub fn dispatch(name: HookName, payload: &Payload) {
    match name {
        HookName::SessionInit => session_init::run(payload),
        HookName::PrereadEditCheck => preread_edit_check::run(payload),
        HookName::PrereadSizeCheck => preread_size_check::run(payload),
        HookName::SearchCounter => search_counter::run(payload),
        HookName::MemoryAnchors => memory_anchors::run(payload),
        HookName::PostEditTrack => post_edit_track::run(payload),
        HookName::RebuildMemoryGraph => rebuild_memory_graph::run(payload),
        HookName::AutoModelDetect => auto_model_detect::run(payload),
        HookName::PrecompactWarn => precompact_warn::run(payload),
        HookName::SessionCleanExit => session_clean_exit::run(payload),
        HookName::MemoryCapture => memory_capture::run(payload),
        HookName::RmWorkspaceGuard => rm_workspace_guard::run(payload),
        HookName::BgAwaitGuard => bg_await_guard::run(payload),
        HookName::NoSlopGuard => no_slop_guard::run(payload),
        HookName::PrecommitCheck => precommit_check::run(payload),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use clap::ValueEnum;
    use std::panic;

    /// Every hook name must survive malformed input without panicking, so a
    /// bug in one hook can never break the PreToolUse hot path. Table-driven
    /// over every declared `HookName` and every malformed-input shape, so a
    /// future variant cannot escape the check.
    #[test]
    fn every_hook_survives_malformed_stdin() {
        // Arrange
        let raw_inputs: [(&str, &str); 4] = [
            ("truncated JSON", r#"{"session_id":"abc","tool_input":{"#),
            ("empty input", ""),
            (
                "valid JSON missing expected field",
                r#"{"unexpected":"value"}"#,
            ),
            ("valid JSON that is not an object", "[1,2,3]"),
        ];
        let mut failures = Vec::new();

        for name in HookName::value_variants() {
            let name = *name;
            for (label, raw) in raw_inputs {
                // Act
                let result = panic::catch_unwind(|| {
                    let payload = Payload::parse(raw);
                    dispatch(name, &payload);
                });

                // Assert (collected, so one failure does not hide the rest)
                if result.is_err() {
                    failures.push(format!("{name:?} panicked on {label}"));
                }
            }
        }

        assert!(failures.is_empty(), "{}", failures.join("\n"));
    }
}
