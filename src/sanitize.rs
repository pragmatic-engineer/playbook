// SPDX-FileCopyrightText: 2026 Igor Santos
// SPDX-License-Identifier: MIT

//! `playbook sanitize`: removes AI attribution from a message file for callers
//! that are not an agent hook, such as a git `commit-msg` hook or CI.
//!
//! By default the file is rewritten in place. With `--check` it is left alone
//! and the caller learns whether anything would be removed. Either way, only
//! the line number and the shape of each removed line are reported.

use crate::common::atomic::write_atomic;
use crate::common::attribution::{sanitize_editor, sanitize_prose, Sanitized};
use std::path::Path;

/// What kind of text the file holds.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    /// A commit message as git's editor leaves it, template included.
    CommitMsg,
    /// A PR title or body: free text with no trailer block.
    PrText,
}

/// One printable line for each removed line of the file at `path`. With
/// `check_only` nothing is written; otherwise a changed file is rewritten
/// atomically.
pub fn run(kind: Kind, path: &Path, check_only: bool) -> Result<Vec<String>, String> {
    let raw = std::fs::read_to_string(path)
        .map_err(|e| format!("could not read {}: {e}", path.display()))?;
    let sanitized = match kind {
        Kind::CommitMsg => sanitize_editor(&raw),
        Kind::PrText => sanitize_prose(&raw),
    };
    if !check_only && !sanitized.removed.is_empty() {
        write_atomic(path, &sanitized.text)
            .map_err(|e| format!("could not rewrite {}: {e}", path.display()))?;
    }
    Ok(report(&sanitized, check_only))
}

fn report(sanitized: &Sanitized, check_only: bool) -> Vec<String> {
    let verb = if check_only {
        "would remove"
    } else {
        "removed"
    };
    sanitized
        .removed
        .iter()
        .map(|(n, shape)| format!("sanitize: {verb} line {n} ({})", shape.name()))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::common::test_support::scratch_dir;
    use std::fs;

    fn file_with(text: &str) -> std::path::PathBuf {
        let dir = scratch_dir("sanitize");
        fs::create_dir_all(&dir).expect("scratch dir");
        let file = dir.join("COMMIT_EDITMSG");
        fs::write(&file, text).expect("message file");
        file
    }

    #[test]
    fn the_default_rewrites_the_file_and_reports_each_line() {
        let file = file_with("feat: x\n\nbody\n\nCo-Authored-By:Claude <a@b.c>\n");

        let got = run(Kind::CommitMsg, &file, false);

        assert_eq!(
            got,
            Ok(vec![
                "sanitize: removed line 5 (credit trailer naming an AI)".to_string()
            ])
        );
        assert_eq!(fs::read_to_string(&file).unwrap(), "feat: x\n\nbody\n");
    }

    #[test]
    fn check_reports_but_leaves_the_file_alone() {
        let text = "feat: x\n\nbody\n\nGenerated with Claude Code\n";
        let file = file_with(text);

        let got = run(Kind::CommitMsg, &file, true);

        assert_eq!(
            got,
            Ok(vec![
                "sanitize: would remove line 5 (generated-with footer)".to_string()
            ])
        );
        assert_eq!(fs::read_to_string(&file).unwrap(), text);
    }

    #[test]
    fn a_clean_file_is_not_rewritten_and_reports_nothing() {
        let file = file_with("feat: x\n\nRefs: PLAT-1\n");
        let before = fs::metadata(&file).unwrap().modified().unwrap();

        let got = run(Kind::CommitMsg, &file, false);

        assert_eq!(got, Ok(Vec::new()));
        assert_eq!(fs::metadata(&file).unwrap().modified().unwrap(), before);
    }

    #[test]
    fn a_report_never_carries_the_removed_text() {
        let file = file_with("feat: x\n\nbody\n\nClaude-Session: topsecret-value\n");

        let lines = run(Kind::CommitMsg, &file, true).unwrap();

        assert!(
            lines.iter().all(|l| !l.contains("topsecret-value")),
            "{lines:?}"
        );
    }

    #[test]
    fn pr_text_has_no_trailer_block() {
        let file = file_with("## Summary\n\nClaude-Model: opus\n");

        assert_eq!(run(Kind::PrText, &file, true), Ok(Vec::new()));
    }

    #[test]
    fn a_missing_file_is_an_error_naming_the_path() {
        let err = run(
            Kind::CommitMsg,
            Path::new("/nonexistent/COMMIT_EDITMSG"),
            true,
        )
        .unwrap_err();

        assert!(err.contains("/nonexistent/COMMIT_EDITMSG"), "{err}");
    }
}
