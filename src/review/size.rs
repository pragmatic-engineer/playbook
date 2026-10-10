// SPDX-FileCopyrightText: 2026 Igor Santos
// SPDX-License-Identifier: Apache-2.0

//! `playbook review size`: how big a diff is and which review path it takes.
//! `/playbook:implement` Step 9 reviews a small diff with one reviewer that
//! holds all five lenses, and a larger one with the triage plus lens swarm.
//! The cut lives here so it is one number, tested, not a sentence in a
//! command file.

use std::path::Path;
use std::process::Command;

/// Changed lines (added plus deleted) up to which one reviewer covers all
/// lenses. Past it the diff is big enough that separate lens passes find
/// things a single pass skims over.
pub const SMALL_DIFF_LINES: u64 = 150;

/// Which review path a diff takes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ReviewPath {
    Single,
    Swarm,
}

impl ReviewPath {
    pub fn name(self) -> &'static str {
        match self {
            ReviewPath::Single => "single",
            ReviewPath::Swarm => "swarm",
        }
    }
}

/// Added plus deleted lines and the number of files, from `git diff --numstat`
/// output. A binary file (`-` counts) adds to the file count only.
pub fn parse_numstat(text: &str) -> (u64, u64) {
    let (mut lines, mut files) = (0u64, 0u64);
    for row in text.lines().filter(|l| !l.trim().is_empty()) {
        let mut cols = row.split('\t');
        let (add, del) = (cols.next().unwrap_or("-"), cols.next().unwrap_or("-"));
        files += 1;
        lines += add.parse::<u64>().unwrap_or(0) + del.parse::<u64>().unwrap_or(0);
    }
    (lines, files)
}

/// One reviewer for a diff of at most `SMALL_DIFF_LINES` lines, unless every
/// lens was asked for explicitly.
pub fn choose(lines: u64, all_lenses: bool) -> ReviewPath {
    if all_lenses || lines > SMALL_DIFF_LINES {
        ReviewPath::Swarm
    } else {
        ReviewPath::Single
    }
}

/// `lines=N files=M path=single|swarm` for `base...head` in `dir`.
pub fn run(dir: &Path, base: &str, head: &str, all_lenses: bool) -> Result<String, String> {
    let out = Command::new("git")
        .arg("-C")
        .arg(dir)
        .args(["diff", "--numstat", &format!("{base}...{head}")])
        .output()
        .map_err(|e| format!("git: {e}"))?;
    if !out.status.success() {
        return Err(format!(
            "git diff {base}...{head} failed: {}",
            String::from_utf8_lossy(&out.stderr).trim()
        ));
    }
    let (lines, files) = parse_numstat(&String::from_utf8_lossy(&out.stdout));
    Ok(format!(
        "lines={lines} files={files} path={}",
        choose(lines, all_lenses).name()
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn numstat_sums_added_and_deleted_lines_and_counts_files() {
        let text = "10\t2\tsrc/a.rs\n0\t5\tsrc/b.rs\n-\t-\timg.png\n\n";
        assert_eq!(parse_numstat(text), (17, 3));
        assert_eq!(parse_numstat(""), (0, 0));
    }

    #[test]
    fn the_cut_is_inclusive_and_all_lenses_always_swarms() {
        assert_eq!(choose(0, false), ReviewPath::Single);
        assert_eq!(choose(SMALL_DIFF_LINES, false), ReviewPath::Single);
        assert_eq!(choose(SMALL_DIFF_LINES + 1, false), ReviewPath::Swarm);
        assert_eq!(choose(3, true), ReviewPath::Swarm);
    }
}
