// SPDX-FileCopyrightText: 2026 Igor Santos
// SPDX-License-Identifier: Apache-2.0

//! `playbook gate check` CLI entry point: query one or more previously
//! recorded phase verdicts for a plan and succeed only when every named
//! phase is PASS or WARN.

use crate::common::paths::{repo_scoped_dir, RepoScope};
use crate::gate::db;
use crate::gate::hash;
use crate::manifest;

/// One phase's resolved state: `Missing` covers a phase never recorded
/// (`db::query_phase` returned `None`); `Stale` covers a recorded row whose
/// `source_hash` no longer matches the current source; the other four
/// mirror `record::Verdict`'s four keywords as read back from the database.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum PhaseState {
    Missing,
    Pass,
    Fail,
    Warn,
    Inconclusive,
    Stale,
}

impl PhaseState {
    /// The exact label printed after the phase name. Each state has its own
    /// distinct literal text, so `"WARN"` never contains `"PASS"` (or vice
    /// versa): a caller inspecting one phase's line can't mistake its state
    /// for another's.
    fn label(&self) -> &'static str {
        match self {
            PhaseState::Missing => "MISSING",
            PhaseState::Pass => "PASS",
            PhaseState::Fail => "FAIL",
            PhaseState::Warn => "WARN",
            PhaseState::Inconclusive => "INCONCLUSIVE",
            PhaseState::Stale => "STALE",
        }
    }

    /// PASS and WARN both satisfy a gate; FAIL, INCONCLUSIVE, MISSING, and
    /// STALE do not.
    fn satisfied(&self) -> bool {
        matches!(self, PhaseState::Pass | PhaseState::Warn)
    }

    /// The `gate_phases.verdict` CHECK constraint (`src/gate/db.rs`)
    /// guarantees a stored verdict is one of the four keywords matched
    /// below; anything else can only reach here via a database written
    /// outside this crate, so it is treated as INCONCLUSIVE rather than
    /// panicking.
    fn from_verdict(verdict: &str) -> Self {
        match verdict {
            "PASS" => PhaseState::Pass,
            "FAIL" => PhaseState::Fail,
            "WARN" => PhaseState::Warn,
            _ => PhaseState::Inconclusive,
        }
    }
}

/// Query every phase in `phases` for `plan_slug` and render a per-phase
/// `"<phase>: <STATE>"` line. `command` is accepted for CLI-shape parity
/// with `gate record` but is not part of the `(plan_slug, phase)` lookup
/// key `db::query_phase` uses, so it plays no role in the result. `source`
/// is hashed once and compared against each recorded row's `source_hash`;
/// a mismatch (or a row with no stored hash) resolves that phase STALE.
///
/// Returns `Ok(output)` when every named phase is satisfied (PASS or WARN),
/// with `output` holding one line per phase. Returns `Err` otherwise: for
/// zero phase names, a pinned "no phases specified" message; for one or
/// more unsatisfied phases (Missing, FAIL, INCONCLUSIVE, or STALE), the same
/// per-phase lines as the success case, so every offending phase is still
/// individually named; for a database or source-read failure, that
/// failure's message.
pub fn run(
    plan_slug: &str,
    _command: &str,
    phases: &[String],
    source: &str,
) -> Result<String, String> {
    if phases.is_empty() {
        return Err(NO_PHASES.to_string());
    }
    let states = evaluate(plan_slug, phases, source)?;
    let output = states
        .iter()
        .map(|(phase, state)| format!("{phase}: {}", state.label()))
        .collect::<Vec<_>>()
        .join("\n");
    if states.iter().all(|(_, state)| state.satisfied()) {
        Ok(output)
    } else {
        Err(output)
    }
}

/// Machine-readable twin of `run`: returns the JSON document and whether the
/// gate is satisfied. An empty phase list yields `ok: false` with an empty
/// `phases` array; a database or source-read failure is still an `Err`.
pub fn run_json(
    plan_slug: &str,
    _command: &str,
    phases: &[String],
    source: &str,
) -> Result<(String, bool), String> {
    let states = if phases.is_empty() {
        Vec::new()
    } else {
        evaluate(plan_slug, phases, source)?
    };
    let ok = !states.is_empty() && states.iter().all(|(_, state)| state.satisfied());
    let rows: Vec<serde_json::Value> = states
        .iter()
        .map(|(phase, state)| serde_json::json!({ "phase": phase, "status": state.label() }))
        .collect();
    let doc = serde_json::json!({
        "version": JSON_VERSION,
        "slug": plan_slug,
        "ok": ok,
        "phases": rows,
    });
    Ok((doc.to_string(), ok))
}

/// The stderr message for a check with no phase names.
pub const NO_PHASES: &str = "no phases specified; provide at least one phase name to check";

/// Schema version of the `--json` document.
const JSON_VERSION: u32 = 1;

/// Resolve each phase's state against the recorded rows and `source`.
fn evaluate(
    plan_slug: &str,
    phases: &[String],
    source: &str,
) -> Result<Vec<(String, PhaseState)>, String> {
    let current_hash = hash::read_and_hash(source)?;

    let repo_root =
        manifest::check::toplevel().ok_or_else(|| "not inside a git repository".to_string())?;
    let dest_base = repo_scoped_dir(RepoScope::Worktree).ok_or_else(|| {
        "could not resolve a worktree-scoped storage location; this repo needs a git \
         'origin' remote and a resolvable worktree toplevel, refusing to fall back to a \
         repo-local path"
            .to_string()
    })?;
    db::migrate_legacy_repo_local(&repo_root, &dest_base)?;
    let db_path = dest_base.join("state.db");
    let conn = db::open_db(&db_path)?;

    let mut states = Vec::with_capacity(phases.len());
    for phase in phases {
        let state = match db::query_phase(&conn, plan_slug, phase)? {
            None => PhaseState::Missing,
            Some(row) => {
                let is_fresh = row.source_hash.as_deref() == Some(current_hash.as_str());
                if is_fresh {
                    PhaseState::from_verdict(&row.verdict)
                } else {
                    PhaseState::Stale
                }
            }
        };
        states.push((phase.clone(), state));
    }
    Ok(states)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn from_verdict_matches_all_four_keywords() {
        // Arrange, Act, Assert
        assert_eq!(PhaseState::from_verdict("PASS"), PhaseState::Pass);
        assert_eq!(PhaseState::from_verdict("FAIL"), PhaseState::Fail);
        assert_eq!(PhaseState::from_verdict("WARN"), PhaseState::Warn);
        assert_eq!(
            PhaseState::from_verdict("INCONCLUSIVE"),
            PhaseState::Inconclusive
        );
    }

    #[test]
    fn pass_and_warn_are_satisfied_others_are_not() {
        // Arrange, Act, Assert
        assert!(PhaseState::Pass.satisfied());
        assert!(PhaseState::Warn.satisfied());
        assert!(!PhaseState::Fail.satisfied());
        assert!(!PhaseState::Inconclusive.satisfied());
        assert!(!PhaseState::Missing.satisfied());
    }

    #[test]
    fn warn_and_pass_labels_are_not_substrings_of_each_other() {
        // Arrange
        let warn = PhaseState::Warn.label();
        let pass = PhaseState::Pass.label();

        // Act, Assert
        assert!(warn.contains("WARN"));
        assert!(!warn.contains("PASS"));
        assert!(pass.contains("PASS"));
        assert!(!pass.contains("WARN"));
    }

    #[test]
    fn stale_label_is_not_a_substring_of_any_other_label() {
        // Arrange
        let stale = PhaseState::Stale.label();
        let others = [
            PhaseState::Missing.label(),
            PhaseState::Pass.label(),
            PhaseState::Fail.label(),
            PhaseState::Warn.label(),
            PhaseState::Inconclusive.label(),
        ];

        // Act, Assert
        for other in others {
            assert!(!stale.contains(other), "STALE must not contain {other}");
            assert!(!other.contains(stale), "{other} must not contain STALE");
        }
    }

    #[test]
    fn zero_phases_is_a_pinned_error_not_a_silent_pass() {
        // Arrange
        let phases: Vec<String> = Vec::new();

        // Act
        let result = run("plan-a", "gate-run", &phases, "unused-source");

        // Assert
        let err = result.expect_err("zero phases must be an error, not a silent pass");
        assert!(err.contains("no phases specified"), "got: {err}");
    }
}
