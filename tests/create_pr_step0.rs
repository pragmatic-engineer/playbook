// SPDX-FileCopyrightText: 2026 Igor Santos
// SPDX-License-Identifier: Apache-2.0

//! Step 0 of `commands/create-pull-request.md` runs `playbook pr rules`, which
//! cuts the writing-style and engineering-standards sections a PR needs. These
//! tests run the real command against this repo's skills.

use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

fn root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
}

fn run_step0() -> (bool, String) {
    run_step0_in(&root())
}

fn run_step0_in(plugin_root: &Path) -> (bool, String) {
    let out = Command::new(env!("CARGO_BIN_EXE_playbook"))
        .args(["pr", "rules", "--plugin-root"])
        .arg(plugin_root)
        .output()
        .unwrap();
    let text = format!(
        "{}{}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );
    (out.status.success(), text)
}

fn path_ending(out: &str, suffix: &str) -> PathBuf {
    let line = out
        .lines()
        .map(str::trim)
        .find(|l| l.ends_with(suffix))
        .unwrap_or_else(|| panic!("no line ending {suffix} in: {out}"));
    PathBuf::from(line)
}

#[test]
fn step0_extracts_cleanly_against_this_repos_skills() {
    let (ok, out) = run_step0();
    assert!(ok, "Step 0 failed: {out}");
    assert!(out.contains("extracted and verified"), "{out}");
    assert!(
        !out.to_lowercase().contains("error"),
        "a guard fired: {out}"
    );
}

#[test]
fn extracted_content_carries_the_rules_step0_claims_to_need() {
    let (_, out) = run_step0();
    let core = path_ending(&out, "writing-style-core.md");
    let prs = path_ending(&out, "writing-style-prs.md");
    let github = path_ending(&out, "writing-style-github.md");
    let eng = path_ending(&out, "eng-standards.md");
    let read = |p: &Path| fs::read_to_string(p).unwrap();

    assert!(read(&core).contains("IRON RULE"));
    assert!(read(&core).contains("Banned Words"));
    assert!(read(&prs).contains("When creating PRs"));
    assert!(read(&github).contains("Prohibited GitHub Content"));
    assert!(read(&github).contains("commit hashes"));
    let eng_text = read(&eng);
    assert!(eng_text.contains("Readiness"));
    assert!(eng_text.contains("Size"));
    assert!(
        !eng_text.to_lowercase().contains("automated testing"),
        "ran past its intended range into Automated Testing"
    );
    assert!(
        !eng_text.to_lowercase().contains("review comments"),
        "end-marker heading leaked into the extract"
    );
    let _ = fs::remove_dir_all(core.parent().unwrap());
}

#[test]
fn a_renamed_end_marker_heading_produces_a_detectable_runaway_extraction() {
    let skill = fs::read_to_string(root().join("skills/engineering-standards/SKILL.md")).unwrap();
    let renamed = skill.replace("### Review Comments\n", "### Comment Guidelines\n");
    let start = renamed.find("### Readiness").expect("Readiness heading");
    let extract = &renamed[start..];
    assert!(
        extract.contains("Automated Testing"),
        "expected the renamed-heading case to run away into Automated Testing"
    );
}

#[test]
fn a_renamed_end_marker_makes_the_command_fail_loudly() {
    let dir = std::env::temp_dir().join(format!("pb-rules-{}", std::process::id()));
    let _ = fs::remove_dir_all(&dir);
    for skill in ["writing-style", "engineering-standards"] {
        fs::create_dir_all(dir.join("skills").join(skill)).unwrap();
        let text = fs::read_to_string(root().join(format!("skills/{skill}/SKILL.md"))).unwrap();
        let text = text.replace("### Review Comments\n", "### Comment Guidelines\n");
        fs::write(dir.join(format!("skills/{skill}/SKILL.md")), text).unwrap();
    }
    let (ok, out) = run_step0_in(&dir);
    assert!(!ok, "{out}");
    assert!(
        out.contains("ran past Review Comments into Automated Testing"),
        "{out}"
    );
    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn a_missing_skill_names_the_fallback() {
    let (ok, out) = run_step0_in(Path::new("/nonexistent-plugin-root"));
    assert!(!ok);
    assert!(
        out.contains("Read the full skill via the Skill tool instead"),
        "{out}"
    );
}

#[test]
fn the_commit_flag_writes_one_small_slice() {
    let o = Command::new(env!("CARGO_BIN_EXE_playbook"))
        .args(["pr", "rules", "--commit", "--plugin-root"])
        .arg(root())
        .output()
        .unwrap();
    assert!(o.status.success(), "{}", String::from_utf8_lossy(&o.stderr));
    let path = PathBuf::from(String::from_utf8_lossy(&o.stdout).trim());
    assert!(path.ends_with("writing-style-commit.md"), "{path:?}");
    let text = fs::read_to_string(&path).unwrap();
    assert!(text.contains("IRON RULE") && text.contains("Banned Words"));
    assert!(!text.contains("When reviewing"));
    let full = fs::read_to_string(root().join("skills/writing-style/SKILL.md")).unwrap();
    assert!(
        text.len() * 5 < full.len(),
        "{} of {}",
        text.len(),
        full.len()
    );
    let _ = fs::remove_dir_all(path.parent().unwrap());
}
