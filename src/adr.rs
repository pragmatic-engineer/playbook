// SPDX-FileCopyrightText: 2026 Igor Santos
// SPDX-License-Identifier: Apache-2.0

//! `playbook adr next`: the number and date for a new ADR, which
//! `commands/adr.md` used to work out with a bash block.

use std::fs;
use std::path::Path;

/// The next four-digit number: one above the highest `NNNN` prefix among the
/// names in `dir`, or `0001` when there are none.
fn next_number(dir: &Path) -> String {
    let highest = fs::read_dir(dir)
        .into_iter()
        .flatten()
        .flatten()
        .filter_map(|e| {
            let name = e.file_name().to_string_lossy().into_owned();
            let digits: String = name.chars().take(4).collect();
            (digits.len() == 4 && digits.chars().all(|c| c.is_ascii_digit()))
                .then(|| digits.parse::<u32>().unwrap_or(0))
        })
        .max()
        .unwrap_or(0);
    format!("{:04}", highest + 1)
}

/// Create `<root>/docs/adr` if needed and print the line the command reads.
pub fn next_line(root: &Path, today: &str) -> Result<String, String> {
    let dir = root.join("docs/adr");
    fs::create_dir_all(&dir).map_err(|e| format!("could not create {}: {e}", dir.display()))?;
    Ok(format!(
        "Next number: {}   Date: {today}",
        next_number(&dir)
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::common::test_support::scratch_dir;

    fn dir(tag: &str) -> std::path::PathBuf {
        let d = scratch_dir(tag);
        fs::create_dir_all(&d).unwrap();
        d
    }

    #[test]
    fn an_empty_folder_starts_at_one() {
        assert_eq!(next_number(&dir("adr-empty")), "0001");
    }

    #[test]
    fn the_highest_prefix_wins_and_gaps_are_not_filled() {
        let d = dir("adr-gap");
        for n in [
            "0001-a.md",
            "0007-b.md",
            "0003-c.md",
            "README.md",
            "12-x.md",
        ] {
            fs::write(d.join(n), "").unwrap();
        }
        assert_eq!(next_number(&d), "0008");
    }

    #[test]
    fn the_line_creates_the_folder_and_carries_the_date() {
        let root = dir("adr-line");
        let line = next_line(&root, "2026-10-09").unwrap();
        assert_eq!(line, "Next number: 0001   Date: 2026-10-09");
        assert!(root.join("docs/adr").is_dir());
    }
}
