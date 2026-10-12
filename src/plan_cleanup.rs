// SPDX-FileCopyrightText: 2026 Igor Santos
// SPDX-License-Identifier: Apache-2.0

//! `playbook plan cleanup`: removes the scratch files `/playbook:plan` leaves in
//! the plans folder, which sits outside the repo and so cannot be removed with
//! `rm` under the `rm_workspace_guard` hook.
//!
//! Only three name shapes are ever touched: `<slug>.checkpoint.md`,
//! `<slug>.checkpoint.md.lock` and `<slug>.gate-source*.md`. The final plan
//! `<slug>.md` and the quality report `<slug>-quality.md` never match.

use std::fs;
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime};

/// Same rule `plan checkpoint` uses to create the files, so every slug that can
/// leave scratch files can also clear them.
pub use crate::planning::valid_slug;

/// The slug a scratch file name belongs to, or `None` when the name is not one
/// of the three scratch shapes.
pub fn scratch_slug(name: &str) -> Option<&str> {
    if let Some(slug) = name.strip_suffix(".checkpoint.md.lock") {
        return Some(slug);
    }
    if let Some(slug) = name.strip_suffix(".checkpoint.md") {
        return Some(slug);
    }
    if name.ends_with(".md") {
        if let Some(at) = name.find(".gate-source") {
            return Some(&name[..at]);
        }
    }
    None
}

/// Scratch entries in `dir` that belong to `slug`, sorted by name.
fn scratch_for(dir: &Path, slug: &str) -> Vec<PathBuf> {
    let Ok(rd) = fs::read_dir(dir) else {
        return Vec::new();
    };
    let mut v: Vec<PathBuf> = rd
        .flatten()
        .filter(|e| {
            let name = e.file_name();
            name.to_str().and_then(scratch_slug) == Some(slug)
        })
        .map(|e| e.path())
        .collect();
    v.sort();
    v
}

/// Refuses a symlink whose target is outside `dir`.
fn check_contained(dir: &Path, path: &Path) -> Result<(), String> {
    let is_link = fs::symlink_metadata(path)
        .map(|m| m.file_type().is_symlink())
        .unwrap_or(false);
    if !is_link {
        return Ok(());
    }
    let base = fs::canonicalize(dir).map_err(|e| format!("{}: {e}", dir.display()))?;
    match fs::canonicalize(path) {
        Ok(target) if target.starts_with(&base) => Ok(()),
        _ => Err(format!(
            "refusing {}: a symlink that leaves the plans folder",
            path.display()
        )),
    }
}

fn remove_one(path: &Path) -> std::io::Result<()> {
    let meta = fs::symlink_metadata(path)?;
    if meta.file_type().is_symlink() || meta.is_file() {
        fs::remove_file(path)
    } else if meta.is_dir() && path.to_string_lossy().ends_with(".lock") {
        fs::remove_dir_all(path)
    } else {
        Err(std::io::Error::other("not a scratch file or lock"))
    }
}

/// Removes the scratch files of one slug. Returns the names handled (removed,
/// or only listed when `dry_run`). Nothing is removed when any entry is refused.
pub fn cleanup_slug(dir: &Path, slug: &str, dry_run: bool) -> Result<Vec<String>, String> {
    if !valid_slug(slug) {
        return Err(format!(
            "invalid slug '{slug}': use letters, digits, '-', '_' and '.', not starting with '.'"
        ));
    }
    let found = scratch_for(dir, slug);
    for path in &found {
        check_contained(dir, path)?;
    }
    apply(&found, dry_run)
}

fn apply(found: &[PathBuf], dry_run: bool) -> Result<Vec<String>, String> {
    let mut names = Vec::new();
    for path in found {
        if !dry_run {
            remove_one(path).map_err(|e| format!("could not remove {}: {e}", path.display()))?;
        }
        if let Some(n) = path.file_name().and_then(|n| n.to_str()) {
            names.push(n.to_string());
        }
    }
    Ok(names)
}

/// Scratch files for every slug that already has a final `<slug>.md`, at least
/// `older_than_days` old by modification time. Deletes only when `yes`.
pub fn cleanup_stale(
    dir: &Path,
    older_than_days: u64,
    yes: bool,
    now: SystemTime,
) -> Result<Vec<String>, String> {
    let Ok(rd) = fs::read_dir(dir) else {
        return Ok(Vec::new());
    };
    let min_age = Duration::from_secs(older_than_days.saturating_mul(86_400));
    let mut found: Vec<PathBuf> = Vec::new();
    for entry in rd.flatten() {
        let name = entry.file_name();
        let Some(slug) = name.to_str().and_then(scratch_slug) else {
            continue;
        };
        if !valid_slug(slug) || !dir.join(format!("{slug}.md")).is_file() {
            continue;
        }
        let old_enough = fs::symlink_metadata(entry.path())
            .and_then(|m| m.modified())
            .map(|t| now.duration_since(t).unwrap_or_default() >= min_age)
            .unwrap_or(false);
        if old_enough {
            check_contained(dir, &entry.path())?;
            found.push(entry.path());
        }
    }
    found.sort();
    apply(&found, !yes)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tmp(tag: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("pb-cleanup-{tag}-{}", crate::testing::run_id()));
        let _ = fs::remove_dir_all(&d);
        fs::create_dir_all(&d).expect("mkdir");
        d
    }

    fn seed(d: &Path) {
        for n in [
            "demo.md",
            "demo-quality.md",
            "demo.checkpoint.md",
            "demo.gate-source.md",
            "demo.gate-source.fact-check.md",
            "other.checkpoint.md",
            "demo-two.checkpoint.md",
        ] {
            fs::write(d.join(n), "x").expect("write");
        }
        fs::create_dir(d.join("demo.checkpoint.md.lock")).expect("lock");
    }

    #[test]
    fn removes_only_the_scratch_patterns() {
        let d = tmp("only");
        seed(&d);
        let got = cleanup_slug(&d, "demo", false).expect("ok");
        assert_eq!(
            got,
            vec![
                "demo.checkpoint.md",
                "demo.checkpoint.md.lock",
                "demo.gate-source.fact-check.md",
                "demo.gate-source.md"
            ]
        );
        assert!(d.join("demo.md").is_file() && d.join("demo-quality.md").is_file());
        assert!(d.join("other.checkpoint.md").is_file());
        assert!(d.join("demo-two.checkpoint.md").is_file());
        assert!(!d.join("demo.checkpoint.md.lock").exists());
    }

    #[test]
    fn slugs_the_checkpoint_command_accepts_are_accepted() {
        let d = tmp("wide");
        fs::write(d.join("My_Plan.v2.checkpoint.md"), "x").expect("w");
        let got = cleanup_slug(&d, "My_Plan.v2", false).expect("ok");
        assert_eq!(got, vec!["My_Plan.v2.checkpoint.md"]);
    }

    #[test]
    fn dry_run_changes_nothing_and_second_run_is_idempotent() {
        let d = tmp("dry");
        seed(&d);
        assert_eq!(cleanup_slug(&d, "demo", true).expect("ok").len(), 4);
        assert!(d.join("demo.checkpoint.md").is_file());
        cleanup_slug(&d, "demo", false).expect("ok");
        assert!(cleanup_slug(&d, "demo", false).expect("ok").is_empty());
    }

    #[test]
    fn bad_slugs_are_rejected() {
        let d = tmp("bad");
        for s in ["", "../x", "a/b", ".a", "..", "a b"] {
            assert!(cleanup_slug(&d, s, false).is_err(), "{s}");
        }
    }

    #[cfg(unix)]
    #[test]
    fn a_symlink_leaving_the_folder_is_refused_and_nothing_is_removed() {
        let d = tmp("link");
        let outside = tmp("link-out");
        fs::write(outside.join("secret"), "s").expect("w");
        fs::write(d.join("demo.checkpoint.md"), "x").expect("w");
        std::os::unix::fs::symlink(outside.join("secret"), d.join("demo.gate-source.md"))
            .expect("link");
        assert!(cleanup_slug(&d, "demo", false).is_err());
        assert!(d.join("demo.checkpoint.md").is_file());
        assert!(outside.join("secret").is_file());
    }

    #[test]
    fn stale_mode_needs_yes_and_a_final_plan() {
        let d = tmp("stale");
        seed(&d);
        let now = SystemTime::now();
        let listed = cleanup_stale(&d, 0, false, now).expect("ok");
        assert_eq!(listed.len(), 4, "{listed:?}");
        assert!(d.join("demo.checkpoint.md").is_file());
        cleanup_stale(&d, 0, true, now).expect("ok");
        assert!(!d.join("demo.checkpoint.md").exists());
        assert!(d.join("other.checkpoint.md").is_file());
        assert!(d.join("demo.md").is_file());
    }

    #[test]
    fn stale_mode_honours_older_than() {
        let d = tmp("age");
        seed(&d);
        assert!(cleanup_stale(&d, 5, true, SystemTime::now())
            .expect("ok")
            .is_empty());
        let later = SystemTime::now() + Duration::from_secs(6 * 86_400);
        assert_eq!(cleanup_stale(&d, 5, true, later).expect("ok").len(), 4);
    }
}
