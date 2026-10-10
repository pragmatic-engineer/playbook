// SPDX-FileCopyrightText: 2026 Igor Santos
// SPDX-License-Identifier: Apache-2.0

//! Where a plan's gate sources live, and `playbook gate snapshot`. The plan
//! command keeps one shared draft file and freezes a copy of it for each gate
//! phase at the moment that phase is dispatched, so a phase records its
//! verdict against what it actually reviewed. The copy used to be the model
//! writing the whole draft out again for every phase; it is one file copy.

use std::path::{Path, PathBuf};

fn valid(label: &str, value: &str) -> Result<(), String> {
    let ok = !value.is_empty()
        && !value.starts_with('.')
        && value
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_' || c == '.');
    if ok {
        Ok(())
    } else {
        Err(format!(
            "invalid {label} '{value}': use letters, digits, '-', '_' and '.' only"
        ))
    }
}

/// `<plans_dir>/<slug>.gate-source.md`: the draft as it stands now, the file
/// the final `gate check` reads.
pub fn shared_path(plans_dir: &Path, slug: &str) -> Result<PathBuf, String> {
    valid("plan slug", slug)?;
    Ok(plans_dir.join(format!("{slug}.gate-source.md")))
}

/// `<plans_dir>/<slug>.gate-source.<phase>.md`: the draft as it was when
/// `phase` was dispatched, the file that phase records its verdict against.
pub fn phase_path(plans_dir: &Path, slug: &str, phase: &str) -> Result<PathBuf, String> {
    valid("plan slug", slug)?;
    valid("phase", phase)?;
    Ok(plans_dir.join(format!("{slug}.gate-source.{phase}.md")))
}

/// Copy the shared draft over each phase's snapshot and return the snapshot
/// paths. Nothing is copied when the shared file is missing or a name is bad.
pub fn snapshot(plans_dir: &Path, slug: &str, phases: &[String]) -> Result<Vec<PathBuf>, String> {
    let shared = shared_path(plans_dir, slug)?;
    let targets: Vec<PathBuf> = phases
        .iter()
        .map(|p| phase_path(plans_dir, slug, p))
        .collect::<Result<_, _>>()?;
    if !shared.is_file() {
        return Err(format!(
            "{} does not exist; write the current draft there first",
            shared.display()
        ));
    }
    for target in &targets {
        std::fs::copy(&shared, target)
            .map_err(|e| format!("could not write {}: {e}", target.display()))?;
    }
    Ok(targets)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn dir(tag: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("pb-gate-snap-{}-{tag}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    #[test]
    fn the_paths_follow_the_names_the_plan_command_uses() {
        let d = Path::new("/p");
        assert_eq!(
            shared_path(d, "my-plan").unwrap(),
            Path::new("/p/my-plan.gate-source.md")
        );
        assert_eq!(
            phase_path(d, "my-plan", "fact-check").unwrap(),
            Path::new("/p/my-plan.gate-source.fact-check.md")
        );
    }

    #[test]
    fn a_name_with_a_separator_or_a_leading_dot_is_refused() {
        let d = Path::new("/p");
        for bad in ["", "../x", "a/b", ".hidden", "a b"] {
            assert!(shared_path(d, bad).is_err(), "{bad:?}");
            assert!(phase_path(d, "ok", bad).is_err(), "{bad:?}");
        }
    }

    #[test]
    fn snapshot_copies_the_shared_draft_to_every_phase() {
        let d = dir("copy");
        std::fs::write(shared_path(&d, "p").unwrap(), "draft v1").unwrap();
        let out = snapshot(&d, "p", &["adversarial".into(), "test-review".into()]).unwrap();
        assert_eq!(out.len(), 2);
        for path in &out {
            assert_eq!(std::fs::read_to_string(path).unwrap(), "draft v1");
        }
        // A later draft overwrites the old snapshot; an untouched phase keeps its own.
        std::fs::write(shared_path(&d, "p").unwrap(), "draft v2").unwrap();
        snapshot(&d, "p", &["adversarial".into()]).unwrap();
        assert_eq!(std::fs::read_to_string(&out[0]).unwrap(), "draft v2");
        assert_eq!(std::fs::read_to_string(&out[1]).unwrap(), "draft v1");
    }

    #[test]
    fn a_missing_shared_draft_or_a_bad_phase_copies_nothing() {
        let d = dir("missing");
        let err = snapshot(&d, "p", &["fact-check".into()]).unwrap_err();
        assert!(err.contains("write the current draft"), "{err}");
        std::fs::write(shared_path(&d, "p").unwrap(), "x").unwrap();
        assert!(snapshot(&d, "p", &["fact-check".into(), "../x".into()]).is_err());
        assert!(!phase_path(&d, "p", "fact-check").unwrap().exists());
    }
}
