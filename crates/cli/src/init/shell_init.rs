// SPDX-FileCopyrightText: 2026 Igor Santos
// SPDX-License-Identifier: Apache-2.0

//! `playbook shell-init`: retired by ADR 0044. It prints one comment line, so
//! an rc file that still runs `eval "$(playbook shell-init)"` defines nothing
//! and starts without an error. `playbook migrate` offers to remove that line.

use super::shim::ShellKind;

/// The text `shell-init` prints: a comment, valid in bash and zsh alike.
pub fn script(shell: ShellKind) -> String {
    let name = match shell {
        ShellKind::Bash => "bash",
        ShellKind::Zsh => "zsh",
    };
    format!(
        "# playbook shell-init ({name}) is retired and defines nothing; \
         `playbook migrate` removes this rc line (ADR 0044)\n"
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn both_shells_get_one_comment_line_and_no_function() {
        for kind in [ShellKind::Bash, ShellKind::Zsh] {
            // Arrange and act
            let text = script(kind);

            // Assert
            assert_eq!(text.lines().count(), 1, "{text}");
            assert!(text.starts_with("# "), "{text}");
            assert!(!text.contains("()"), "{text}");
        }
    }
}
