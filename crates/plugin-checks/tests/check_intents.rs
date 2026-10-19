// SPDX-FileCopyrightText: 2026 Igor Santos
// SPDX-License-Identifier: Apache-2.0

//! Runs `scripts/check-intents.sh` on throwaway copies of the repo layout: a
//! sorted list passes, an unsorted list, a bad id and an empty list fail, and
//! a stub binary whose tool map lacks a required intent fails. A last check
//! runs it on this repository's `plugins/intents.json`.

use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::atomic::{AtomicU64, Ordering};

static COUNTER: AtomicU64 = AtomicU64::new(0);

fn repo_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../..")
}

/// A scratch tree holding `scripts/check-intents.sh` and `plugins/intents.json`
/// with `intents`. The script finds the list next to itself.
fn tree(intents: &str) -> PathBuf {
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
    dir
}

fn run(dir: &Path, args: &[&str]) -> (bool, String) {
    let out = Command::new("bash")
        .arg(dir.join("scripts/check-intents.sh"))
        .args(args)
        .output()
        .expect("run the script");
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
    let dir = tree(GOOD);
    let (ok, out) = run(&dir, &[]);
    assert!(ok, "{out}");
    assert!(out.contains("PASS intents.json lists 2 intents"), "{out}");
    assert!(out.contains("SKIP no playbook binary given"), "{out}");
}

#[test]
fn an_unsorted_list_a_bad_id_and_an_empty_list_fail() {
    let cases = [
        (
            r#"[{"intent":"b-two","required":true},{"intent":"a-one","required":true}]"#,
            "sorted and unique",
        ),
        (r#"[{"intent":"Not_Kebab","required":true}]"#, "kebab-case"),
        ("[]", "non-empty array"),
    ];
    for (intents, needle) in cases {
        let dir = tree(intents);
        let (ok, out) = run(&dir, &[]);
        assert!(!ok, "{intents}: {out}");
        assert!(out.contains(needle), "{intents}: {out}");
    }
}

#[test]
fn a_required_intent_missing_from_the_tool_map_fails() {
    let dir = tree(GOOD);
    let bin = dir.join("fake-playbook");
    fs::write(&bin, "#!/bin/sh\necho '[{\"intent\":\"b-two\"}]'\n").expect("write the stub");
    let status = Command::new("chmod")
        .arg("+x")
        .arg(&bin)
        .status()
        .expect("chmod");
    assert!(status.success());
    let (ok, out) = run(&dir, &[bin.to_str().expect("utf-8 path")]);
    assert!(!ok, "{out}");
    assert!(out.contains("FAIL required intent a-one"), "{out}");
}

#[test]
fn this_repository_list_is_valid() {
    let (ok, out) = run(&repo_root(), &[]);
    assert!(ok, "{out}");
}
