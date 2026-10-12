// SPDX-FileCopyrightText: 2026 Igor Santos
// SPDX-License-Identifier: Apache-2.0

//! The deterministic half of writing a handoff: the rules and template the
//! model follows, the "Where we are now" facts read from git, and the shape
//! check run before `handoff save`. The model writes only the judgment parts
//! (decisions and why, next steps, open questions).

use std::path::Path;
use std::process::Command;

/// Section headings in the required order. The last one is optional.
const SECTIONS: [&str; 4] = [
    "Where we are now",
    "Decisions made",
    "Next steps",
    "Open questions",
];

/// Sections that must appear; the last one is optional.
const REQUIRED: usize = SECTIONS.len() - 1;

/// Placeholder tokens from the template that must not survive into a handoff.
const PLACEHOLDERS: [&str; 6] = [
    "<fact>",
    "<chosen path>",
    "<reason>",
    "<action>",
    "<gap>",
    "<decision or task>",
];

/// The rules and the template `playbook handoff template` prints.
pub const TEMPLATE: &str = "\
Write for a reader who has not seen this conversation: a future session, a teammate, or the user in a week.
A handoff that needs the source conversation to be understood has failed.

Sections, in this order. Skip one with nothing real in it; do not pad.
1. Where we are now. Observable facts only. Paste the output of `playbook handoff facts`, then add the last lint, typecheck, test and build results, background work, and last verified behavior.
2. Decisions made. One line per surviving decision, with its WHY (mandatory). Record the chosen path, not the alternatives. Deferrals and reversals go here.
3. Next steps. Only concrete remaining work, in order, each with why-now or why-next. At a clean stopping point write the single line `Clean stop. No pending work.` Never list speculative enhancements.
4. Open questions. Ambiguous, deferred or risky items, each with the decision or task that surfaced it. Omit the section if nothing is open.

Style:
- Decisions and facts only. No tool narration, exploration recap, chronology, praise or speculation.
- Concrete identifiers: short SHAs, path:line, exact versions, absolute ISO dates.
- Each section under about 10 lines unless the session spanned several subsystems.
- No emojis, no em or en dashes, no preamble, no closing offer.
- No memory pointers or ledger paths unless a next step depends on them.

Template:

# Session Handoff - <one-line topic>

## Where we are now
- <fact>

## Decisions made
- <chosen path>. Why: <reason>.

## Next steps
1. <action>. Why now: <reason>.

## Open questions
- <gap>. Surfaced by: <decision or task>.

The topic line is specific (\"v0.2.0 monorepo restructure complete\"), never generic (\"Coding session\").";

/// Check a handoff document. Returns every problem found, empty when it passes.
pub fn check(text: &str) -> Vec<String> {
    let mut problems = Vec::new();
    for (n, line) in text.lines().enumerate() {
        if line.contains('\u{2014}') || line.contains('\u{2013}') {
            problems.push(format!("line {}: em or en dash", n + 1));
        }
    }
    let mut found: Vec<(usize, &str)> = Vec::new();
    let mut bodies: Vec<String> = Vec::new();
    for line in text.lines() {
        if let Some(head) = line.strip_prefix("## ") {
            let head = head.trim();
            if let Some(i) = SECTIONS.iter().position(|s| *s == head) {
                found.push((i, SECTIONS[i]));
                bodies.push(String::new());
                continue;
            }
            problems.push(format!("unknown section heading: {}", line.trim()));
            bodies.push(String::new());
            found.push((usize::MAX, "unknown"));
            continue;
        }
        if let Some(body) = bodies.last_mut() {
            body.push_str(line.trim());
        }
    }
    for (i, name) in SECTIONS.iter().enumerate().take(REQUIRED) {
        if !found.iter().any(|(j, _)| *j == i) {
            problems.push(format!("missing section: {name}"));
        }
    }
    let known: Vec<usize> = found
        .iter()
        .map(|f| f.0)
        .filter(|i| *i != usize::MAX)
        .collect();
    if known.windows(2).any(|w| w[0] >= w[1]) {
        problems.push("sections are out of order or repeated".to_string());
    }
    for ((_, name), body) in found.iter().zip(&bodies) {
        if *name == "unknown" {
            continue;
        }
        if body.is_empty() {
            problems.push(format!("empty section: {name}"));
        } else if PLACEHOLDERS.iter().any(|p| body.contains(p)) {
            problems.push(format!("unfilled placeholder in section: {name}"));
        }
    }
    problems
}

fn git(dir: &Path, args: &[&str]) -> Option<String> {
    let out = Command::new("git")
        .arg("-C")
        .arg(dir)
        .args(args)
        .output()
        .ok()?;
    out.status
        .success()
        .then(|| String::from_utf8_lossy(&out.stdout).trim_end().to_string())
}

/// The "Where we are now" facts for `dir`, as Markdown bullets.
pub fn facts(dir: &Path) -> String {
    let Some(top) = git(dir, &["rev-parse", "--show-toplevel"]) else {
        return format!("- Not a git repository: {}", dir.display());
    };
    let branch = git(dir, &["branch", "--show-current"]).unwrap_or_default();
    let branch = if branch.is_empty() {
        "(detached)".to_string()
    } else {
        branch
    };
    let head = git(dir, &["log", "-1", "--format=%h %s"]).unwrap_or_else(|| "(no commits)".into());
    let status = git(dir, &["status", "--short"]).unwrap_or_default();
    let mut out = format!("- Worktree: {top}\n- Branch: {branch}\n- HEAD: {head}\n");
    if status.is_empty() {
        out.push_str("- Working tree: clean\n");
    } else {
        out.push_str(&format!(
            "- Working tree: {} changed file(s)\n",
            status.lines().count()
        ));
        for line in status.lines().take(20) {
            out.push_str(&format!("  - {line}\n"));
        }
    }
    out.trim_end().replace(['\u{2014}', '\u{2013}'], "-")
}

#[cfg(test)]
mod tests {
    use super::*;

    const GOOD: &str = "# Session Handoff - x\n\n## Where we are now\n- a\n\n## Decisions made\n- b. Why: c.\n\n## Next steps\nClean stop. No pending work.\n";

    #[test]
    fn a_complete_handoff_passes() {
        assert!(check(GOOD).is_empty());
    }

    #[test]
    fn dashes_missing_and_empty_sections_are_named() {
        let bad = GOOD
            .replace("- a", "a \u{2014} b")
            .replace("Clean stop. No pending work.", "");
        let p = check(&bad);
        assert!(p.iter().any(|m| m.contains("dash")), "{p:?}");
        assert!(p.iter().any(|m| m == "empty section: Next steps"), "{p:?}");
        let p = check("## Next steps\n1. x\n");
        assert!(
            p.iter().any(|m| m == "missing section: Where we are now"),
            "{p:?}"
        );
    }

    #[test]
    fn out_of_order_sections_fail() {
        let t = "## Next steps\n1. x\n## Where we are now\n- a\n## Decisions made\n- b\n";
        assert!(check(t).iter().any(|m| m.contains("out of order")));
    }

    #[test]
    fn placeholders_and_unknown_headings_fail() {
        let p = check(&GOOD.replace("- a", "- <fact>"));
        assert!(p.iter().any(|m| m.contains("placeholder")), "{p:?}");
        let p = check(&GOOD.replace("## Decisions made", "## Decisions made (all)"));
        assert!(p.iter().any(|m| m.contains("unknown section")), "{p:?}");
    }

    #[test]
    fn the_template_names_every_section_heading() {
        for name in SECTIONS {
            assert!(TEMPLATE.contains(&format!("## {name}")), "{name}");
        }
    }

    #[test]
    fn facts_report_a_repo_branch_and_unstaged_status() {
        let dir = crate::common::test_support::scratch_dir("facts");
        std::fs::create_dir_all(&dir).unwrap();
        let run = |args: &[&str]| {
            Command::new("git")
                .arg("-C")
                .arg(&dir)
                .args(args)
                .output()
                .unwrap();
        };
        run(&["init", "-q"]);
        run(&[
            "-c",
            "user.name=t",
            "-c",
            "user.email=t@t",
            "commit",
            "-q",
            "--allow-empty",
            "-m",
            "first",
        ]);
        std::fs::write(dir.join("a.txt"), "x").unwrap();
        run(&["add", "a.txt"]);
        run(&[
            "-c",
            "user.name=t",
            "-c",
            "user.email=t@t",
            "commit",
            "-q",
            "-m",
            "second",
        ]);
        std::fs::write(dir.join("a.txt"), "y").unwrap();
        let f = facts(&dir);
        assert!(f.contains("- Branch: ") && f.contains("second"), "{f}");
        assert!(
            f.contains("1 changed file(s)") && f.contains("  -  M a.txt"),
            "{f}"
        );
    }

    #[test]
    fn the_template_has_no_dashes_of_its_own() {
        assert!(!TEMPLATE.contains('\u{2014}') && !TEMPLATE.contains('\u{2013}'));
    }
}
