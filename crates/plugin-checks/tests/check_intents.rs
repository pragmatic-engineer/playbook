// SPDX-FileCopyrightText: 2026 Igor Santos
// SPDX-License-Identifier: Apache-2.0

//! Runs `scripts/check-intents.sh` on throwaway copies of the repo layout:
//! the list checks, then the compare against a stub binary's tool map, then
//! this repository's `plugins/intents.json`.

use std::ffi::OsStr;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::atomic::{AtomicU64, Ordering};

static COUNTER: AtomicU64 = AtomicU64::new(0);

fn repo_root() -> PathBuf {
    PathBuf::from(env!("PLAYBOOK_REPO_ROOT"))
}

/// A scratch tree holding `scripts/check-intents.sh` and `plugins/intents.json`.
/// The script finds the list next to itself. The tree is removed on drop.
struct Tree(PathBuf);

impl Drop for Tree {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

impl Tree {
    fn new(intents: &str) -> Self {
        let n = COUNTER.fetch_add(1, Ordering::Relaxed);
        let dir = std::env::temp_dir().join(format!("check-intents-{}-{n}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(dir.join("scripts")).expect("scripts dir");
        fs::create_dir_all(dir.join("plugins")).expect("plugins dir");
        fs::copy(
            repo_root().join("scripts/check-intents.sh"),
            dir.join("scripts/check-intents.sh"),
        )
        .expect("copy the script");
        fs::write(dir.join("plugins/intents.json"), intents).expect("write the list");
        Self(dir)
    }

    /// A stub `playbook` that prints `body` for `tools map --json` and exits
    /// with `code`.
    fn stub(&self, body: &str, code: i32) -> PathBuf {
        let bin = self.0.join("fake-playbook");
        fs::write(&bin, format!("#!/bin/sh\necho '{body}'\nexit {code}\n")).expect("stub");
        let status = Command::new("chmod")
            .arg("+x")
            .arg(&bin)
            .status()
            .expect("chmod");
        assert!(status.success());
        bin
    }
}

/// Runs the script under `root` with `args`, with `INTENTS_STRICT` set to
/// `strict` or unset. Returns success and the combined output.
fn run(root: &Path, args: &[&OsStr], strict: Option<&str>) -> (bool, String) {
    let mut cmd = Command::new("bash");
    cmd.arg(root.join("scripts/check-intents.sh")).args(args);
    match strict {
        Some(v) => cmd.env("INTENTS_STRICT", v),
        None => cmd.env_remove("INTENTS_STRICT"),
    };
    let out = cmd.output().expect("run the script");
    let text = format!(
        "{}{}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );
    (out.status.success(), text)
}

const GOOD: &str = r#"[{"intent":"a-one","required":true},{"intent":"b-two","required":false}]"#;

#[test]
fn a_sorted_list_passes_and_skips_the_binary_compare() {
    let tree = Tree::new(GOOD);
    let (ok, out) = run(&tree.0, &[], None);
    assert!(ok, "{out}");
    assert!(out.contains("PASS intents.json lists 2 intents"), "{out}");
    assert!(out.contains("SKIP no playbook binary given"), "{out}");
}

#[test]
fn a_bad_list_fails() {
    let cases = [
        (
            r#"[{"intent":"b-two","required":true},{"intent":"a-one","required":true}]"#,
            "sorted and unique",
        ),
        (r#"[{"intent":"Not_Kebab","required":true}]"#, "kebab-case"),
        (
            r#"[{"intent":"a-one","required":"true"}]"#,
            "boolean required",
        ),
        ("[]", "non-empty array"),
    ];
    for (intents, needle) in cases {
        let tree = Tree::new(intents);
        let (ok, out) = run(&tree.0, &[], None);
        assert!(!ok, "{intents}: {out}");
        assert!(out.contains(needle), "{intents}: {out}");
    }
}

#[test]
fn a_tool_map_with_every_intent_passes() {
    let tree = Tree::new(GOOD);
    let bin = tree.stub(r#"[{"intent":"a-one"},{"intent":"b-two"}]"#, 0);
    let (ok, out) = run(&tree.0, &[bin.as_os_str()], Some("1"));
    assert!(ok, "{out}");
    assert!(!out.contains("WARN"), "{out}");
}

#[test]
fn a_missing_optional_intent_warns_and_a_missing_required_one_fails() {
    let tree = Tree::new(GOOD);
    let only_required = tree.stub(r#"[{"intent":"a-one"}]"#, 0);
    let (ok, out) = run(&tree.0, &[only_required.as_os_str()], None);
    assert!(ok, "{out}");
    assert!(out.contains("WARN intent b-two"), "{out}");

    let only_optional = tree.stub(r#"[{"intent":"b-two"}]"#, 0);
    let (ok, out) = run(&tree.0, &[only_optional.as_os_str()], None);
    assert!(!ok, "{out}");
    assert!(out.contains("FAIL required intent a-one"), "{out}");
}

#[test]
fn a_binary_without_a_tool_map_fails_only_when_strict() {
    let tree = Tree::new(GOOD);
    let bin = tree.stub("unknown command", 2);
    let (ok, out) = run(&tree.0, &[bin.as_os_str()], Some("1"));
    assert!(!ok, "{out}");
    assert!(out.contains("gave no tools map"), "{out}");

    let (ok, out) = run(&tree.0, &[bin.as_os_str()], None);
    assert!(ok, "{out}");
    assert!(out.contains("has no tools map yet"), "{out}");
}

#[test]
fn this_repository_list_is_valid() {
    let (ok, out) = run(&repo_root(), &[], None);
    assert!(ok, "{out}");
}
