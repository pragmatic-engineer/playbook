// SPDX-FileCopyrightText: 2026 Igor Santos
// SPDX-License-Identifier: Apache-2.0

//! The closing steps that stay allowed once the budget guard parks a caller:
//! the skills and commands that deliver finished work (commit, push, open a
//! pull request) and read-only checks of where the work stands. Anything that
//! starts new work (an edit, a build, a merge, a release, an `Agent` spawn)
//! is not on the list.
//!
//! A Bash call counts only when every simple command in it is on the list,
//! so `git status && rm -rf x` is denied. A word that only the shell can
//! evaluate (`$VAR`, `$(...)`, a backtick) is denied, since what it runs is
//! unknown here. This is a guardrail against drift, not a security boundary.

use crate::common::shell::{commands, program_name, Command};

/// The skills a closing step loads: the two delivery skills and the ones they
/// depend on.
const CLOSING_SKILLS: [&str; 4] = [
    "playbook:create-pull-request",
    "playbook:commit-and-push",
    "playbook:writing-style",
    "playbook:engineering-standards",
];

/// Whether a `Skill` call names a closing skill.
pub fn is_closing_skill(skill: &str) -> bool {
    CLOSING_SKILLS.contains(&skill.trim())
}

/// Whether every simple command of `command` is a closing step.
pub fn is_closing_command(command: &str) -> bool {
    // The shell reader treats `&` as a separator, so the stream merges are
    // taken out first. Quoted text that holds one loses it, which is harmless.
    let plain = ["2>&1", "1>&2", ">&2", "&>/dev/null"]
        .iter()
        .fold(command.to_string(), |text, merge| text.replace(merge, ""));
    let parsed = commands(&plain);
    !parsed.is_empty() && parsed.iter().all(is_closing_step)
}

fn is_closing_step(command: &Command) -> bool {
    if command.spans.iter().any(|span| span.dynamic) {
        return false;
    }
    let Some(words) = without_redirects(&command.words) else {
        return false;
    };
    let Some((program, args)) = words.split_first() else {
        return false;
    };
    match program_name(program).as_str() {
        "git" => is_git_step(args),
        "gh" => is_gh_read(args),
        "playbook" => is_playbook_step(args),
        // Output of a closing step may be trimmed.
        "head" | "tail" | "wc" => command.piped,
        _ => false,
    }
}

/// The words with the redirections removed, or `None` when one writes or reads
/// a file. Sending output to `/dev/null` or to the other stream, and a
/// heredoc, are fine.
fn without_redirects(words: &[String]) -> Option<Vec<&str>> {
    let mut kept = Vec::new();
    let mut rest = words.iter().map(String::as_str);
    while let Some(word) = rest.next() {
        let operator = word.trim_start_matches(|c: char| c.is_ascii_digit());
        if !operator.starts_with(['<', '>']) {
            kept.push(word);
            continue;
        }
        let symbols = operator.trim_start_matches(['<', '>']);
        let target = if symbols.is_empty() {
            rest.next()?
        } else {
            symbols
        };
        let is_heredoc = operator.starts_with("<<") && !operator.starts_with("<<<");
        if !is_heredoc && !matches!(target, "/dev/null" | "&1" | "&2") {
            return None;
        }
    }
    Some(kept)
}

/// `git` with at most `-C <dir>` before the subcommand.
fn is_git_step(args: &[&str]) -> bool {
    let args = match args {
        ["-C", _, rest @ ..] => rest,
        rest => rest,
    };
    match args {
        [] => false,
        ["add" | "commit" | "status" | "log", ..] => true,
        ["diff", rest @ ..] => !rest.iter().any(|arg| arg.starts_with("--output")),
        ["worktree", "list", ..] => true,
        ["push", rest @ ..] => !is_destructive_push(rest),
        _ => false,
    }
}

/// A push that forces, deletes or mirrors. Closing a task never needs one,
/// and the other hooks keep guarding protected branches.
fn is_destructive_push(args: &[&str]) -> bool {
    args.iter().any(|arg| {
        let flag = arg.starts_with('-') && !arg.starts_with("--");
        arg.starts_with("--force")
            || matches!(*arg, "--delete" | "--mirror" | "--prune")
            || (flag && arg.contains(['f', 'd']))
            || arg.starts_with(['+', ':'])
    })
}

/// Read-only `gh`: viewing and listing pull requests and runs, and `api`
/// without a method other than GET or any body.
fn is_gh_read(args: &[&str]) -> bool {
    match args {
        ["pr", "view" | "list" | "checks" | "diff" | "status", ..] => true,
        ["run", "view" | "list", ..] => true,
        ["api", rest @ ..] => is_get_api(rest),
        _ => false,
    }
}

fn is_get_api(args: &[&str]) -> bool {
    let mut at = 0;
    while let Some(arg) = args.get(at) {
        let lower = arg.to_lowercase();
        let method = |value: &str| value.eq_ignore_ascii_case("GET");
        match lower.as_str() {
            "-x" | "--method" => {
                if !args.get(at + 1).is_some_and(|value| method(value)) {
                    return false;
                }
                at += 1;
            }
            "--input" | "--field" | "--raw-field" => return false,
            _ if arg.starts_with("-X") => {
                if !method(&arg[2..]) {
                    return false;
                }
            }
            _ if arg.starts_with("--method=") => {
                if !method(&arg["--method=".len()..]) {
                    return false;
                }
            }
            _ if arg.starts_with("--input=")
                || arg.starts_with("--field=")
                || arg.starts_with("--raw-field=") =>
            {
                return false;
            }
            _ if *arg == "-f" || *arg == "-F" => return false,
            _ => {}
        }
        at += 1;
    }
    true
}

/// The `playbook` commands the commit and pull request skills run, and the
/// read-only status and config reads.
fn is_playbook_step(args: &[&str]) -> bool {
    matches!(
        args,
        ["pr", "create" | "prepare" | "rules", ..]
            | ["commit", "run" | "prepare", ..]
            | ["mode", "status"]
            | ["mode", "status", "--json"]
            | ["config", "get", ..]
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_listed_command_is_a_closing_step_and_new_work_is_not() {
        let closing = [
            "git add -A",
            "git add src/a.rs docs/x.md",
            "git commit -S -s -m \"feat: x\"",
            "git push origin feat/x",
            "git push -u origin feat/x",
            "git -C ../wt status --short",
            "git status",
            "git diff HEAD~1 --stat",
            "git log --oneline -5",
            "git worktree list",
            "gh pr view 12 --json state",
            "gh pr list --state open",
            "gh pr checks 12",
            "gh run view 99 --log-failed",
            "gh api repos/o/r/pulls/12",
            "gh api -X GET repos/o/r/pulls",
            "playbook pr create --title \"fix: x\" --body-file b.md",
            "playbook commit run --auto",
            "playbook pr prepare --base main",
            "playbook mode status",
            "playbook mode status --json",
            "git status && git diff",
            "git log --oneline 2>&1 | head -5",
            "playbook commit run <<'MSG'\nfeat: x\nMSG",
        ];
        let new_work = [
            "cargo build",
            "npm test",
            "rm -rf target",
            "gh pr merge 12 --auto",
            "gh release create v1",
            "gh api -X POST repos/o/r/issues",
            "gh api repos/o/r/issues -f title=x",
            "playbook mode ask",
            "playbook config set auto.budgetUsd 50",
            "git status && rm -rf x",
            "git push --force origin main",
            "git push -f",
            "git push --force-with-lease",
            "git push origin :main",
            "git push origin +main",
            "git push --delete origin x",
            "git reset --hard",
            "git checkout main",
            "git commit -m \"$(cat <<'EOF'\nx\nEOF\n)\"",
            "git status > out.txt",
            "git diff --output=x",
            "echo hi | head",
            "head -5 file",
            "",
        ];
        for command in closing {
            assert!(is_closing_command(command), "{command:?} must be allowed");
        }
        for command in new_work {
            assert!(!is_closing_command(command), "{command:?} must be denied");
        }
    }

    #[test]
    fn only_the_delivery_skills_and_their_dependencies_are_closing_skills() {
        for skill in [
            "playbook:create-pull-request",
            "playbook:commit-and-push",
            "playbook:writing-style",
            "playbook:engineering-standards",
        ] {
            assert!(is_closing_skill(skill), "{skill}");
        }
        for skill in ["playbook:implement", "playbook:fix", "commit-and-push", ""] {
            assert!(!is_closing_skill(skill), "{skill}");
        }
    }
}
