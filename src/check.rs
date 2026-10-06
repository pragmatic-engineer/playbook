// SPDX-FileCopyrightText: 2026 Igor Santos
// SPDX-License-Identifier: MIT

//! `playbook check commit-msg <file>`: the shared pristine-message check for
//! callers that are not an agent hook, such as a git `commit-msg` hook or CI.

use crate::common::attribution::commit_message_problems;
use std::path::Path;

/// The line git prints above the diff that `git commit -v` appends.
const SCISSORS: &str = "# ------------------------ >8 ------------------------";

/// Problems in a commit message file, one plain sentence each. Empty means the
/// message is pristine.
pub fn commit_msg(path: &Path) -> Result<Vec<String>, String> {
    let raw = std::fs::read_to_string(path)
        .map_err(|e| format!("could not read commit message file {}: {e}", path.display()))?;
    Ok(commit_message_problems(&without_template(&raw)))
}

/// The message as git would store it by default: comment lines are dropped
/// and everything from the scissors line down is cut. Dropped lines become
/// blank, so the line numbers in a problem still match the file.
fn without_template(raw: &str) -> String {
    let kept = raw.lines().take_while(|line| *line != SCISSORS);
    kept.map(|line| if line.starts_with('#') { "" } else { line })
        .collect::<Vec<_>>()
        .join("\n")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::common::test_support::scratch_dir;
    use std::fs;

    fn problems_for(text: &str) -> Result<Vec<String>, String> {
        let dir = scratch_dir("check-commit-msg");
        fs::create_dir_all(&dir).expect("scratch dir");
        let file = dir.join("COMMIT_EDITMSG");
        fs::write(&file, text).expect("message file");
        commit_msg(&file)
    }

    #[test]
    fn a_pristine_message_has_no_problems() {
        let got = problems_for("feat: x\n\nbody\n\nRefs: PLAT-1\nSigned-off-by: A <a@b.c>\n");

        assert_eq!(got, Ok(Vec::new()));
    }

    #[test]
    fn each_problem_is_reported() {
        let got = problems_for("feat: x\n\nbody\n\nClaude-Session: u\n");

        assert_eq!(
            got.map(|p| p.len()),
            Ok(2),
            "one attribution line and one trailer"
        );
    }

    #[test]
    fn comment_lines_are_ignored() {
        let got = problems_for("feat: x\n\nRefs: 1\n# Generated with Claude Code\n");

        assert_eq!(got, Ok(Vec::new()));
    }

    #[test]
    fn everything_below_the_scissors_line_is_ignored() {
        let text = format!("feat: x\n\nRefs: 1\n{SCISSORS}\n+Claude-Session: in a diff\n");

        assert_eq!(problems_for(&text), Ok(Vec::new()));
    }

    #[test]
    fn a_missing_file_is_an_error_naming_the_path() {
        let err = commit_msg(Path::new("/nonexistent/COMMIT_EDITMSG")).unwrap_err();

        assert!(err.contains("/nonexistent/COMMIT_EDITMSG"), "{err}");
    }
}
