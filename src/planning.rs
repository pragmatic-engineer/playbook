// SPDX-FileCopyrightText: 2026 Igor Santos
// SPDX-License-Identifier: Apache-2.0

//! Helpers for `/playbook:plan` and the review commands: the plan checkpoint,
//! the glossary append, and the grounding-review reference lookup.

use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::time::Duration;

use crate::common::atomic::{atomic_append, with_dir_lock, write_atomic};

/// A slug is one kebab-case file stem: no separators and no leading dot.
pub fn valid_slug(slug: &str) -> bool {
    !slug.is_empty()
        && !slug.starts_with('.')
        && slug
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_' || c == '.')
}

/// `<plans_dir>/<slug>.checkpoint.md`.
pub fn checkpoint_path(plans_dir: &Path, slug: &str) -> Result<PathBuf, String> {
    if !valid_slug(slug) {
        return Err(format!(
            "invalid topic slug '{slug}': use letters, digits, '-' and '_' only"
        ));
    }
    Ok(plans_dir.join(format!("{slug}.checkpoint.md")))
}

/// Replaces the checkpoint under a directory lock. The lock is advisory: when
/// it cannot be taken the write still happens.
pub fn write_checkpoint(path: &Path, content: &str) -> io::Result<()> {
    let lock = PathBuf::from(format!("{}.lock", path.display()));
    let (acquired, result) = with_dir_lock(&lock, 20, Duration::from_millis(50), || {
        write_atomic(path, content)
    });
    if acquired {
        let _ = fs::remove_dir(&lock);
    }
    result
}

/// Appends one entry to `<root>/GLOSSARY.md` under a directory lock.
pub fn glossary_add(root: &Path, entry: &str) -> PathBuf {
    let file = root.join("GLOSSARY.md");
    atomic_append(&file.display().to_string(), entry);
    file
}

/// The reference file for a skill, or the skill's `SKILL.md` when the file is
/// missing or no name was given.
pub fn skill_ref(plugin_root: &Path, skill: &str, name: Option<&str>) -> PathBuf {
    let dir = plugin_root.join("skills").join(skill);
    if let Some(name) = name {
        let name = name.strip_suffix(".md").unwrap_or(name);
        if valid_slug(name) {
            let file = dir.join("references").join(format!("{name}.md"));
            if file.is_file() {
                return file;
            }
        }
    }
    dir.join("SKILL.md")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn scratch(tag: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("pb-planning-{tag}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn slugs_with_separators_or_a_leading_dot_are_rejected() {
        for bad in ["", "../x", "a/b", ".hidden", "a b"] {
            assert!(checkpoint_path(Path::new("/p"), bad).is_err(), "{bad}");
        }
        assert_eq!(
            checkpoint_path(Path::new("/p"), "proj-123").unwrap(),
            PathBuf::from("/p/proj-123.checkpoint.md")
        );
    }

    #[test]
    fn a_checkpoint_is_replaced_whole_and_leaves_no_lock_behind() {
        let dir = scratch("cp");
        let path = dir.join("t.checkpoint.md");
        write_checkpoint(&path, "one").unwrap();
        write_checkpoint(&path, "two").unwrap();
        assert_eq!(fs::read_to_string(&path).unwrap(), "two");
        assert!(!dir.join("t.checkpoint.md.lock").exists());
    }

    #[test]
    fn glossary_entries_accumulate_in_order() {
        let dir = scratch("gl");
        glossary_add(&dir, "- **Term**: a thing");
        let file = glossary_add(&dir, "- **Other**: another");
        assert_eq!(
            fs::read_to_string(file).unwrap(),
            "- **Term**: a thing\n- **Other**: another\n"
        );
    }

    #[test]
    fn a_missing_reference_falls_back_to_the_skill_file() {
        let root = scratch("ref");
        let refs = root.join("skills/grounding-review/references");
        fs::create_dir_all(&refs).unwrap();
        fs::write(refs.join("security.md"), "").unwrap();
        let skill = root.join("skills/grounding-review/SKILL.md");
        assert_eq!(
            skill_ref(&root, "grounding-review", Some("security")),
            refs.join("security.md")
        );
        assert_eq!(skill_ref(&root, "grounding-review", Some("nope")), skill);
        assert_eq!(skill_ref(&root, "grounding-review", None), skill);
        assert_eq!(skill_ref(&root, "grounding-review", Some("../x")), skill);
    }
}
