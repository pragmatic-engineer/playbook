// SPDX-FileCopyrightText: 2026 Igor Santos
// SPDX-License-Identifier: MIT

//! `playbook ci`: the model-free repo checks in one step, for CI and other
//! unattended runs. It calls the same functions as `manifest check`,
//! `agents check`, and `settings check`, never reads stdin, and writes
//! nothing but its own report.

use serde_json::json;
use std::path::{Path, PathBuf};

const PLUGIN_MARKER: &str = ".claude-plugin/plugin.json";
const NOT_A_CHECKOUT: &str = "not a playbook checkout";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Status {
    Pass,
    Fail,
    Skip,
}

impl Status {
    fn word(self) -> &'static str {
        match self {
            Status::Pass => "pass",
            Status::Fail => "fail",
            Status::Skip => "skip",
        }
    }
}

#[derive(Debug, Clone)]
pub struct CheckResult {
    pub name: &'static str,
    pub status: Status,
    pub detail: String,
}

#[derive(Debug, Default)]
pub struct Report {
    pub checks: Vec<CheckResult>,
}

impl Report {
    fn count(&self, status: Status) -> usize {
        self.checks.iter().filter(|c| c.status == status).count()
    }

    pub fn failed(&self) -> usize {
        self.count(Status::Fail)
    }

    /// One line per check (extra lines of a long detail indented), then the totals.
    pub fn to_text(&self) -> String {
        let mut out = String::new();
        for check in &self.checks {
            let mut lines = check.detail.lines();
            let first = lines.next().unwrap_or("");
            out.push_str(&format!(
                "{} {}: {first}\n",
                check.status.word().to_uppercase(),
                check.name
            ));
            for rest in lines {
                out.push_str(&format!("  {rest}\n"));
            }
        }
        out.push_str(&format!(
            "ci: {} passed, {} failed, {} skipped",
            self.count(Status::Pass),
            self.failed(),
            self.count(Status::Skip)
        ));
        out
    }

    pub fn to_json(&self) -> String {
        let checks: Vec<_> = self
            .checks
            .iter()
            .map(|c| json!({"name": c.name, "status": c.status.word(), "detail": c.detail}))
            .collect();
        json!({
            "checks": checks,
            "passed": self.count(Status::Pass),
            "failed": self.failed(),
            "skipped": self.count(Status::Skip),
        })
        .to_string()
    }
}

fn result(name: &'static str, status: Status, detail: impl Into<String>) -> CheckResult {
    CheckResult {
        name,
        status,
        detail: detail.into(),
    }
}

fn from_check(name: &'static str, outcome: Result<String, String>) -> CheckResult {
    match outcome {
        Ok(msg) => result(name, Status::Pass, msg),
        Err(msg) => result(name, Status::Fail, msg),
    }
}

/// Runs the three checks against `root`. A check whose inputs are absent is
/// skipped, never failed, so the command is safe to run anywhere.
pub fn run_in(root: &Path) -> Report {
    let mut report = Report::default();
    if !root.join(PLUGIN_MARKER).is_file() {
        for name in ["manifest", "agents", "settings"] {
            report
                .checks
                .push(result(name, Status::Skip, NOT_A_CHECKOUT));
        }
        return report;
    }

    report
        .checks
        .push(from_check("manifest", crate::manifest::check::check(root)));

    let agents = root.join("agents");
    report.checks.push(if agents.is_dir() {
        from_check("agents", crate::agents::check::check(&agents))
    } else {
        result("agents", Status::Skip, "no agents directory")
    });

    let template = root.join("settings.shared.json");
    let perms = root.join("permissions.shared.json");
    report
        .checks
        .push(if template.is_file() && perms.is_file() {
            from_check(
                "settings",
                crate::settings::check::check(&template, &perms, root),
            )
        } else {
            result("settings", Status::Skip, "no shared settings templates")
        });
    report
}

fn enter_dir(dir: &str) -> Result<(), String> {
    let path = Path::new(dir)
        .canonicalize()
        .map_err(|e| format!("--dir {dir}: {e}"))?;
    if !path.is_dir() {
        return Err(format!("--dir {dir} is not a directory"));
    }
    std::env::set_current_dir(&path).map_err(|e| format!("--dir {dir}: {e}"))
}

fn checkout_root() -> PathBuf {
    crate::manifest::check::toplevel()
        .or_else(|| std::env::current_dir().ok())
        .unwrap_or_else(|| PathBuf::from("."))
}

/// CLI entry. Returns the text to print and the exit code (1 when any check fails).
pub fn run(dir: Option<&str>, as_json: bool) -> Result<(String, i32), String> {
    if let Some(dir) = dir {
        enter_dir(dir)?;
    }
    let report = run_in(&checkout_root());
    let text = if as_json {
        report.to_json()
    } else {
        report.to_text()
    };
    Ok((text, i32::from(report.failed() > 0)))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn report(statuses: &[(&'static str, Status, &str)]) -> Report {
        Report {
            checks: statuses.iter().map(|(n, s, d)| result(n, *s, *d)).collect(),
        }
    }

    #[test]
    fn text_has_one_line_per_check_and_exact_totals() {
        let r = report(&[
            (
                "manifest",
                Status::Pass,
                "check-manifest: OK (3 tracked files)",
            ),
            ("agents", Status::Fail, "1 problem:\nbad.md: model 'gpt'"),
            ("settings", Status::Skip, "no shared settings templates"),
        ]);

        assert_eq!(
            r.to_text(),
            "PASS manifest: check-manifest: OK (3 tracked files)\n\
             FAIL agents: 1 problem:\n  bad.md: model 'gpt'\n\
             SKIP settings: no shared settings templates\n\
             ci: 1 passed, 1 failed, 1 skipped"
        );
    }

    #[test]
    fn json_has_the_stable_field_names() {
        let r = report(&[("manifest", Status::Pass, "ok")]);

        let value: serde_json::Value = serde_json::from_str(&r.to_json()).unwrap();

        assert_eq!(
            value,
            serde_json::json!({
                "checks": [{"name": "manifest", "status": "pass", "detail": "ok"}],
                "passed": 1,
                "failed": 0,
                "skipped": 0,
            })
        );
    }

    #[test]
    fn only_a_failed_check_makes_the_exit_code_nonzero() {
        assert_eq!(report(&[("a", Status::Skip, "x")]).failed(), 0);
        assert_eq!(report(&[("a", Status::Fail, "x")]).failed(), 1);
    }
}
