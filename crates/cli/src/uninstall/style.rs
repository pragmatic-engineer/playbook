// SPDX-FileCopyrightText: 2026 Igor Santos
// SPDX-License-Identifier: Apache-2.0

//! The uninstall report as a terminal sees it: one marked line per action,
//! then the totals. A pipe still gets the plain lines from `Report`.

use super::Report;
use crate::ui::render::{self, Counts, Status};
use crate::ui::Caps;

/// The status a report line stands for, `None` for a line that is only context.
fn status_of(line: &str) -> Option<Status> {
    if line.starts_with("removed") {
        Some(Status::Done)
    } else if line.starts_with("would ") {
        Some(Status::Planned)
    } else if line.starts_with("kept") || line.contains("left unchanged") {
        Some(Status::Skipped)
    } else if line.starts_with("could not") {
        Some(Status::Failed)
    } else {
        None
    }
}

/// Render the report. `dry_run` says nothing was changed.
pub fn render(caps: &Caps, report: &Report, dry_run: bool) -> String {
    let title = if dry_run {
        "playbook uninstall (preview, nothing is changed)"
    } else {
        "playbook uninstall"
    };
    let mut out = render::heading(caps, title);
    let mut statuses = Vec::new();
    if report.lines.is_empty() && report.errors.is_empty() {
        out.push_str(&render::progress_line(
            caps,
            Status::Info,
            "",
            "nothing from playbook was found to remove",
        ));
        out.push('\n');
    }
    for line in &report.lines {
        match status_of(line) {
            Some(st) => {
                statuses.push(st);
                out.push_str(&render::progress_line(caps, st, "", line));
            }
            None if line.starts_with("  ") => out.push_str(&format!("      {}", line.trim_start())),
            None => out.push_str(&render::progress_line(caps, Status::Info, "", line)),
        }
        out.push('\n');
    }
    for err in &report.errors {
        statuses.push(Status::Failed);
        out.push_str(&render::progress_line(caps, Status::Failed, "", err));
        out.push('\n');
    }
    let next: &[&str] = if dry_run {
        &["Run `playbook uninstall --yes` to remove these."]
    } else {
        &["Open a new terminal so the shell changes load."]
    };
    out.push_str(&render::next_steps(caps, Counts::tally(&statuses), next));
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn report() -> Report {
        Report {
            lines: vec![
                "would remove 29 hook entries from /h/.claude/settings.json".into(),
                "kept /h/.config/playbook/statusline.sh (your status line still runs it)".into(),
                "remaining manual steps (playbook does not change Claude Code's plugins):".into(),
                "  claude plugin uninstall playbook@x".into(),
            ],
            errors: vec!["rc left unchanged: denied".into()],
        }
    }

    #[test]
    fn a_dry_run_marks_each_line_and_says_nothing_changed() {
        let got = render(&Caps::plain(80), &report(), true);
        assert!(got.contains("preview, nothing is changed"));
        assert!(got.contains("[plan] "));
        assert!(got.contains("[skip] "));
        assert!(got.contains("[fail] rc left unchanged: denied"));
        assert!(got.contains("[info] remaining manual steps"));
        assert!(got.contains("      claude plugin uninstall playbook@x"));
        assert!(got.contains("1 planned"));
        assert!(got.contains("playbook uninstall --yes"));
    }

    #[test]
    fn an_empty_report_says_nothing_was_found() {
        let got = render(&Caps::plain(80), &Report::default(), true);
        assert!(got.contains("[info] nothing from playbook was found to remove"));
    }

    #[test]
    fn a_real_run_marks_removals_done() {
        let r = Report {
            lines: vec!["removed /h/.config/playbook/plugin-seen".into()],
            errors: vec![],
        };
        let got = render(&Caps::plain(80), &r, false);
        assert!(got.contains("[ok] "), "{got}");
        assert!(got.contains("1 done"));
    }
}
