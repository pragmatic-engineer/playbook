// SPDX-FileCopyrightText: 2026 Igor Santos
// SPDX-License-Identifier: Apache-2.0

//! Runs `scripts/check-links.sh` on small throwaway trees: good links pass,
//! a missing file and a missing anchor fail, code and web links are ignored.
//! A last check runs it on this repository, so a broken link fails the build.

use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

fn script() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../scripts/check-links.sh")
}

/// Writes `files` under a fresh directory and runs the script there.
fn run(name: &str, files: &[(&str, &str)]) -> (bool, String) {
    let dir = std::env::temp_dir().join(format!("check-links-{}-{name}", std::process::id()));
    let _ = fs::remove_dir_all(&dir);
    for (rel, text) in files {
        let path = dir.join(rel);
        fs::create_dir_all(path.parent().expect("parent")).expect("dirs");
        fs::write(path, text).expect("write");
    }
    let out = Command::new("bash")
        .arg(script())
        .arg(&dir)
        .output()
        .expect("run");
    let _ = fs::remove_dir_all(&dir);
    (
        out.status.success(),
        String::from_utf8_lossy(&out.stdout).into_owned(),
    )
}

#[test]
fn good_links_pass() {
    let (ok, out) = run("good", &[
        ("a.md", "# Top\n\n## Config keys (v2)\n\n[b](docs/b.md) [k](#config-keys-v2) [t](docs/b.md#the-b-page)\n[w](https://example.invalid/x)\n"),
        ("docs/b.md", "# The B page\n\nback to [a](../a.md#top)\n"),
    ]);
    assert!(ok, "{out}");
}

#[test]
fn repeated_headings_get_numbered_anchors() {
    let (ok, out) = run("dup", &[("a.md", "# Same\n\n# Same\n\n[two](#same-1)\n")]);
    assert!(ok, "{out}");
}

#[test]
fn missing_file_and_anchor_fail() {
    let (ok, out) = run(
        "bad",
        &[
            (
                "a.md",
                "# Top\n\n[x](gone.md)\n[y](#nope)\n[z](b.md#nope)\n",
            ),
            ("b.md", "# B\n"),
        ],
    );
    assert!(!ok);
    assert!(out.contains("a.md:3 missing file: gone.md"), "{out}");
    assert!(out.contains("a.md:4 missing anchor: #nope"), "{out}");
    assert!(out.contains("a.md:5 missing anchor: b.md#nope"), "{out}");
}

#[test]
fn code_is_ignored() {
    let (ok, out) = run(
        "code",
        &[(
            "a.md",
            "# A\n\n```\n[x](gone.md)\n```\n\nSee `- [T](file.md): note\nmore` here.\n",
        )],
    );
    assert!(ok, "{out}");
}

#[test]
fn this_repository_has_no_broken_links() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let out = Command::new("bash")
        .arg(script())
        .arg(&root)
        .output()
        .expect("run");
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stdout)
    );
}
