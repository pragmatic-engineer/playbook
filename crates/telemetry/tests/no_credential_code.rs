// SPDX-FileCopyrightText: 2026 Igor Santos
// SPDX-License-Identifier: Apache-2.0

//! Keeps the decision "limits come from what Claude Code already reports, never
//! from the login token" enforceable: no product source names the Anthropic API
//! host, and the telemetry crate has no HTTP client.

use std::fs;
use std::path::{Path, PathBuf};

fn crates_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .expect("the telemetry crate sits in the crates folder")
        .to_path_buf()
}

fn rust_files(dir: &Path, out: &mut Vec<PathBuf>) {
    let Ok(entries) = fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            rust_files(&path, out);
        } else if path.extension().is_some_and(|e| e == "rs") {
            out.push(path);
        }
    }
}

/// Names a manifest declares: `name = ...`, `name.workspace = ...` and
/// `[dependencies.name]` headers.
fn dependency_names(text: &str) -> Vec<String> {
    text.lines()
        .filter_map(|l| {
            let l = l.trim();
            if let Some(table) = l.strip_prefix('[').and_then(|t| t.strip_suffix(']')) {
                return table
                    .rsplit_once("dependencies.")
                    .map(|(_, name)| name.to_string());
            }
            let (key, _) = l.split_once('=')?;
            Some(key.split('.').next()?.trim().to_string())
        })
        .collect()
}

#[test]
fn no_product_source_names_the_anthropic_api_host() {
    // Built in two parts so this file does not name the host itself.
    let host = ["api.", "anthropic", ".com"].concat();
    let mut files = Vec::new();
    for krate in fs::read_dir(crates_dir()).expect("crates folder").flatten() {
        rust_files(&krate.path().join("src"), &mut files);
    }
    assert!(!files.is_empty());

    let hits: Vec<_> = files
        .iter()
        .filter(|f| fs::read_to_string(f).is_ok_and(|text| text.contains(&host)))
        .collect();

    assert!(hits.is_empty(), "{hits:?}");
}

#[test]
fn the_telemetry_crate_has_no_http_client_and_the_cli_adds_none_beyond_ureq() {
    let clients = [
        "ureq",
        "reqwest",
        "hyper",
        "attohttpc",
        "isahc",
        "surf",
        "curl",
        "minreq",
    ];
    let manifest = |krate: &str| {
        fs::read_to_string(crates_dir().join(krate).join("Cargo.toml")).expect("manifest")
    };
    let telemetry = dependency_names(&manifest("telemetry"));
    let cli = dependency_names(&manifest("cli"));

    for client in clients {
        assert!(
            !telemetry.iter().any(|n| n == client),
            "telemetry: {client}"
        );
        if client != "ureq" {
            assert!(!cli.iter().any(|n| n == client), "cli: {client}");
        }
    }
}

#[test]
fn the_name_reader_sees_dotted_keys_and_dependency_tables() {
    let text =
        "[dependencies]\nreqwest.workspace = true\n[dev-dependencies.curl]\nversion = \"1\"\n";

    let found = dependency_names(text);

    assert!(found.contains(&"reqwest".to_string()));
    assert!(found.contains(&"curl".to_string()));
}
