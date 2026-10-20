// SPDX-FileCopyrightText: 2026 Igor Santos
// SPDX-License-Identifier: Apache-2.0

//! The `playbook` binary must never link the document converter: `anydoc` and
//! `pdf-inspector` belong to `pbdoc` alone, so the main binary stays small and
//! its supply chain stays unchanged.

use std::collections::{BTreeSet, HashMap};
use std::process::Command;

use serde_json::{json, Value};

const ROOT_PACKAGE: &str = "playbook";
const FORBIDDEN_PACKAGES: [&str; 2] = ["anydoc", "pdf-inspector"];
const DEV_KIND: &str = "dev";

fn forbidden_in_closure(metadata: &Value, root: &str) -> Vec<String> {
    let names: HashMap<&str, &str> = metadata["packages"]
        .as_array()
        .expect("metadata has packages")
        .iter()
        .map(|package| {
            (
                package["id"].as_str().expect("package id"),
                package["name"].as_str().expect("package name"),
            )
        })
        .collect();
    let root_id = names
        .iter()
        .find(|(_, name)| **name == root)
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
        for dep in nodes[id]["deps"].as_array().expect("node deps") {
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
    seen.into_iter()
        .map(|id| names[id])
        .filter(|name| FORBIDDEN_PACKAGES.contains(name))
        .map(str::to_owned)
        .collect()
}

fn real_metadata() -> Value {
    let cargo = std::env::var("CARGO").expect("cargo sets CARGO for tests");
    let output = Command::new(cargo)
        .args(["metadata", "--format-version", "1", "--locked", "--offline"])
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
fn a_converter_reached_through_another_crate_is_reported() {
    // Arrange
    let normal = json!([{"kind": null, "target": null}]);
    let metadata = json!({
        "packages": [
            {"id": "p", "name": ROOT_PACKAGE},
            {"id": "x", "name": "x"},
            {"id": "a", "name": "anydoc"},
        ],
        "resolve": {"nodes": [
            {"id": "p", "deps": [{"pkg": "x", "dep_kinds": normal}]},
            {"id": "x", "deps": [{"pkg": "a", "dep_kinds": normal}]},
            {"id": "a", "deps": []},
        ]},
    });

    // Act
    let found = forbidden_in_closure(&metadata, ROOT_PACKAGE);

    // Assert
    assert_eq!(found, vec!["anydoc".to_owned()]);
}

#[test]
fn a_converter_reached_only_through_a_dev_dependency_is_not_reported() {
    // Arrange
    let metadata = json!({
        "packages": [
            {"id": "p", "name": ROOT_PACKAGE},
            {"id": "a", "name": "pdf-inspector"},
        ],
        "resolve": {"nodes": [
            {"id": "p", "deps": [{"pkg": "a", "dep_kinds": [{"kind": DEV_KIND, "target": null}]}]},
            {"id": "a", "deps": []},
        ]},
    });

    // Act
    let found = forbidden_in_closure(&metadata, ROOT_PACKAGE);

    // Assert
    assert!(
        found.is_empty(),
        "dev edges are outside the closure: {found:?}"
    );
}

#[test]
fn the_real_playbook_closure_excludes_the_converter() {
    // Arrange
    let metadata = real_metadata();

    // Act
    let found = forbidden_in_closure(&metadata, ROOT_PACKAGE);

    // Assert
    assert!(
        found.is_empty(),
        "converter crates linked into playbook: {found:?}"
    );
}
