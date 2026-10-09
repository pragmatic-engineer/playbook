// SPDX-FileCopyrightText: 2026 Igor Santos
// SPDX-License-Identifier: Apache-2.0

//! `playbook pr ci-wait`, `land-wait` and `merge`: the polling loops and the
//! merge wrappers that `commands/implement.md` used to run as bash blocks.
//! The loops take their inputs and clock as parameters, so tests run them
//! without a network or a sleep.

use crate::common::proc::run_with_timeout;
use crate::json::ghjson;
use serde_json::Value;
use std::process::Command;
use std::time::{Duration, Instant};

const GH_TIMEOUT: Duration = Duration::from_secs(60);

/// Check counts from one `gh pr checks` read.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Counts {
    pub total: usize,
    pub pending: usize,
    pub fail: usize,
    pub cancel: usize,
}

impl Counts {
    pub fn from_json(json: &str) -> Counts {
        let b = ghjson::bucket_counts(json, &["pending", "fail", "cancel"]);
        let n = |i: usize| b.get(i).map_or(0, |(_, c)| *c);
        Counts {
            total: ghjson::array_length(json),
            pending: n(0),
            fail: n(1),
            cancel: n(2),
        }
    }
}

/// The verdict for one read, or `None` to keep waiting. Order matters: a
/// failure wins over pending checks, and cancelled only counts once nothing
/// is pending.
fn ci_verdict(c: Counts) -> Option<&'static str> {
    if c.total == 0 {
        Some("NONE")
    } else if c.fail > 0 {
        Some("FAIL")
    } else if c.pending == 0 && c.cancel > 0 {
        Some("CANCELLED")
    } else if c.pending == 0 {
        Some("PASS")
    } else {
        None
    }
}

/// Poll until a verdict or `timeout`. `read` returns the checks JSON,
/// `emit` prints a progress line, `sleep` waits between reads. Returns the
/// verdict word.
pub fn ci_wait(
    timeout: Duration,
    read: &mut dyn FnMut() -> String,
    emit: &mut dyn FnMut(String),
    sleep: &mut dyn FnMut(),
) -> &'static str {
    let start = Instant::now();
    loop {
        let c = Counts::from_json(&read());
        emit(format!(
            "checks total={} pending={} fail={} cancel={}",
            c.total, c.pending, c.fail, c.cancel
        ));
        if let Some(v) = ci_verdict(c) {
            return v;
        }
        if start.elapsed() >= timeout {
            return "TIMEOUT";
        }
        sleep();
    }
}

/// One PR status line, `state mergeState review armed`, tab separated, as the
/// land loop reads it.
fn land_line(view: &Value) -> String {
    let s = |k: &str| view.get(k).and_then(Value::as_str).unwrap_or("");
    let review = match s("reviewDecision") {
        "" => "-",
        r => r,
    };
    let armed = if view.get("autoMergeRequest").is_some_and(|v| !v.is_null()) {
        "armed"
    } else {
        "-"
    };
    format!(
        "{}\t{}\t{}\t{}",
        s("state"),
        s("mergeStateStatus"),
        review,
        armed
    )
}

/// The verdict for one status line, or `None` to keep waiting.
fn land_verdict(line: &str) -> Option<&'static str> {
    if line.starts_with("MERGED") {
        Some("MERGED")
    } else if line.contains("REVIEW_REQUIRED") {
        Some("REVIEW_GATE")
    } else if line.contains("CHANGES_REQUESTED") {
        Some("CHANGES_REQUESTED")
    } else if line.contains("DIRTY") || line.contains("BEHIND") {
        Some("RESTATE")
    } else {
        None
    }
}

/// Poll until the PR lands, hits a gate, or `timeout`.
pub fn land_wait(
    timeout: Duration,
    read: &mut dyn FnMut() -> String,
    emit: &mut dyn FnMut(String),
    sleep: &mut dyn FnMut(),
) -> &'static str {
    let start = Instant::now();
    loop {
        let line = read();
        emit(line.clone());
        if let Some(v) = land_verdict(&line) {
            return v;
        }
        if start.elapsed() >= timeout {
            return "TIMEOUT";
        }
        sleep();
    }
}

/// Stdout of `gh` whatever its exit status. `gh pr checks` exits non-zero
/// while checks fail but still prints the JSON, so the status is ignored.
fn gh_stdout(args: &[&str]) -> Option<String> {
    let mut cmd = Command::new("gh");
    cmd.args(args);
    let out = run_with_timeout(&mut cmd, GH_TIMEOUT)?;
    Some(String::from_utf8_lossy(&out.stdout).into_owned())
}

/// The required checks of `pr` as `gh` prints them, `[]` when unreadable.
pub fn read_checks(pr: &str) -> String {
    gh_stdout(&[
        "pr",
        "checks",
        pr,
        "--required",
        "--json",
        "name,bucket,link",
    ])
    .filter(|t| serde_json::from_str::<Value>(t).is_ok())
    .unwrap_or_else(|| "[]".to_string())
}

/// The land status line of `pr`, empty when unreadable.
pub fn read_land_line(pr: &str) -> String {
    gh_stdout(&[
        "pr",
        "view",
        pr,
        "--json",
        "state,mergeStateStatus,reviewDecision,autoMergeRequest",
    ])
    .and_then(|t| serde_json::from_str::<Value>(&t).ok())
    .map(|v| land_line(&v))
    .unwrap_or_default()
}

/// `gh pr merge` with `--auto`, or `--admin --squash`. Returns the two lines
/// the command file reads: `<label>_rc=<code>` then gh's combined output.
pub fn merge(pr: &str, admin: bool) -> String {
    let (label, args): (&str, Vec<&str>) = if admin {
        ("admin", vec!["pr", "merge", pr, "--admin", "--squash"])
    } else {
        ("merge", vec!["pr", "merge", pr, "--auto"])
    };
    let mut cmd = Command::new("gh");
    cmd.args(&args);
    match run_with_timeout(&mut cmd, GH_TIMEOUT) {
        Some(out) => {
            let mut text = String::from_utf8_lossy(&out.stdout).into_owned();
            text.push_str(&String::from_utf8_lossy(&out.stderr));
            format!(
                "{label}_rc={}\n{}",
                out.status.code().unwrap_or(-1),
                text.trim_end()
            )
        }
        None => format!("{label}_rc=-1\ngh did not finish (missing, or timed out)"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn checks(buckets: &[&str]) -> String {
        serde_json::to_string(
            &buckets
                .iter()
                .map(|b| json!({"name":"c","bucket":b}))
                .collect::<Vec<_>>(),
        )
        .unwrap()
    }

    fn verdict_of(buckets: &[&str]) -> Option<&'static str> {
        ci_verdict(Counts::from_json(&checks(buckets)))
    }

    #[test]
    fn no_checks_is_none() {
        assert_eq!(verdict_of(&[]), Some("NONE"));
    }

    #[test]
    fn a_failure_wins_over_pending() {
        assert_eq!(verdict_of(&["pass", "fail", "pending"]), Some("FAIL"));
    }

    #[test]
    fn cancelled_waits_for_pending_to_finish() {
        assert_eq!(verdict_of(&["cancel", "pending"]), None);
        assert_eq!(verdict_of(&["cancel", "pass"]), Some("CANCELLED"));
    }

    #[test]
    fn all_passed_is_pass_and_pending_keeps_waiting() {
        assert_eq!(verdict_of(&["pass", "skipping"]), Some("PASS"));
        assert_eq!(verdict_of(&["pass", "pending"]), None);
    }

    #[test]
    fn the_loop_reports_each_read_then_the_verdict() {
        let mut reads = vec![checks(&["pending"]), checks(&["pass"])].into_iter();
        let mut lines = Vec::new();
        let v = ci_wait(
            Duration::from_secs(60),
            &mut || reads.next().unwrap(),
            &mut |l| lines.push(l),
            &mut || {},
        );
        assert_eq!(v, "PASS");
        assert_eq!(
            lines,
            vec![
                "checks total=1 pending=1 fail=0 cancel=0",
                "checks total=1 pending=0 fail=0 cancel=0"
            ]
        );
    }

    #[test]
    fn the_loop_times_out() {
        let v = ci_wait(
            Duration::ZERO,
            &mut || checks(&["pending"]),
            &mut |_| {},
            &mut || {},
        );
        assert_eq!(v, "TIMEOUT");
    }

    #[test]
    fn the_land_line_has_four_tab_separated_fields() {
        let v = json!({"state":"OPEN","mergeStateStatus":"CLEAN","reviewDecision":null,"autoMergeRequest":{"x":1}});
        assert_eq!(land_line(&v), "OPEN\tCLEAN\t-\tarmed");
        let v = json!({"state":"OPEN","mergeStateStatus":"BLOCKED","reviewDecision":"APPROVED","autoMergeRequest":null});
        assert_eq!(land_line(&v), "OPEN\tBLOCKED\tAPPROVED\t-");
    }

    #[test]
    fn land_verdicts_follow_the_documented_order() {
        assert_eq!(land_verdict("MERGED\tCLEAN\t-\t-"), Some("MERGED"));
        assert_eq!(
            land_verdict("OPEN\tBLOCKED\tREVIEW_REQUIRED\t-"),
            Some("REVIEW_GATE")
        );
        assert_eq!(
            land_verdict("OPEN\tBLOCKED\tCHANGES_REQUESTED\t-"),
            Some("CHANGES_REQUESTED")
        );
        assert_eq!(land_verdict("OPEN\tDIRTY\t-\t-"), Some("RESTATE"));
        assert_eq!(land_verdict("OPEN\tBEHIND\t-\tarmed"), Some("RESTATE"));
        assert_eq!(land_verdict("OPEN\tBLOCKED\t-\tarmed"), None);
    }

    #[test]
    fn the_land_loop_stops_on_the_first_verdict() {
        let mut reads = vec![
            "OPEN\tBLOCKED\t-\tarmed".to_string(),
            "MERGED\tCLEAN\t-\t-".to_string(),
        ]
        .into_iter();
        let mut n = 0;
        let v = land_wait(
            Duration::from_secs(60),
            &mut || reads.next().unwrap(),
            &mut |_| {},
            &mut || n += 1,
        );
        assert_eq!((v, n), ("MERGED", 1));
    }
}
