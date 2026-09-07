// SPDX-FileCopyrightText: 2026 Igor Santos
// SPDX-License-Identifier: MIT

//! Port of `shell/check-shared-settings.py`, guarding what ships in
//! `settings.shared.json`: a seed that pinned a model, carried a
//! non-shippable key, or named a hook that does not exist would be
//! installed verbatim everywhere.
//!
//! Message wording and exit status match the python exactly, because
//! `tests/settings_check.rs` diffs the two implementations.

use crate::settings::keys::{SHIPPABLE_ENV, SHIPPABLE_KEYS};
use crate::HookName;
use clap::ValueEnum;
use serde_json::Value;
use std::path::Path;

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

    check_shippable_keys(&template, template_path)?;
    check_hook_commands(&template, repo_root)?;

    Ok(format!("check-shared-settings: OK ({template_display})"))
}

/// Every template key must be on `SHIPPABLE_KEYS`, and every `env` key on
/// `SHIPPABLE_ENV`: the allowlist inversion of the old personal-key denylist.
fn check_shippable_keys(template: &Value, template_path: &Path) -> Result<(), String> {
    let Some(obj) = template.as_object() else {
        return Ok(());
    };

    let offending: Vec<&str> = obj
        .keys()
        .map(String::as_str)
        .filter(|k| !SHIPPABLE_KEYS.contains(k))
        .collect();
    if !offending.is_empty() {
        return Err(format!(
            "key(s) not in the shippable allowlist in {}: {}",
            template_path.display(),
            offending.join(", ")
        ));
    }

    if let Some(env) = obj.get("env").and_then(Value::as_object) {
        let offending_env: Vec<&str> = env
            .keys()
            .map(String::as_str)
            .filter(|k| !SHIPPABLE_ENV.contains(k))
            .collect();
        if !offending_env.is_empty() {
            return Err(format!(
                "env key(s) not in the shippable allowlist in {}: {}",
                template_path.display(),
                offending_env.join(", ")
            ));
        }
    }
    Ok(())
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

/// A `playbook hook <name>` command is checked against the real `HookName`
/// set instead of being bypassed: it was the only external-command shape the
/// shipped product ever writes (`src/init/wire.rs` never writes any other
/// `playbook ...` form), so trusting it unconditionally gave no real
/// protection against a typo'd or dangling hook name.
fn check_one_command(cmd: &str, repo_root: &Path) -> Result<(), String> {
    let stripped = cmd.strip_prefix(BASH_WRAPPER).unwrap_or(cmd);

    if let Some(name) = stripped.strip_prefix("playbook hook ") {
        return HookName::from_str(name, true)
            .map(|_| ())
            .map_err(|_| format!("unknown hook name in playbook hook command: '{name}'"));
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
