// SPDX-FileCopyrightText: 2026 Igor Santos
// SPDX-License-Identifier: Apache-2.0

//! PATH checks: find every `playbook` on PATH, warn when an earlier one hides
//! the updated binary.

use std::ffi::OsStr;
use std::fs;
use std::path::{Path, PathBuf};

fn is_executable(path: &Path) -> bool {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        path.metadata()
            .map(|m| m.is_file() && m.permissions().mode() & 0o111 != 0)
            .unwrap_or(false)
    }
    #[cfg(not(unix))]
    {
        path.is_file()
    }
}

/// Every distinct executable named `playbook` on `path_var`, in PATH order,
/// like `which -a`. Two entries that resolve to one file count once.
pub fn scan(path_var: &OsStr) -> Vec<PathBuf> {
    let mut found: Vec<PathBuf> = Vec::new();
    let mut real: Vec<PathBuf> = Vec::new();
    for dir in std::env::split_paths(path_var) {
        // A relative entry (`.`, empty) would run a repo-local file.
        if !dir.is_absolute() {
            continue;
        }
        let candidate = dir.join("playbook");
        if !is_executable(&candidate) {
            continue;
        }
        let canonical = fs::canonicalize(&candidate).unwrap_or_else(|_| candidate.clone());
        if !real.contains(&canonical) {
            real.push(canonical);
            found.push(candidate);
        }
    }
    found
}

fn same_file(a: &Path, b: &Path) -> bool {
    match (fs::canonicalize(a), fs::canonicalize(b)) {
        (Ok(x), Ok(y)) => x == y,
        _ => a == b,
    }
}

use super::is_homebrew;

/// A warning when the first `playbook` on PATH is not `updated`, naming both
/// paths and the fix. `None` when the updated binary wins.
pub fn shadow_warning(path_var: &OsStr, updated: &Path) -> Option<String> {
    let first = scan(path_var).into_iter().next()?;
    if same_file(&first, updated) {
        return None;
    }
    let fix = if is_homebrew(&first) {
        "run `brew uninstall playbook`, or put the updated directory first on PATH"
    } else {
        "remove it, or put the updated directory first on PATH"
    };
    Some(format!(
        "{} runs before the updated {} on PATH, so hooks and your shell keep using the old binary; {fix}",
        first.display(),
        updated.display()
    ))
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;
    use crate::common::test_support::scratch_dir;
    use std::ffi::OsString;

    fn bin_in(tag: &str) -> PathBuf {
        let dir = scratch_dir(tag);
        fs::create_dir_all(&dir).unwrap();
        let bin = dir.join("playbook");
        fs::write(&bin, "#!/bin/sh\n").unwrap();
        super::super::swap::make_executable(&bin).unwrap();
        bin
    }

    fn path_of(bins: &[&Path]) -> OsString {
        std::env::join_paths(bins.iter().map(|b| b.parent().unwrap())).unwrap()
    }

    #[test]
    fn an_earlier_playbook_is_reported_as_hiding_the_updated_one() {
        let old = bin_in("shadow-old");
        let new = bin_in("shadow-new");
        let warning = shadow_warning(&path_of(&[&old, &new]), &new).unwrap();
        assert!(warning.contains(&old.display().to_string()), "{warning}");
        assert!(warning.contains(&new.display().to_string()), "{warning}");
    }

    #[test]
    fn no_warning_when_the_updated_binary_comes_first() {
        let old = bin_in("noshadow-old");
        let new = bin_in("noshadow-new");
        assert!(shadow_warning(&path_of(&[&new, &old]), &new).is_none());
    }

    #[test]
    fn a_symlink_to_the_updated_binary_is_not_a_shadow() {
        let new = bin_in("link-new");
        let dir = scratch_dir("link-dir");
        fs::create_dir_all(&dir).unwrap();
        std::os::unix::fs::symlink(&new, dir.join("playbook")).unwrap();
        assert!(shadow_warning(&path_of(&[&dir.join("playbook"), &new]), &new).is_none());
    }

    #[test]
    fn a_homebrew_shadow_gets_the_brew_fix() {
        let cellar = scratch_dir("brew").join("Cellar/playbook/0.14.0/bin");
        fs::create_dir_all(&cellar).unwrap();
        let brew = cellar.join("playbook");
        fs::write(&brew, "#!/bin/sh\n").unwrap();
        super::super::swap::make_executable(&brew).unwrap();
        let new = bin_in("brew-new");
        let warning = shadow_warning(&path_of(&[&brew, &new]), &new).unwrap();
        assert!(warning.contains("brew uninstall playbook"), "{warning}");
    }

    #[test]
    fn relative_path_entries_are_never_scanned() {
        let bin = bin_in("relative");
        let dir = bin.parent().unwrap();
        let relative = std::env::join_paths([Path::new("."), Path::new(""), dir]).unwrap();
        assert_eq!(scan(&relative), vec![bin]);
    }
}
