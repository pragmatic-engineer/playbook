// SPDX-FileCopyrightText: 2026 Igor Santos
// SPDX-License-Identifier: Apache-2.0

//! `playbook prose check <files>`: the prose lint of the writing-style skill
//! run on files, for docs and ADRs. The same table backs the
//! `commit-message-sanitizer` hook.

use crate::common::prose_lint::{has_hard, lint, Finding};
use std::fs;
use std::path::Path;

/// What a run found.
pub struct Report {
    /// The lines to print.
    pub lines: Vec<String>,
    /// Findings of every kind.
    pub total: usize,
    /// Findings of a hard class.
    pub hard: usize,
}

fn line_for(path: &Path, finding: &Finding, github: bool) -> String {
    if github {
        format!(
            "::warning file={},line={}::{}",
            path.display(),
            finding.line,
            finding.render()
        )
    } else {
        format!(
            "{}:{}",
            path.display(),
            finding.render().replacen("line ", "", 1)
        )
    }
}

/// Lint every file. `github` prints GitHub Actions annotations.
pub fn check(files: &[std::path::PathBuf], github: bool) -> Result<Report, String> {
    let mut report = Report {
        lines: Vec::new(),
        total: 0,
        hard: 0,
    };
    for path in files {
        let text = fs::read_to_string(path)
            .map_err(|e| format!("could not read {}: {e}", path.display()))?;
        let findings = lint(&text);
        report.total += findings.len();
        if has_hard(&findings) {
            report.hard += findings.iter().filter(|f| f.class.hard()).count();
        }
        report
            .lines
            .extend(findings.iter().map(|f| line_for(path, f, github)));
    }
    Ok(report)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::common::test_support::scratch_dir;

    fn file(tag: &str, text: &str) -> std::path::PathBuf {
        let dir = scratch_dir(tag);
        fs::create_dir_all(&dir).unwrap();
        let path = dir.join("a.md");
        fs::write(&path, text).unwrap();
        path
    }

    #[test]
    fn a_clean_file_has_no_lines() {
        let path = file("prose-clean", "# Title\n\nThe cache is cold.\n");

        let report = check(&[path], false).unwrap();

        assert!(report.lines.is_empty());
        assert_eq!((report.total, report.hard), (0, 0));
    }

    #[test]
    fn a_finding_prints_as_path_line_message() {
        let path = file("prose-hit", "# Title\n\nLet that sink in.\n");

        let report = check(std::slice::from_ref(&path), false).unwrap();

        assert_eq!(report.hard, 1);
        let expected = format!("{}:3: emphasis crutch", path.display());
        assert!(report.lines[0].starts_with(&expected), "{:?}", report.lines);
    }

    #[test]
    fn github_mode_prints_a_warning_annotation() {
        let path = file("prose-gh", "Let that sink in.\n");

        let report = check(&[path], true).unwrap();

        assert!(report.lines[0].starts_with("::warning file="));
        assert!(report.lines[0].contains(",line=1::"));
    }

    #[test]
    fn a_soft_finding_counts_but_is_not_hard() {
        let path = file("prose-soft", "It always fails.\n");

        let report = check(&[path], false).unwrap();

        assert_eq!((report.total, report.hard), (1, 0));
    }

    #[test]
    fn a_missing_file_is_an_error() {
        let err = check(&[std::path::PathBuf::from("/nonexistent/x.md")], false).err();

        assert!(err.is_some_and(|e| e.contains("could not read")));
    }
}
