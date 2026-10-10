// SPDX-FileCopyrightText: 2026 Igor Santos
// SPDX-License-Identifier: Apache-2.0

//! `playbook pr rules`: Step 0 of `/playbook:create-pull-request`. It cuts the
//! few sections of the writing-style and engineering-standards skills that a
//! PR needs into four files, and checks each cut with marker strings, so a
//! renamed heading fails loudly instead of pulling in half a skill.

use std::fs;
use std::path::{Path, PathBuf};

/// Where a cut ends.
enum End<'a> {
    /// The line before the first later line starting with this text.
    Before(&'a str),
}

/// Lines from the first one starting with `start` up to, and not including,
/// the next line starting with `end`. A later line starting with `start`
/// opens another range, as `sed -n '/a/,/b/p'` does. A range that never finds
/// its end runs to the last line, which is dropped, as `sed '$d'` drops it.
fn cut(text: &str, start: Option<&str>, end: End<'_>) -> Vec<String> {
    let End::Before(end) = end;
    let lines: Vec<&str> = text.lines().collect();
    let mut out: Vec<&str> = Vec::new();
    let mut inside = start.is_none();
    let mut first = true;
    for line in &lines {
        if inside {
            if !(first && start.is_none()) && line.starts_with(end) {
                // The end line is part of sed's range but `sed '$d'` removes it.
                inside = false;
                continue;
            }
            out.push(line);
            first = false;
        } else if start.is_some_and(|s| line.starts_with(s)) {
            inside = true;
            out.push(line);
            first = false;
        }
    }
    if inside {
        // Ran to the end of the file: `sed '$d'` removes the last line.
        out.pop();
    }
    out.into_iter().map(str::to_string).collect()
}

fn write(path: &Path, lines: &[String]) -> Result<(), String> {
    let mut text = lines.join("\n");
    if !text.is_empty() {
        text.push('\n');
    }
    fs::write(path, text).map_err(|e| format!("could not write {}: {e}", path.display()))
}

/// The four extracted files, for the caller to read.
pub struct Extracted {
    pub core: PathBuf,
    pub prs: PathBuf,
    pub github: PathBuf,
    pub eng: PathBuf,
}

/// The slice a commit message needs: the dash and plain English rule, the
/// commit-message bullet, and the banned words. About a tenth of the skill.
pub fn extract_commit(plugin_root: &Path, out_dir: &Path) -> Result<PathBuf, String> {
    let ws = plugin_root.join("skills/writing-style/SKILL.md");
    let text = fs::read_to_string(&ws).map_err(|_| {
        format!(
            "{} not found under $CLAUDE_PLUGIN_ROOT/skills/. Read the full skill via the Skill tool instead.",
            ws.display()
        )
    })?;
    fs::create_dir_all(out_dir)
        .map_err(|e| format!("could not create {}: {e}", out_dir.display()))?;
    let mut lines = cut(
        &text,
        Some("> **IRON RULE:** MUST NEVER use em dashes"),
        End::Before("> **IRON RULE:**"),
    );
    lines.extend(cut(
        &text,
        Some("- Commit messages MUST"),
        End::Before("- PR descriptions"),
    ));
    lines.push(String::new());
    lines.extend(cut(&text, Some("## Banned Words"), End::Before("---")));
    let has = |s: &str| lines.iter().any(|l| l.contains(s));
    if !(has("IRON RULE") && has("Commit messages MUST") && has("delve")) {
        return Err("writing-style commit extraction is missing an expected rule; read the full skill via the Skill tool instead.".into());
    }
    if has("PR descriptions MUST") || has("# GitHub-Specific Rules") {
        return Err("writing-style commit extraction ran past its end marker; read the full skill via the Skill tool instead.".into());
    }
    let path = out_dir.join("writing-style-commit.md");
    write(&path, &lines)?;
    Ok(path)
}

/// Cut and verify. `out_dir` is created if missing.
pub fn extract(plugin_root: &Path, out_dir: &Path) -> Result<Extracted, String> {
    let ws = plugin_root.join("skills/writing-style/SKILL.md");
    let es = plugin_root.join("skills/engineering-standards/SKILL.md");
    let read = |p: &Path| {
        fs::read_to_string(p).map_err(|_| {
            format!(
                "{} not found under $CLAUDE_PLUGIN_ROOT/skills/. Read the full skill via the Skill tool instead.",
                p.display()
            )
        })
    };
    let ws_text = read(&ws)?;
    let es_text = read(&es)?;
    fs::create_dir_all(out_dir)
        .map_err(|e| format!("could not create {}: {e}", out_dir.display()))?;
    let ex = Extracted {
        core: out_dir.join("writing-style-core.md"),
        prs: out_dir.join("writing-style-prs.md"),
        github: out_dir.join("writing-style-github.md"),
        eng: out_dir.join("eng-standards.md"),
    };
    let core = cut(&ws_text, None, End::Before("# GitHub-Specific Rules"));
    let prs = cut(&ws_text, Some("### When creating PRs"), End::Before("## "));
    let github = cut(
        &ws_text,
        Some("## Prohibited GitHub Content"),
        End::Before("## Examples"),
    );
    let eng = cut(
        &es_text,
        Some("### Readiness"),
        End::Before("### Review Comments"),
    );
    for (path, lines) in [
        (&ex.core, &core),
        (&ex.prs, &prs),
        (&ex.github, &github),
        (&ex.eng, &eng),
    ] {
        write(path, lines)?;
        if lines.is_empty() {
            return Err(format!(
                "{} extracted empty; the source skill's heading text likely changed. Read the full skill via the Skill tool instead before continuing.",
                path.display()
            ));
        }
    }
    let has = |lines: &[String], s: &str| lines.iter().any(|l| l.contains(s));
    if !(has(&core, "IRON RULE") && has(&core, "Banned Words")) {
        return Err("writing-style core extraction is missing an expected rule; read the full skill via the Skill tool instead.".into());
    }
    if !(has(&eng, "Readiness") && has(&eng, "Size")) {
        return Err("engineering-standards extraction is missing Readiness or Size; read the full skill via the Skill tool instead.".into());
    }
    if has(&eng, "Automated Testing") {
        return Err("engineering-standards extraction ran past Review Comments into Automated Testing; the end-marker heading likely changed. Read the full skill via the Skill tool instead.".into());
    }
    Ok(ex)
}

/// The text the command prints on success.
pub fn report(ex: &Extracted) -> String {
    format!(
        "Skill sections extracted and verified:\n  {}\n  {}\n  {}\n  {}",
        ex.core.display(),
        ex.prs.display(),
        ex.github.display(),
        ex.eng.display()
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn repo_root() -> PathBuf {
        PathBuf::from(env!("CARGO_MANIFEST_DIR"))
    }

    fn headings(lines: &[String]) -> Vec<&str> {
        lines
            .iter()
            .filter(|l| l.starts_with('#'))
            .map(String::as_str)
            .collect()
    }

    /// Pins the shape of the four `pr rules` slices: their headings, first
    /// and last line, and a size ceiling in bytes. A heading rename or a
    /// stray section that lands inside a slice fails here, not silently.
    #[test]
    fn the_four_slices_keep_their_shape() {
        let ws = fs::read_to_string(repo_root().join("skills/writing-style/SKILL.md")).unwrap();
        let es =
            fs::read_to_string(repo_root().join("skills/engineering-standards/SKILL.md")).unwrap();
        let core = cut(&ws, None, End::Before("# GitHub-Specific Rules"));
        let prs = cut(&ws, Some("### When creating PRs"), End::Before("## "));
        let github = cut(
            &ws,
            Some("## Prohibited GitHub Content"),
            End::Before("## Examples"),
        );
        let eng = cut(
            &es,
            Some("### Readiness"),
            End::Before("### Review Comments"),
        );
        let bytes = |l: &[String]| l.iter().map(|s| s.len() + 1).sum::<usize>();

        assert_eq!(
            headings(&core),
            [
                "# Writing Style",
                "## Reviewer usability (MUST, adapted from \"Don't Make Me Think\")",
                "## Voice",
                "## Prohibitions",
                "## Banned Words",
            ]
        );
        assert!(core.iter().any(|l| l.contains("IRON RULE")));
        assert!(bytes(&core) < 14_000, "core is {} bytes", bytes(&core));

        assert_eq!(headings(&prs), ["### When creating PRs"]);
        assert!(bytes(&prs) < 800);

        assert_eq!(headings(&github), ["## Prohibited GitHub Content"]);
        assert!(github.iter().any(|l| l.starts_with("11. ")));
        assert!(bytes(&github) < 2_500);

        assert_eq!(headings(&eng), ["### Readiness", "### Size"]);
        assert!(bytes(&eng) < 1_500);
    }

    #[test]
    fn the_commit_slice_is_small_and_has_the_rules_a_commit_needs() {
        let dir =
            std::env::temp_dir().join(format!("pb-rules-commit-{}", crate::testing::run_id()));
        let path = extract_commit(&repo_root(), &dir).unwrap();
        let text = fs::read_to_string(&path).unwrap();
        assert!(text.contains("MUST NEVER use em dashes"));
        assert!(text.contains("- Commit messages MUST"));
        assert!(text.contains("## Banned Words"));
        assert!(!text.contains("PR descriptions MUST"));
        assert!(!text.contains("GitHub-Specific"));
        assert!(text.len() < 3_500, "commit slice is {} bytes", text.len());
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn the_examples_heading_stub_stays_so_the_github_slice_has_an_end() {
        let ws = fs::read_to_string(repo_root().join("skills/writing-style/SKILL.md")).unwrap();
        assert!(ws.lines().any(|l| l.starts_with("## Examples")));
    }

    #[test]
    fn a_range_stops_before_its_end_heading() {
        let t = "intro\n### When creating PRs\nbody\nmore\n## Next\nafter\n";
        assert_eq!(
            cut(t, Some("### When creating PRs"), End::Before("## ")),
            vec!["### When creating PRs", "body", "more"]
        );
    }

    #[test]
    fn a_three_hash_heading_does_not_end_a_two_hash_range() {
        let t = "### When creating PRs\nbody\n### Sub\nkeep\n## End\n";
        assert_eq!(
            cut(t, Some("### When creating PRs"), End::Before("## ")),
            vec!["### When creating PRs", "body", "### Sub", "keep"]
        );
    }

    #[test]
    fn the_core_cut_runs_from_the_first_line_to_the_marker() {
        let t = "# Title\nIRON RULE\n# GitHub-Specific Rules\nrest\n";
        assert_eq!(
            cut(t, None, End::Before("# GitHub-Specific Rules")),
            vec!["# Title", "IRON RULE"]
        );
    }

    #[test]
    fn a_missing_end_marker_runs_to_the_end_minus_the_last_line() {
        let t = "### Readiness\na\nb\nc\n";
        assert_eq!(
            cut(t, Some("### Readiness"), End::Before("### Review Comments")),
            vec!["### Readiness", "a", "b"]
        );
    }

    #[test]
    fn a_missing_start_marker_cuts_nothing() {
        assert!(cut("a\nb\n", Some("### X"), End::Before("## ")).is_empty());
    }
}
