// SPDX-FileCopyrightText: 2026 Igor Santos
// SPDX-License-Identifier: MIT

//! `playbook gate record` CLI entry point: parse a phase agent's raw output
//! for a `VERDICT: PASS|FAIL|WARN|INCONCLUSIVE` line and upsert it into the
//! gate-check database at `.claude/state.db`.

use crate::gate::db;
use crate::manifest;
use std::fmt;
use std::io::Read;

/// One of the four verdicts a phase agent can report, matching the
/// `gate_phases.verdict` CHECK constraint in `src/gate/db.rs`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Verdict {
    Pass,
    Fail,
    Warn,
    Inconclusive,
}

impl Verdict {
    /// The exact uppercase keyword this verdict is stored and matched as.
    pub fn as_str(&self) -> &'static str {
        match self {
            Verdict::Pass => "PASS",
            Verdict::Fail => "FAIL",
            Verdict::Warn => "WARN",
            Verdict::Inconclusive => "INCONCLUSIVE",
        }
    }

    /// Exact, case-insensitive match against one of the four keywords. Not
    /// public: only `extract_verdict` needs it, and it never partially
    /// matches ("PASSING" must not parse as PASS).
    fn parse(value: &str) -> Option<Self> {
        match value.to_ascii_uppercase().as_str() {
            "PASS" => Some(Verdict::Pass),
            "FAIL" => Some(Verdict::Fail),
            "WARN" => Some(Verdict::Warn),
            "INCONCLUSIVE" => Some(Verdict::Inconclusive),
            _ => None,
        }
    }
}

impl fmt::Display for Verdict {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

/// Scan every line of `raw` for a `verdict:` prefix, tolerant of `#`/`*`
/// heading or bold decoration occurring anywhere in the line. If more than
/// one line matches, the LAST one wins, so a real closing verdict overrides
/// an earlier quoted example. A line that starts with the `verdict:` prefix
/// but carries a value outside the four keywords is not counted as a match:
/// scanning continues and an earlier valid match still stands. `None` is
/// returned when the whole input has no valid match at all.
pub fn extract_verdict(raw: &str) -> Option<Verdict> {
    let mut result = None;
    for line in raw.lines() {
        let stripped: String = line.chars().filter(|c| *c != '#' && *c != '*').collect();
        let trimmed = stripped.trim();
        let lower = trimmed.to_ascii_lowercase();
        if lower.starts_with("verdict:") {
            // `to_ascii_lowercase` only rewrites ASCII bytes in place, so
            // `lower` and `trimmed` share the same byte length: slicing
            // `trimmed` at the same offset keeps the value's original case
            // for `Verdict::parse`.
            let value = trimmed["verdict:".len()..].trim();
            if let Some(verdict) = Verdict::parse(value) {
                result = Some(verdict);
            }
        }
    }
    result
}

/// Read `input` (a file path, or `"-"` for stdin), extract its verdict, and
/// upsert it into the gate-check database at `<repo root>/.claude/state.db`.
/// Returns `Err` without ever calling [`db::upsert_phase`] when no valid
/// `VERDICT:` line is found, so a bad report never overwrites a prior good
/// recording.
pub fn run(plan_slug: &str, command: &str, phase: &str, input: &str) -> Result<(), String> {
    let raw = read_input(input)?;
    let Some(verdict) = extract_verdict(&raw) else {
        return Err(format!("no VERDICT line found in {input}"));
    };

    let repo_root =
        manifest::check::toplevel().ok_or_else(|| "not inside a git repository".to_string())?;
    let db_path = repo_root.join(".claude").join("state.db");
    let conn = db::open_db(&db_path)?;
    let recorded_at = recorded_at_now();
    db::upsert_phase(
        &conn,
        plan_slug,
        phase,
        verdict.as_str(),
        &raw,
        command,
        &recorded_at,
    )
}

/// Read the whole of `input`: `"-"` means stdin, anything else is a file
/// path.
fn read_input(input: &str) -> Result<String, String> {
    if input == "-" {
        let mut buf = String::new();
        std::io::stdin()
            .read_to_string(&mut buf)
            .map_err(|e| format!("failed to read stdin: {e}"))?;
        Ok(buf)
    } else {
        std::fs::read_to_string(input).map_err(|e| format!("failed to read {input}: {e}"))
    }
}

/// Current UTC time as an ISO-8601 timestamp, via the same `date` subprocess
/// pattern `cc::sessions::local_timestamp` already uses to render a
/// formatted clock reading (`src/cc/sessions.rs:115-133`), rather than
/// adding a new date/time dependency for one human-readable string.
/// `date -u +%Y-%m-%dT%H:%M:%SZ` needs no BSD/GNU fallback the way that
/// formatter does: both date implementations accept `-u` and `+FORMAT`
/// identically for the CURRENT time; that formatter's fallback only exists
/// because it re-renders an arbitrary PAST epoch, which BSD and GNU `date`
/// spell with different flags. Falls back to raw unix seconds, matching
/// `hooks::post_edit_track::now_unix_seconds`'s `SystemTime` usage, if
/// `date` is unavailable at all.
fn recorded_at_now() -> String {
    if let Ok(out) = std::process::Command::new("date")
        .args(["-u", "+%Y-%m-%dT%H:%M:%SZ"])
        .output()
    {
        if out.status.success() {
            let stamp = String::from_utf8_lossy(&out.stdout).trim().to_string();
            if !stamp.is_empty() {
                return stamp;
            }
        }
    }
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
        .to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bare_verdict_line_extracts_pass() {
        // Arrange
        let raw = "VERDICT: PASS";

        // Act
        let result = extract_verdict(raw);

        // Assert
        assert_eq!(result, Some(Verdict::Pass));
    }

    #[test]
    fn heading_decorated_verdict_extracts_fail() {
        // Arrange
        let raw = "## Verdict: FAIL";

        // Act
        let result = extract_verdict(raw);

        // Assert
        assert_eq!(result, Some(Verdict::Fail));
    }

    #[test]
    fn bold_decorated_verdict_extracts_warn() {
        // Arrange
        let raw = "**Verdict:** WARN";

        // Act
        let result = extract_verdict(raw);

        // Assert
        assert_eq!(result, Some(Verdict::Warn));
    }

    #[test]
    fn two_verdict_lines_last_one_wins() {
        // Arrange: an early quoted example line, then the real closing line.
        let raw = "Report format looks like:\nVERDICT: FAIL\n\nActual result:\nVERDICT: PASS";

        // Act
        let result = extract_verdict(raw);

        // Assert
        assert_eq!(result, Some(Verdict::Pass));
    }
}
