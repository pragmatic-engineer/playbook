// SPDX-FileCopyrightText: 2026 Igor Santos
// SPDX-License-Identifier: Apache-2.0

//! The doctor table as a terminal sees it. A pipe, `--json` and the
//! `/playbook:doctor` skill keep reading the plain `check::render` text.

use super::check::{Level, Row};
use crate::ui::render::{self, Counts, Status};
use crate::ui::Caps;

fn status_of(level: Level) -> Status {
    match level {
        Level::Pass => Status::Done,
        Level::Info => Status::Info,
        Level::Warn => Status::Warn,
        Level::Fail => Status::Failed,
    }
}

/// One marked line per row, a hint under it, then the totals and the fixes.
pub fn render(caps: &Caps, rows: &[Row]) -> String {
    let mut out = render::heading(caps, "playbook doctor");
    let mut statuses = Vec::new();
    for r in rows {
        let st = status_of(r.level);
        statuses.push(st);
        out.push_str(&render::progress_line(caps, st, "", &r.message));
        out.push('\n');
        if let Some(h) = &r.hint {
            out.push_str(&render::wrap(&format!("Fix: {h}"), caps.width, 6));
            out.push('\n');
        }
    }
    let closing = super::check::closing(rows);
    out.push_str(&render::next_steps(
        caps,
        Counts::tally(&statuses),
        &closing,
    ));
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn row(level: Level, message: &str, hint: Option<&str>) -> Row {
        Row {
            layer: "1",
            level,
            message: message.to_string(),
            hint: hint.map(str::to_string),
        }
    }

    #[test]
    fn rows_get_marks_hints_and_totals() {
        let rows = vec![
            row(Level::Pass, "plugin installed", None),
            row(
                Level::Warn,
                "status line differs",
                Some("run playbook init"),
            ),
            row(Level::Fail, "binary missing", Some("install it")),
        ];
        let got = render(&Caps::plain(80), &rows);
        assert!(got.contains("[ok] plugin installed\n"), "{got}");
        assert!(
            got.contains("[warn] status line differs\n      Fix: run playbook init\n"),
            "{got}"
        );
        assert!(got.contains("[fail] binary missing"));
        assert!(got.contains("1 done, 0 unchanged, 0 skipped, 1 failed, 1 warnings"));
        assert!(got.contains("Run /playbook:setup to fix layers 1 to 5."));
    }

    #[test]
    fn info_rows_are_not_counted_and_multiline_messages_keep_the_closing_lines() {
        let rows = vec![
            row(Level::Info, "note", None),
            row(
                Level::Fail,
                "hook points at a missing file: cd /x &&\n bash /y.sh",
                Some("fix"),
            ),
        ];
        let got = render(&Caps::plain(80), &rows);
        assert!(
            got.contains("0 done, 0 unchanged, 0 skipped, 1 failed"),
            "{got}"
        );
        assert!(got.contains("Run /playbook:setup to fix layers 1 to 5."));
        assert!(!got.contains("  bash /y.sh --"));
    }

    #[test]
    fn a_clean_table_says_all_pass() {
        let got = render(&Caps::plain(80), &[row(Level::Pass, "ok", None)]);
        assert!(got.contains("All required layers pass."));
    }
}
