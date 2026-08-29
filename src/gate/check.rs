// SPDX-FileCopyrightText: 2026 Igor Santos
// SPDX-License-Identifier: MIT

//! `playbook gate check` CLI entry point: query one or more previously
//! recorded phase verdicts for a plan and succeed only when every named
//! phase is PASS or WARN.

use crate::gate::db;
use crate::manifest;

/// One phase's resolved state: `Missing` covers a phase never recorded
/// (`db::query_phase` returned `None`); the other four mirror
/// `record::Verdict`'s four keywords as read back from the database.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum PhaseState {
    Missing,
    Pass,
    Fail,
    Warn,
    Inconclusive,
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
        }
    }

    /// PASS and WARN both satisfy a gate; FAIL, INCONCLUSIVE, and MISSING do
    /// not.
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
/// key `db::query_phase` uses, so it plays no role in the result.
///
/// Returns `Ok(output)` when every named phase is satisfied (PASS or WARN),
/// with `output` holding one line per phase. Returns `Err` otherwise: for
/// zero phase names, a pinned "no phases specified" message; for one or
/// more unsatisfied phases (Missing, FAIL, or INCONCLUSIVE), the same
/// per-phase lines as the success case, so every offending phase is still
/// individually named; for a database failure, that failure's message.
pub fn run(plan_slug: &str, _command: &str, phases: &[String]) -> Result<String, String> {
    if phases.is_empty() {
        return Err("no phases specified; provide at least one phase name to check".to_string());
    }

    let repo_root =
        manifest::check::toplevel().ok_or_else(|| "not inside a git repository".to_string())?;
    let db_path = repo_root.join(".claude").join("state.db");
    let conn = db::open_db(&db_path)?;

    let mut lines = Vec::with_capacity(phases.len());
    let mut all_satisfied = true;
    for phase in phases {
        let state = match db::query_phase(&conn, plan_slug, phase)? {
            None => PhaseState::Missing,
            Some(row) => PhaseState::from_verdict(&row.verdict),
        };
        if !state.satisfied() {
            all_satisfied = false;
        }
        lines.push(format!("{phase}: {}", state.label()));
    }
    let output = lines.join("\n");

    if all_satisfied {
        Ok(output)
    } else {
        Err(output)
    }
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
    fn zero_phases_is_a_pinned_error_not_a_silent_pass() {
        // Arrange
        let phases: Vec<String> = Vec::new();

        // Act
        let result = run("plan-a", "gate-run", &phases);

        // Assert
        let err = result.expect_err("zero phases must be an error, not a silent pass");
        assert!(err.contains("no phases specified"), "got: {err}");
    }
}
