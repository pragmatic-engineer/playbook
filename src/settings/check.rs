// SPDX-FileCopyrightText: 2026 Igor Santos
// SPDX-License-Identifier: MIT

//! Port of `shell/check-shared-settings.py`, guarding what ships in
//! `settings.shared.json`: a seed that pinned a model, leaked a personal key or
//! named a hook that does not exist would be installed verbatim everywhere.
//!
//! Message wording and exit status match the python exactly, because
//! `tests/settings_check.rs` diffs the two implementations.

use serde_json::Value;
use std::path::Path;

/// Meaningful only on one developer's machine, so shipping any of them pushes
/// local preferences onto every install.
const PERSONAL_KEYS: [&str; 4] = [
    "effortLevel",
    "theme",
    "preferredNotifChannel",
    "prefersReducedMotion",
];

/// Resolved on PATH rather than inside the repo, so there is no file to check.
const EXTERNAL_COMMANDS: [&str; 2] = ["rtk", "playbook"];

/// Name the install location, which maps onto the repo root here.
const INSTALL_PREFIXES: [&str; 2] = ["~/.claude/", "$HOME/.claude/"];

const BASH_WRAPPER: &str = "bash ";

pub fn check(
    template_path: &Path,
    permissions_path: &Path,
    repo_root: &Path,
) -> Result<String, String> {
    let template_display = template_path.display();
    let permissions_display = permissions_path.display();

    let template_raw = std::fs::read_to_string(template_path)
        .map_err(|_| format!("template not readable: {template_display}"))?;
    let permissions_raw = std::fs::read_to_string(permissions_path)
        .map_err(|_| format!("permissions not readable: {permissions_display}"))?;
    if !repo_root.is_dir() {
        return Err(format!(
            "repo root is not a directory: {}",
            repo_root.display()
        ));
    }

    let template: Value = serde_json::from_str(&template_raw)
        .map_err(|_| format!("template is not valid JSON: {template_display}"))?;
    let permissions: Value = serde_json::from_str(&permissions_raw)
        .map_err(|_| format!("permissions is not valid JSON: {permissions_display}"))?;

    if !permissions.is_object() {
        return Err(format!(
            "permissions file is not a JSON object: {permissions_display}"
        ));
    }

    match template.get("permissions") {
        Some(block) if block.is_object() => {
            if *block != permissions {
                return Err(format!(
                    ".permissions in template does not deep-equal {permissions_display}"
                ));
            }
        }
        _ => {
            return Err(format!(
                ".permissions is missing or not an object in {template_display}"
            ))
        }
    }

    if template.get("model").is_some() {
        return Err(format!(
            ".model must not ship in {template_display} (the harness or user picks the model)"
        ));
    }

    if template.get("skipAutoPermissionPrompt") != Some(&Value::Bool(false)) {
        return Err(format!(
            ".skipAutoPermissionPrompt must be false in {template_display}"
        ));
    }

    for key in PERSONAL_KEYS {
        if template.get(key).is_some() {
            return Err(format!("personal key must be absent from template: {key}"));
        }
    }

    check_hook_commands(&template, repo_root)?;

    Ok(format!("check-shared-settings: OK ({template_display})"))
}

/// Malformed shapes are skipped rather than rejected, matching the python's
/// isinstance guards: this validates commands, and shape is the settings
/// schema's concern.
fn check_hook_commands(template: &Value, repo_root: &Path) -> Result<(), String> {
    let Some(hooks) = template.get("hooks").and_then(Value::as_object) else {
        return Ok(());
    };

    for entries in hooks.values() {
        let Some(entries) = entries.as_array() else {
            continue;
        };
        for entry in entries {
            let Some(inner) = entry.get("hooks").and_then(Value::as_array) else {
                continue;
            };
            for hook in inner {
                if let Some(cmd) = hook.get("command").and_then(Value::as_str) {
                    check_one_command(cmd, repo_root)?;
                }
            }
        }
    }
    Ok(())
}

fn check_one_command(cmd: &str, repo_root: &Path) -> Result<(), String> {
    let stripped = cmd.strip_prefix(BASH_WRAPPER).unwrap_or(cmd);

    if EXTERNAL_COMMANDS.iter().any(|e| stripped.starts_with(e)) {
        return Ok(());
    }

    let rel = INSTALL_PREFIXES
        .iter()
        .find_map(|p| stripped.strip_prefix(p))
        .unwrap_or(stripped);

    let full = repo_root.join(rel);
    if !full.exists() {
        return Err(format!(
            "hook command path not found under repo root: '{stripped}' (looked for {})",
            full.display()
        ));
    }
    Ok(())
}
