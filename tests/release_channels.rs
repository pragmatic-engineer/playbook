// SPDX-FileCopyrightText: 2026 Igor Santos
// SPDX-License-Identifier: MIT

//! `playbook release render-formula` and `pin-marketplace`: the behavior
//! `shell/release-channels.test.sh` pinned, plus byte-for-byte goldens
//! captured from the shell scripts they replace.

use serde_json::Value;
use std::path::PathBuf;
use std::process::{Command, Stdio};

fn fixture(name: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/release-channels")
        .join(name)
}

struct Run {
    stdout: String,
    stderr: String,
    ok: bool,
}

fn run(args: &[&str]) -> Run {
    let out = Command::new(env!("CARGO_BIN_EXE_playbook"))
        .arg("release")
        .args(args)
        .stdin(Stdio::null())
        .output()
        .expect("playbook spawns");
    Run {
        stdout: String::from_utf8_lossy(&out.stdout).into_owned(),
        stderr: String::from_utf8_lossy(&out.stderr).into_owned(),
        ok: out.status.success(),
    }
}

fn formula(version: &str) -> Run {
    run(&[
        "render-formula",
        version,
        fixture("SHA256SUMS").to_str().unwrap(),
    ])
}

fn pin(version: &str, file: &str, sha: &str) -> Run {
    run(&[
        "pin-marketplace",
        version,
        fixture(file).to_str().unwrap(),
        sha,
    ])
}

fn sha(c: char) -> String {
    c.to_string().repeat(64)
}

fn source(market: &str) -> Value {
    let doc: Value = serde_json::from_str(market).unwrap();
    doc["plugins"]
        .as_array()
        .unwrap()
        .iter()
        .find(|p| p["name"] == "playbook")
        .unwrap()["source"]
        .clone()
}

#[test]
fn formula_matches_the_shell_output_byte_for_byte() {
    let want = std::fs::read_to_string(fixture("formula.expected.rb")).unwrap();
    assert_eq!(formula("9.8.7").stdout, want);
}

#[test]
fn formula_fills_each_target_sha_next_to_its_own_url() {
    let out = formula("9.8.7").stdout;
    assert!(!out.contains('@'), "placeholder left behind");
    assert_eq!(out.matches("download/v9.8.7/playbook-9.8.7-").count(), 4);
    for (target, digit) in [
        ("aarch64-apple-darwin", '1'),
        ("x86_64-apple-darwin", '2'),
        ("aarch64-unknown-linux-musl", '3'),
        ("x86_64-unknown-linux-musl", '4'),
    ] {
        let lines: Vec<&str> = out.lines().collect();
        let at = lines
            .iter()
            .position(|l| l.contains(&format!("{target}\"")))
            .unwrap_or_else(|| panic!("no url for {target}"));
        assert!(
            lines[at + 1].contains(&format!("sha256 \"{}\"", sha(digit))),
            "sha for {target} not on the next line"
        );
    }
    assert!(!out.contains("5555"), "windows sha must not be used");
}

#[test]
fn formula_fails_when_the_version_is_missing_from_the_sums() {
    assert!(!formula("9.8.6").ok);
}

#[test]
fn formula_rejects_a_v_prefix_and_a_quote() {
    let v = formula("v9.8.7");
    assert!(!v.ok && v.stderr.contains("bad version"));
    assert!(!formula("9.8.7\"x").ok);
}

#[test]
fn pin_matches_the_shell_output_byte_for_byte() {
    let want = std::fs::read_to_string(fixture("marketplace.pinned.expected.json")).unwrap();
    assert_eq!(pin("9.8.7", "marketplace.json", &sha('a')).stdout, want);
}

#[test]
fn pin_sets_an_archive_source_and_keeps_everything_else() {
    let out = pin("9.8.7", "marketplace.json", &sha('a')).stdout;
    let src = source(&out);
    assert_eq!(src["source"], "archive");
    assert_eq!(
        src["url"],
        "https://github.com/pragmatic-engineer/playbook/releases/download/v9.8.7/playbook-plugin-9.8.7.zip"
    );
    assert_eq!(src["sha256"], sha('a'));
    let mut keys: Vec<&String> = src.as_object().unwrap().keys().collect();
    keys.sort();
    assert_eq!(keys, ["sha256", "source", "url"], "no git ref left behind");
    let doc: Value = serde_json::from_str(&out).unwrap();
    let plugins = doc["plugins"].as_array().unwrap();
    assert_eq!(plugins[1]["category"], "workflow");
    assert_eq!(
        plugins[0]["source"].to_string(),
        r#"{"source":"url","url":"https://example.com/other.git"}"#
    );
}

#[test]
fn repinning_replaces_the_url_and_the_hash() {
    let first = pin("9.8.7", "marketplace.json", &sha('a')).stdout;
    let tmp = std::env::temp_dir().join(format!("pin-repin-{}.json", std::process::id()));
    std::fs::write(&tmp, first).unwrap();
    let again = run(&["pin-marketplace", "9.8.8", tmp.to_str().unwrap(), &sha('b')]);
    let _ = std::fs::remove_file(&tmp);
    let src = source(&again.stdout);
    assert!(src["url"]
        .as_str()
        .unwrap()
        .contains("v9.8.8/playbook-plugin-9.8.8.zip"));
    assert_eq!(src["sha256"], sha('b'));
}

#[test]
fn pin_rejects_bad_input() {
    assert!(!pin("9.8.7", "marketplace-empty.json", &sha('a')).ok);
    assert!(!pin("9.8.7", "marketplace.json", "abc123").ok);
    assert!(!pin("9.8.7", "marketplace.json", &sha('a').to_uppercase()).ok);
    assert!(!pin("v9.8.7", "marketplace.json", &sha('a')).ok);
    assert!(
        !run(&[
            "pin-marketplace",
            "9.8.7",
            "/nonexistent/market.json",
            &sha('a')
        ])
        .ok
    );
}
