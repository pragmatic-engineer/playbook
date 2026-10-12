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
    let plain = without_stream_merges(command);
    let parsed = commands(&plain);
    !parsed.is_empty() && parsed.iter().all(is_closing_step)
}

/// The text without the stream merges (`2>&1`, `1>&2`, `>&2`, `&>/dev/null`)
/// that sit outside quotes. The shell reader treats `&` as a separator, so
/// they are taken out first. Quoted text is left as it is.
fn without_stream_merges(command: &str) -> String {
    const MERGES: [&str; 4] = ["2>&1", "1>&2", ">&2", "&>/dev/null"];
    let mut out = String::with_capacity(command.len());
    let mut quote: Option<char> = None;
    let mut rest = command;
    while let Some(c) = rest.chars().next() {
        if quote.is_none() && c != '\\' {
            if let Some(merge) = MERGES.iter().find(|m| rest.starts_with(**m)) {
                rest = &rest[merge.len()..];
                continue;
            }
        }
        match (quote, c) {
            (None, '\'' | '"') => quote = Some(c),
            (Some(open), _) if open == c => quote = None,
            _ => {}
        }
        let width = if c == '\\' && quote != Some('\'') {
            c.len_utf8()
                + rest[c.len_utf8()..]
                    .chars()
                    .next()
                    .map_or(0, char::len_utf8)
        } else {
            c.len_utf8()
        };
        out.push_str(&rest[..width]);
        rest = &rest[width..];
    }
    out
}

fn is_closing_step(command: &Command) -> bool {
    if command.spans.iter().any(|span| span.dynamic) {
        return false;
    }
    // An unquoted heredoc expands `$` and backticks in its body.
    if command
        .heredocs
        .iter()
        .any(|doc| !doc.quoted && doc.text.contains(['$', '`']))
    {
        return false;
    }
    if is_pr_body_write(command) {
        return true;
    }
    let Some(words) = without_redirects(&command.words) else {
        return false;
    };
    let words = unwrap_group(&words);
    // A lone `}` closes a group the skill's version probe opened.
    if words.is_empty() && command.words.first().is_some_and(|word| word == "}") {
        return true;
    }
    let Some((program, args)) = words.split_first() else {
        return false;
    };
    match program_name(program).as_str() {
        "git" => is_git_step(args),
        "gh" => is_gh_read(args),
        "playbook" => is_playbook_step(args),
        // The skills' version probe ends in `|| { echo "..." >&2; exit 1; }`.
        "echo" | "exit" | "true" => true,
        // Output of a closing step may be trimmed.
        "head" | "tail" | "wc" => command.piped,
        _ => false,
    }
}

/// The words without a leading `{` or a trailing `}` of a command group.
fn unwrap_group<'a>(words: &[&'a str]) -> Vec<&'a str> {
    let mut words = words;
    while let Some((&"{", rest)) = words.split_first() {
        words = rest;
    }
    while let Some((&"}", rest)) = words.split_last() {
        words = rest;
    }
    words.to_vec()
}

/// `cat > <dir>/pr-body.md <<'EOF'`: the one file write the pull request
/// skill makes before `playbook pr create`. The body must be a quoted heredoc.
fn is_pr_body_write(command: &Command) -> bool {
    let words: Vec<&str> = command.words.iter().map(String::as_str).collect();
    let [cat, ">", target, ..] = words.as_slice() else {
        return false;
    };
    let rest = &words[3..];
    program_name(cat) == "cat"
        && target.ends_with("/pr-body.md")
        && !target.contains("..")
        && command.heredocs.iter().all(|doc| doc.quoted)
        && rest.iter().all(|word| word.starts_with("<<"))
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
            // A redirect glued to a word, as in `status>out.txt`.
            if word.contains(['<', '>']) {
                return None;
            }
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
        ["add" | "commit" | "status", ..] => true,
        ["diff" | "log", rest @ ..] => !rest.iter().any(|arg| arg.starts_with("--output")),
        ["worktree", "list", ..] => true,
        ["push", rest @ ..] => is_safe_push(rest),
        _ => false,
    }
}

/// Branches a closing step never pushes to.
const PROTECTED_BRANCHES: [&str; 4] = ["main", "master", "trunk", "develop"];

/// A push of a named branch to a remote. It must not force, delete, mirror,
/// push every branch, run a program of its choosing, or target a protected
/// branch. A push with no explicit branch is denied too, since it may push
/// the protected branch the work tree is on.
fn is_safe_push(args: &[&str]) -> bool {
    let unsafe_flag = args.iter().any(|arg| {
        let short = arg.starts_with('-') && !arg.starts_with("--");
        arg.starts_with("--force")
            || ["--delete", "--mirror", "--prune", "--all", "--tags"].contains(arg)
            || ["--receive-pack", "--exec", "--repo"]
                .iter()
                .any(|flag| arg.starts_with(flag))
            || (short && arg.contains(['f', 'd']))
            || arg.starts_with(['+', ':'])
    });
    let positional: Vec<&str> = args
        .iter()
        .copied()
        .filter(|arg| !arg.starts_with('-'))
        .collect();
    let [_remote, refspecs @ ..] = positional.as_slice() else {
        return false;
    };
    !unsafe_flag
        && !refspecs.is_empty()
        && refspecs.iter().all(|spec| {
            let destination = spec.rsplit(':').next().unwrap_or(spec);
            let branch = destination
                .strip_prefix("refs/heads/")
                .unwrap_or(destination);
            !branch.is_empty() && branch != "HEAD" && !PROTECTED_BRANCHES.contains(&branch)
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

/// `gh api` that only reads. A short flag with its value attached (`-fkey=v`,
/// `-Fkey=v`, `-XPOST`) or in a cluster (`-if`) is judged like the spaced
/// form, and `graphql` is a POST by default.
fn is_get_api(args: &[&str]) -> bool {
    let mut at = 0;
    while let Some(&arg) = args.get(at) {
        let is_get = |value: &str| value == "GET";
        if arg == "graphql" {
            return false;
        }
        if let Some(long) = arg.strip_prefix("--") {
            let (name, value) = long.split_once('=').unwrap_or((long, ""));
            match name {
                "input" | "field" | "raw-field" => return false,
                "method" => {
                    let value = if long.contains('=') {
                        Some(value)
                    } else {
                        at += 1;
                        args.get(at).copied()
                    };
                    if !value.is_some_and(is_get) {
                        return false;
                    }
                }
                _ => {}
            }
        } else if let Some(short) = arg.strip_prefix('-').filter(|short| !short.is_empty()) {
            if short.contains(['f', 'F']) {
                return false;
            }
            if let Some(method) = short.strip_prefix('X') {
                let method = if method.is_empty() {
                    at += 1;
                    args.get(at).copied().unwrap_or_default()
                } else {
                    method
                };
                if !is_get(method) {
                    return false;
                }
            } else if short.contains('X') {
                return false;
            }
        }
        at += 1;
    }
    true
}

/// The `playbook` commands the commit and pull request skills run, and the
/// read-only status and config reads. `commit run` must carry `--auto`:
/// without it the command may force push after an amend or a rebase.
fn is_playbook_step(args: &[&str]) -> bool {
    match args {
        ["commit", "run", rest @ ..] => rest.contains(&"--auto"),
        ["pr", "create" | "prepare" | "rules", ..]
        | ["commit", "prepare", ..]
        | ["mode", "status"]
        | ["mode", "status", "--json"]
        | ["config", "get", ..] => true,
        _ => false,
    }
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
            "git push origin HEAD:refs/heads/feat/x",
            "git log --oneline 2>&1 | head -5",
            "playbook pr prepare --help >/dev/null 2>&1 || { echo \"ERROR: upgrade\" >&2; exit 1; }",
            "cat > \"/tmp/state/pr-body.md\" <<'PRBODY_EOF'\nbody $x `y`\nPRBODY_EOF\nplaybook pr create --title \"fix: x\" --body-file /tmp/state/pr-body.md",
            "git commit -m \"feat: a b\" 2>&1",
            "playbook pr prepare --base main",
            "playbook mode status",
            "playbook mode status --json",
            "git status && git diff",
            "git log --oneline 2>&1 | head -5",
            "playbook commit run --auto <<'MSG'\nfeat: x\nMSG",
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
            "gh api repos/o/r/issues -fkey=v",
            "gh api repos/o/r/issues -Fkey=v",
            "gh api repos/o/r/issues -XPOST",
            "gh api repos/o/r/issues -if",
            "gh api graphql",
            "gh api repos/o/r --method=POST",
            "cat <<EOF | playbook commit run --auto\n$HOME\nEOF",
            "playbook commit run --auto <<EOF\n$(id)\nEOF",
            "git status>out.txt",
            "git log>>out.txt",
            "git log --output=x",
            "git log --output x",
            "git push --receive-pack=evil origin feat/x",
            "git push --exec=evil origin feat/x",
            "git push origin main",
            "git push origin HEAD:main",
            "git push origin feat/x:refs/heads/master",
            "git push",
            "git push origin",
            "git push --all origin",
            "git push origin HEAD",
            "playbook commit run",
            "playbook commit run -a",
            "git commit -m \"x\" \"$(echo 2>&1)\"",
            "git status '2>&1' > out.txt",
            "git status \"2>&1\" > out.txt",
            "cat > /tmp/other.txt <<'EOF'\nx\nEOF",
            "cat > ../pr-body.md <<'EOF'\nx\nEOF",
            "cat > /tmp/pr-body.md <<EOF\n$(id)\nEOF",
            "echo hi > out.txt",
            // A word that holds a redirect symbol is denied, quoted or not.
            "git commit -m \"a 2>&1 b\"",
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
