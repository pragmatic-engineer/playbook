// SPDX-FileCopyrightText: 2026 Igor Santos
// SPDX-License-Identifier: Apache-2.0

//! Supply chain guards for `pbdoc`: its normal and build dependency closure
//! must stay offline, free of OCR, under allowed licences and listed in
//! `NOTICE-pbdoc`. The guards are pure functions over `cargo metadata` JSON so
//! the negative cases run on synthetic graphs.

use std::collections::{BTreeSet, HashMap};
use std::path::Path;
use std::process::Command;

use serde_json::{json, Value};

const ROOT_PACKAGE: &str = "pbdoc";
const NOTICE_FILE: &str = "NOTICE-pbdoc";
const DEV_KIND: &str = "dev";
const DENIED_NAMES: [&str; 13] = [
    "hyper",
    "reqwest",
    "ureq",
    "rustls",
    "native-tls",
    "openssl",
    "h2",
    "http",
    "curl",
    "tokio",
    "mio",
    "socket2",
    "firecrawl",
];
const DENIED_FRAGMENTS: [&str; 2] = ["ocr", "tesseract"];
const ALLOWED_LICENCES: [&str; 11] = [
    "MIT",
    "Apache-2.0",
    "BSD-2-Clause",
    "BSD-3-Clause",
    "ISC",
    "Zlib",
    "Unicode-3.0",
    "Unlicense",
    "CC0-1.0",
    "MPL-2.0",
    "0BSD",
];

fn package_by_id(metadata: &Value) -> HashMap<&str, &Value> {
    metadata["packages"]
        .as_array()
        .expect("metadata has packages")
        .iter()
        .map(|package| (package["id"].as_str().expect("package id"), package))
        .collect()
}

/// Packages reachable from `root` over normal and build edges, root excluded.
fn closure<'a>(metadata: &'a Value, root: &str) -> Vec<&'a Value> {
    let packages = package_by_id(metadata);
    let root_id = packages
        .iter()
        .find(|(_, package)| package["name"] == root)
        .map(|(id, _)| *id)
        .unwrap_or_else(|| panic!("root package {root} is not in the metadata"));
    let nodes: HashMap<&str, &Value> = metadata["resolve"]["nodes"]
        .as_array()
        .expect("metadata has resolve nodes")
        .iter()
        .map(|node| (node["id"].as_str().expect("node id"), node))
        .collect();

    let mut seen: BTreeSet<&str> = BTreeSet::from([root_id]);
    let mut pending = vec![root_id];
    while let Some(id) = pending.pop() {
        let deps = nodes[id]["deps"].as_array().expect("node deps");
        for dep in deps {
            let reaches_runtime = dep["dep_kinds"]
                .as_array()
                .expect("dep kinds")
                .iter()
                .any(|kind| kind["kind"].as_str() != Some(DEV_KIND));
            let target = dep["pkg"].as_str().expect("dep pkg");
            if reaches_runtime && seen.insert(target) {
                pending.push(target);
            }
        }
    }
    seen.remove(root_id);
    seen.into_iter().map(|id| packages[id]).collect()
}

fn name_of(package: &Value) -> &str {
    package["name"].as_str().expect("package name")
}

fn denied_names(metadata: &Value, root: &str) -> Vec<String> {
    closure(metadata, root)
        .into_iter()
        .map(name_of)
        .filter(|name| {
            DENIED_NAMES.contains(name) || DENIED_FRAGMENTS.iter().any(|part| name.contains(part))
        })
        .map(str::to_owned)
        .collect()
}

fn licence_option_is_allowed(option: &str) -> bool {
    option
        .split(" AND ")
        .map(|term| term.split(" WITH ").next().unwrap_or(term))
        .all(|licence| ALLOWED_LICENCES.contains(&licence.trim().trim_matches(['(', ')'])))
}

fn licence_allows_some_option(expression: &str) -> bool {
    expression
        .replace('/', " OR ")
        .split(" OR ")
        .any(|option| licence_option_is_allowed(option.trim().trim_matches(['(', ')'])))
}

fn disallowed_licences(metadata: &Value, root: &str) -> Vec<String> {
    closure(metadata, root)
        .into_iter()
        .filter(|package| {
            !package["license"]
                .as_str()
                .is_some_and(licence_allows_some_option)
        })
        .map(|package| name_of(package).to_owned())
        .collect()
}

/// True when `name` appears in `notice` as a whole token, so `adler` does not
/// match a line that only names `adler2`.
fn notice_lists(notice: &str, name: &str) -> bool {
    notice
        .split(|c: char| !(c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.')))
        .any(|token| token == name)
}

fn missing_from_notice(metadata: &Value, root: &str, notice: &str) -> Vec<String> {
    closure(metadata, root)
        .into_iter()
        .map(name_of)
        .filter(|name| !notice_lists(notice, name))
        .map(str::to_owned)
        .collect()
}

/// A graph `root -> dependency`, with the edge kind (`null` for normal) and
/// the dependency's licence set by the caller.
fn two_node_metadata(dependency: &str, edge_kind: Value, licence: &str) -> Value {
    json!({
        "packages": [
            {"id": "root-id", "name": ROOT_PACKAGE, "license": "Apache-2.0"},
            {"id": "dep-id", "name": dependency, "license": licence},
        ],
        "resolve": {"nodes": [
            {"id": "root-id", "deps": [
                {"pkg": "dep-id", "dep_kinds": [{"kind": edge_kind, "target": null}]},
            ]},
            {"id": "dep-id", "deps": []},
        ]},
    })
}

fn real_metadata() -> Value {
    let cargo = std::env::var("CARGO").expect("cargo sets CARGO for tests");
    let output = Command::new(cargo)
        .args(["metadata", "--format-version", "1", "--locked"])
        .current_dir(env!("CARGO_MANIFEST_DIR"))
        .output()
        .expect("cargo metadata starts");
    assert!(
        output.status.success(),
        "cargo metadata failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    serde_json::from_slice(&output.stdout).expect("cargo metadata prints JSON")
}

#[test]
fn every_denied_name_in_the_closure_is_reported() {
    for name in DENIED_NAMES
        .into_iter()
        .chain(["some-ocr-engine", "tesseract-sys"])
    {
        // Arrange
        let metadata = two_node_metadata(name, Value::Null, "MIT");

        // Act
        let denied = denied_names(&metadata, ROOT_PACKAGE);

        // Assert
        assert_eq!(denied, vec![name.to_owned()], "{name} must be reported");
    }
}

#[test]
fn a_crate_reached_only_through_a_dev_dependency_is_not_reported() {
    // Arrange
    let metadata = two_node_metadata("reqwest", json!(DEV_KIND), "MIT");

    // Act
    let denied = denied_names(&metadata, ROOT_PACKAGE);

    // Assert
    assert!(
        denied.is_empty(),
        "dev edges are outside the closure: {denied:?}"
    );
}

#[test]
fn a_crate_reached_through_a_build_dependency_is_reported() {
    // Arrange
    let metadata = two_node_metadata("openssl", json!("build"), "MIT");

    // Act
    let denied = denied_names(&metadata, ROOT_PACKAGE);

    // Assert
    assert_eq!(denied, vec!["openssl".to_owned()]);
}

#[test]
fn a_gpl_only_crate_is_reported() {
    // Arrange
    let metadata = two_node_metadata("copyleft", Value::Null, "GPL-3.0-only");

    // Act
    let disallowed = disallowed_licences(&metadata, ROOT_PACKAGE);

    // Assert
    assert_eq!(disallowed, vec!["copyleft".to_owned()]);
}

#[test]
fn a_dual_licence_with_an_allowed_option_passes() {
    // Arrange
    let metadata = two_node_metadata("dual", Value::Null, "MIT OR GPL-3.0-only");

    // Act
    let disallowed = disallowed_licences(&metadata, ROOT_PACKAGE);

    // Assert
    assert!(
        disallowed.is_empty(),
        "an allowed option is enough: {disallowed:?}"
    );
}

#[test]
fn a_crate_with_no_licence_is_reported() {
    // Arrange
    let mut metadata = two_node_metadata("unlicensed", Value::Null, "MIT");
    metadata["packages"][1]["license"] = Value::Null;

    // Act
    let disallowed = disallowed_licences(&metadata, ROOT_PACKAGE);

    // Assert
    assert_eq!(disallowed, vec!["unlicensed".to_owned()]);
}

#[test]
fn a_crate_absent_from_the_notice_text_is_reported_by_name() {
    // Arrange
    let metadata = two_node_metadata("forgotten-crate", Value::Null, "MIT");

    // Act
    let missing = missing_from_notice(&metadata, ROOT_PACKAGE, "pbdoc bundles other-crate only");

    // Assert
    assert_eq!(missing, vec!["forgotten-crate".to_owned()]);
}

#[test]
fn a_crate_named_in_the_notice_text_is_not_reported() {
    // Arrange
    let metadata = two_node_metadata("listed-crate", Value::Null, "MIT");

    // Act
    let missing = missing_from_notice(&metadata, ROOT_PACKAGE, "bundles listed-crate 1.0");

    // Assert
    assert!(missing.is_empty(), "listed crates pass: {missing:?}");
}

#[test]
fn the_real_closure_has_no_denied_crates() {
    // Arrange
    let metadata = real_metadata();

    // Act
    let denied = denied_names(&metadata, ROOT_PACKAGE);

    // Assert
    assert!(
        denied.is_empty(),
        "denied crates in the closure: {denied:?}"
    );
}

#[test]
fn the_real_closure_has_only_allowed_licences() {
    // Arrange
    let metadata = real_metadata();

    // Act
    let disallowed = disallowed_licences(&metadata, ROOT_PACKAGE);

    // Assert
    assert!(
        disallowed.is_empty(),
        "crates without an allowed licence: {disallowed:?}"
    );
}

#[test]
fn the_real_closure_is_fully_listed_in_the_notice_file() {
    // Arrange
    let metadata = real_metadata();
    let notice_path = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .join(NOTICE_FILE);
    let notice = std::fs::read_to_string(&notice_path)
        .unwrap_or_else(|error| panic!("{NOTICE_FILE} must exist and be readable: {error}"));

    // Act
    let missing = missing_from_notice(&metadata, ROOT_PACKAGE, &notice);

    // Assert
    assert!(
        missing.is_empty(),
        "crates missing from {NOTICE_FILE}: {missing:?}"
    );
}

#[test]
fn a_notice_line_for_adler2_does_not_list_adler() {
    // Arrange
    let notice = "adler2 2.0.1 MIT\nsee https://example.com";

    // Assert
    assert!(!notice_lists(notice, "adler"));
    assert!(notice_lists(notice, "adler2"));
    assert!(!notice_lists(notice, "http"));
}

#[test]
fn the_anydoc_pin_matches_the_converter_version_the_binary_prints() {
    // Arrange
    let manifest = include_str!("../Cargo.toml");
    let line = manifest
        .lines()
        .find(|line| line.starts_with("anydoc ="))
        .expect("manifest pins anydoc");
    let pin = line
        .split_once("version = \"=")
        .and_then(|(_, rest)| rest.split('"').next())
        .expect("anydoc has an exact pin");

    // Act
    let output = Command::new(env!("CARGO_BIN_EXE_pbdoc"))
        .arg("--converter-version")
        .output()
        .expect("pbdoc starts");

    // Assert
    assert_eq!(
        String::from_utf8_lossy(&output.stdout).trim_end(),
        format!("anydoc {pin}")
    );
}
