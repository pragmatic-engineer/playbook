// SPDX-FileCopyrightText: 2026 Igor Santos
// SPDX-License-Identifier: Apache-2.0

//! `playbook pr stack <pr> --context`: one shared context for a whole-stack
//! review. Open PRs are read in full (a diff file each). Merged PRs are read
//! in summary form (title, description, file list). Closed PRs are skipped.

use super::stack::{Gh, Limits, Stack, StackPr, State};
use crate::common::par;
use serde_json::Value;
use std::fs;
use std::path::PathBuf;

/// Longest PR description kept in the context file, in characters.
const BODY_MAX: usize = 1500;
/// Most changed files listed for one PR.
const FILES_MAX: usize = 40;

/// What one PR contributes to the context.
struct Fetched {
    body: String,
    files: Vec<(String, u64, u64)>,
    diff: Option<String>,
}

fn fetch(run: Gh, pr: &StackPr) -> Result<Fetched, String> {
    let n = pr.number.to_string();
    let raw = run(&["pr", "view", &n, "--json", "body,files"])?;
    let v: Value =
        serde_json::from_str(&raw).map_err(|e| format!("could not read PR #{n}: {e}"))?;
    let files = v["files"]
        .as_array()
        .map(|a| {
            a.iter()
                .map(|f| {
                    (
                        f["path"].as_str().unwrap_or("").to_string(),
                        f["additions"].as_u64().unwrap_or(0),
                        f["deletions"].as_u64().unwrap_or(0),
                    )
                })
                .collect()
        })
        .unwrap_or_default();
    let diff = if pr.state == State::Open {
        Some(run(&["pr", "diff", &n])?)
    } else {
        None
    };
    Ok(Fetched {
        body: v["body"].as_str().unwrap_or("").to_string(),
        files,
        diff,
    })
}

fn trim_body(body: &str) -> String {
    let body = body.trim();
    if body.chars().count() <= BODY_MAX {
        return body.to_string();
    }
    let cut: String = body.chars().take(BODY_MAX).collect();
    format!("{cut}\n[description cut at {BODY_MAX} characters]")
}

/// Fetch every PR that needs reading and write the context directory.
/// Returns the report lines the review command reads.
pub fn write(run: Gh, stack: &Stack, limits: Limits) -> Result<String, String> {
    let dir: PathBuf = std::env::temp_dir().join(format!("pr-stack-{}", stack.current));
    fs::create_dir_all(&dir).map_err(|e| format!("could not create {}: {e}", dir.display()))?;
    let wanted: Vec<&StackPr> = stack
        .prs
        .iter()
        .filter(|p| p.state != State::Closed)
        .collect();
    let fetched = par::map(&wanted, par::MAX_CONCURRENT, |p| fetch(run, p));

    let mut doc = format!(
        "# Stack review context: {} PRs in {}\n\nPRs are listed bottom first. Each PR targets the branch of the PR above it in this list. Findings go only to PRs marked REVIEW. PRs marked CONTEXT are merged: read them to understand the code, and never review or comment on them.\n\n",
        stack.prs.len(),
        stack.repo
    );
    let mut lines = Vec::new();
    for (i, p) in stack.prs.iter().enumerate() {
        let role = match p.state {
            State::Open => "REVIEW",
            State::Merged => "CONTEXT (merged)",
            State::Closed => "SKIPPED (closed without merging)",
        };
        doc.push_str(&format!(
            "## {}. #{} {} [{role}]\n\nbranch `{}` into `{}`, +{} -{}, {} files\n\n",
            i + 1,
            p.number,
            p.title,
            p.head,
            p.base,
            p.additions,
            p.deletions,
            p.changed_files
        ));
        let Some(pos) = wanted.iter().position(|w| w.number == p.number) else {
            continue;
        };
        let f = match &fetched[pos] {
            Ok(f) => f,
            Err(e) => return Err(format!("error: could not read PR #{}: {e}", p.number)),
        };
        let body = trim_body(&f.body);
        if !body.is_empty() {
            doc.push_str(&format!("{body}\n\n"));
        }
        if p.state == State::Merged {
            doc.push_str("Files:\n");
            for (path, add, del) in f.files.iter().take(FILES_MAX) {
                doc.push_str(&format!("- {path} (+{add} -{del})\n"));
            }
            if f.files.len() > FILES_MAX {
                doc.push_str(&format!("- and {} more files\n", f.files.len() - FILES_MAX));
            }
            doc.push('\n');
        }
        if let Some(diff) = &f.diff {
            let path = dir.join(format!("pr-{}.diff", p.number));
            fs::write(&path, diff)
                .map_err(|e| format!("could not write {}: {e}", path.display()))?;
            doc.push_str(&format!("Diff: {}\n\n", path.display()));
            lines.push(format!("diff_file_{}={}", p.number, path.display()));
        }
    }
    let context_path = dir.join("context.md");
    fs::write(&context_path, doc)
        .map_err(|e| format!("could not write {}: {e}", context_path.display()))?;
    let stack_path = dir.join("stack.json");
    let json = serde_json::to_string_pretty(&stack.to_json(limits)).map_err(|e| e.to_string())?;
    fs::write(&stack_path, json)
        .map_err(|e| format!("could not write {}: {e}", stack_path.display()))?;

    let j = stack.to_json(limits);
    let mut out = vec![
        format!("context_file={}", context_path.display()),
        format!("stack_file={}", stack_path.display()),
        format!(
            "open={} merged={} open_lines={} over_budget={}",
            j["open_count"], j["merged_count"], j["open_lines"], j["over_budget"]
        ),
    ];
    out.extend(lines);
    Ok(out.join("\n"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::pr::stack::{detect, Limits};

    fn gh(args: &[&str]) -> Result<String, String> {
        let fx = |n: &str| {
            std::fs::read_to_string(
                std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
                    .join("tests/fixtures/pr_stack")
                    .join(n),
            )
            .map_err(|e| e.to_string())
        };
        match (args[0], args[1]) {
            ("repo", _) => fx("repo.json"),
            ("api", _) => fx("api-middle-merged.json"),
            ("pr", "view") => Ok(format!(
                r#"{{"body":"Body of {n}","files":[{{"path":"src/f{n}.rs","additions":3,"deletions":1}}]}}"#,
                n = args[2]
            )),
            ("pr", "diff") => Ok(format!(
                "diff --git a/f b/f\n+++ b/f\n@@ -1 +1 @@\n+PR {}",
                args[2]
            )),
            _ => Err("unexpected".into()),
        }
    }

    #[test]
    fn open_prs_get_a_diff_file_and_merged_prs_only_a_summary() {
        let stack = detect(&gh, Some(3)).unwrap();
        let out = write(&gh, &stack, Limits::default()).unwrap();
        let ctx = out
            .lines()
            .find_map(|l| l.strip_prefix("context_file="))
            .unwrap();
        let doc = std::fs::read_to_string(ctx).unwrap();
        assert!(doc.contains("## 1. #1 PR 1 [CONTEXT (merged)]"), "{doc}");
        assert!(doc.contains("- src/f1.rs (+3 -1)"), "{doc}");
        assert!(doc.contains("## 3. #3 PR 3 [REVIEW]"), "{doc}");
        assert!(
            out.contains("diff_file_3=") && out.contains("diff_file_4="),
            "{out}"
        );
        assert!(!out.contains("diff_file_1="), "{out}");
        assert!(out.contains("open=2 merged=2"), "{out}");
    }

    #[test]
    fn a_long_description_is_cut() {
        let long = "x".repeat(BODY_MAX + 10);
        assert!(trim_body(&long).contains("description cut"));
        assert_eq!(trim_body("  short  "), "short");
    }
}
