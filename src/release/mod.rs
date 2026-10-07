// SPDX-FileCopyrightText: 2026 Igor Santos
// SPDX-License-Identifier: MIT

//! `playbook release`: the two file renderers the `publish-channels` release
//! job runs, ported from `shell/render-formula.sh` and
//! `shell/pin-marketplace.sh`. Both print to stdout and never touch the
//! network.

use regex::Regex;
use serde_json::{json, Value};

const FORMULA_TEMPLATE: &str = include_str!("formula.rb.tmpl");

/// Release targets whose binaries the Homebrew formula points at, in the
/// order the template lists them.
const FORMULA_TARGETS: [&str; 4] = [
    "aarch64-apple-darwin",
    "x86_64-apple-darwin",
    "aarch64-unknown-linux-musl",
    "x86_64-unknown-linux-musl",
];

fn is_sha256(s: &str) -> bool {
    s.len() == 64 && s.bytes().all(|b| matches!(b, b'0'..=b'9' | b'a'..=b'f'))
}

/// The sha256 for `file` in a `SHA256SUMS` body, accepting the binary-mode
/// `*` prefix. `None` unless exactly one valid lowercase hash matches.
fn sum_for(sums: &str, file: &str) -> Option<String> {
    let starred = format!("*{file}");
    let mut hits = sums.lines().filter_map(|line| {
        let mut fields = line.split_whitespace();
        let (hash, name) = (fields.next()?, fields.next()?);
        (name == file || name == starred).then(|| hash.to_string())
    });
    let first = hits.next()?;
    (hits.next().is_none() && is_sha256(&first)).then_some(first)
}

/// Renders the Homebrew formula for `version` from the release's
/// `SHA256SUMS` text.
pub fn render_formula(version: &str, sums: &str) -> Result<String, String> {
    if !Regex::new(r"^[0-9]+\.[0-9]+\.[0-9]+$")
        .expect("static regex")
        .is_match(version)
    {
        return Err(format!("bad version '{version}'"));
    }
    let mut out = FORMULA_TEMPLATE.replace("@VERSION@", version);
    for target in FORMULA_TARGETS {
        let file = format!("playbook-{version}-{target}");
        let sha = sum_for(sums, &file)
            .ok_or_else(|| format!("no valid sha256 for {target} in SHA256SUMS"))?;
        let key = target.to_uppercase().replace('-', "_");
        out = out.replace(&format!("@SHA_{key}@"), &sha);
    }
    Ok(format!("{}\n", out.trim_end_matches('\n')))
}

/// Returns `marketplace` (JSON text) with the `playbook` plugin source set to
/// the release's plugin archive, pretty-printed with a trailing newline.
pub fn pin_marketplace(version: &str, marketplace: &str, sha256: &str) -> Result<String, String> {
    if !Regex::new(r"^[0-9]+\.[0-9]+\.[0-9]+([-+.][0-9A-Za-z.+-]+)?$")
        .expect("static regex")
        .is_match(version)
    {
        return Err(format!("bad version: {version}"));
    }
    if !is_sha256(sha256) {
        return Err(format!("bad sha256: {sha256}"));
    }
    let mut doc: Value =
        serde_json::from_str(marketplace).map_err(|e| format!("marketplace is not JSON: {e}"))?;
    let entry = doc
        .get_mut("plugins")
        .and_then(Value::as_array_mut)
        .and_then(|plugins| {
            plugins
                .iter_mut()
                .find(|p| p.get("name").and_then(Value::as_str) == Some("playbook"))
        })
        .ok_or("no playbook plugin entry")?;
    let url = format!(
        "https://github.com/pragmatic-engineer/playbook/releases/download/v{version}/playbook-plugin-{version}.zip"
    );
    entry["source"] = json!({"source": "archive", "url": url, "sha256": sha256});
    let mut text = serde_json::to_string_pretty(&doc).map_err(|e| e.to_string())?;
    text.push('\n');
    Ok(text)
}
