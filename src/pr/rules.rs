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
