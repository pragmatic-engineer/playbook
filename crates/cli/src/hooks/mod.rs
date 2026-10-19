// SPDX-FileCopyrightText: 2026 Igor Santos
// SPDX-License-Identifier: Apache-2.0

//! One module per hook name, matching the `HookName` set exactly.
//! Each module owns its hook's behaviour, and `dispatch` below is exhaustive
//! over `HookName`.

use crate::common::payload::Payload;
use crate::HookName;

pub mod auto_cost;
pub mod auto_guard;
pub mod auto_model_detect;
pub mod bg_await_guard;
pub mod commit_message_sanitizer;
pub mod context_hygiene;
pub mod disk_guard;
pub mod doc_read_hint;
pub mod memory_anchors;
pub mod memory_capture;
pub mod memory_signals;
pub mod policy_guard;
pub mod post_edit_track;
pub mod precommit_check;
pub mod precompact_warn;
pub mod preread_edit_check;
pub mod preread_reread;
pub mod preread_size_check;
pub mod rebuild_memory_graph;
pub mod rm_workspace_guard;
pub mod search_counter;
pub mod session_clean_exit;
pub mod session_init;
pub mod staleness;
pub mod usage_warm;
pub mod worktree_create;
pub mod worktree_remove;
pub mod worktree_setup;

/// The text `playbook hooks explain-rm` prints for `command` run in `cwd`
/// (the current folder when `None`): one line per path with its verdict, then
/// the overall verdict and the denial text, or the same as JSON. An error when
/// `cwd` is not a folder.
///
/// The folder is made absolute without following links, as the hook gets it
/// from Claude Code, because `cd ..` is applied by name.
pub fn explain_rm(
    command: &str,
    cwd: Option<&std::path::Path>,
    json: bool,
) -> Result<String, String> {
    let cwd = match cwd {
        Some(dir) => std::path::absolute(dir).map_err(|e| format!("{}: {e}", dir.display()))?,
        None => std::env::current_dir().map_err(|e| format!("the current folder: {e}"))?,
    };
    if !cwd.is_dir() {
        return Err(format!("{} is not a folder", cwd.display()));
    }
    let (targets, denial) = rm_workspace_guard::explain(command, &cwd);
    let verdict = if denial.is_some() { "deny" } else { "allow" };
    if json {
        let targets: Vec<serde_json::Value> = targets
            .iter()
            .map(|t| {
                serde_json::json!({
                    "target": t.target,
                    "kind": t.kind,
                    "verdict": if t.rule.is_some() { "deny" } else { "allow" },
                    "rule": t.rule,
                    "resolved": t.resolved.as_ref().map(|p| p.display().to_string()),
                })
            })
            .collect();
        let out = serde_json::json!({
            "verdict": verdict,
            "cwd": cwd.display().to_string(),
            "targets": targets,
            "message": denial,
        });
        return Ok(format!("{out}\n"));
    }
    if targets.is_empty() {
        return Ok(
            "allow: the command deletes and overwrites nothing the guard reads\n".to_string(),
        );
    }
    let mut out = String::new();
    for t in &targets {
        let line = match (t.rule, &t.resolved) {
            (Some(rule), Some(dir)) => format!(
                "deny   {:<12} {}  (rule: {rule}, resolved to {})",
                t.kind,
                t.target,
                dir.display()
            ),
            (Some(rule), None) => format!("deny   {:<12} {}  (rule: {rule})", t.kind, t.target),
            (None, _) => format!("allow  {:<12} {}", t.kind, t.target),
        };
        out.push_str(&line);
        out.push('\n');
    }
    out.push_str(&format!("verdict: {verdict}\n"));
    if let Some(text) = denial {
        out.push_str(&format!("\n{text}\n"));
    }
    Ok(out)
}

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
        // Retired: allow, so an old settings entry stays quiet.
        HookName::NoSlopGuard => {}
        HookName::CommitMessageSanitizer => commit_message_sanitizer::run(payload),
        HookName::PolicyGuard => policy_guard::run(payload),
        HookName::PrecommitCheck => precommit_check::run(payload),
        HookName::AutoGuard => auto_guard::run(payload),
        HookName::AutoCost => auto_cost::run(payload),
        HookName::WorktreeCreate => worktree_create::run(payload),
        HookName::WorktreeRemove => worktree_remove::run(payload),
        HookName::ContextHygiene => context_hygiene::run(payload),
        HookName::DocReadHint => doc_read_hint::run(payload),
        HookName::UsageWarm => usage_warm::run(payload),
        HookName::DiskGuard => disk_guard::run(payload),
        // Retired. Old plugin caches still call it; succeed without output.
        HookName::MigrationCheck => {}
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
