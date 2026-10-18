// SPDX-FileCopyrightText: 2026 Igor Santos
// SPDX-License-Identifier: Apache-2.0

//! Guards what ships in the seed (`settings.shared.json`): a seed that
//! pinned a model, carried a non-shippable key, shipped no permissions or
//! named a hook that does not exist would be installed everywhere.

use crate::settings::keys::{shipped_permissions, SHIPPABLE_KEYS};
use crate::HookName;
use clap::ValueEnum;
use serde_json::Value;
use std::path::Path;

/// Name the install location, which maps onto the repo root here.
const INSTALL_PREFIXES: [&str; 2] = ["~/.claude/", "$HOME/.claude/"];

const BASH_WRAPPER: &str = "bash ";

/// Deny rules the shipped permissions always carry.
const REQUIRED_DENY: [&str; 2] = ["Read(**/.env)", "Read(**/.env.*)"];

pub fn check(template_path: &Path, repo_root: &Path) -> Result<String, String> {
    let template_display = template_path.display();

    let template_raw = std::fs::read_to_string(template_path)
        .map_err(|_| format!("template not readable: {template_display}"))?;
    if !repo_root.is_dir() {
        return Err(format!(
            "repo root is not a directory: {}",
            repo_root.display()
        ));
    }

    let template: Value = serde_json::from_str(&template_raw)
        .map_err(|_| format!("template is not valid JSON: {template_display}"))?;

    if shipped_permissions(&template).is_none() {
        return Err(format!(
            ".permissions must be an object with a non-empty allow array in {template_display}"
        ));
    }
    check_conservative_permissions(&template, template_path)?;

    if template.get("model").is_some() {
        return Err(format!(
            ".model must not ship in {template_display} (the harness or user picks the model)"
        ));
    }

    if template.get("skipAutoPermissionPrompt") == Some(&Value::Bool(true)) {
        return Err(format!(
            ".skipAutoPermissionPrompt must not be true in {template_display}"
        ));
    }

    check_shippable_keys(&template, template_path)?;
    check_hook_commands(&template, repo_root)?;

    Ok(format!("check-shared-settings: OK ({template_display})"))
}

/// The floor the README promises for the shipped permissions: no bare `Bash`
/// in `allow`, the `.env` read rules in `deny`, and no `bypassPermissions`
/// mode. A hand edit to the seed that breaks one of these fails here.
fn check_conservative_permissions(template: &Value, template_path: &Path) -> Result<(), String> {
    let display = template_path.display();
    let list = |key: &str| -> Vec<&str> {
        template
            .pointer(&format!("/permissions/{key}"))
            .and_then(Value::as_array)
            .map(|items| items.iter().filter_map(Value::as_str).collect())
            .unwrap_or_default()
    };
    if list("allow").contains(&"Bash") {
        return Err(format!(
            ".permissions.allow must not hold a bare Bash in {display}"
        ));
    }
    let deny = list("deny");
    for rule in REQUIRED_DENY {
        if !deny.contains(&rule) {
            return Err(format!(".permissions.deny must hold {rule} in {display}"));
        }
    }
    if template.pointer("/permissions/defaultMode") == Some(&Value::from("bypassPermissions")) {
        return Err(format!(
            ".permissions.defaultMode must not be bypassPermissions in {display}"
        ));
    }
    Ok(())
}

/// Every template key must be on `SHIPPABLE_KEYS`: the allowlist inversion of
/// the old personal-key denylist.
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

    Ok(())
}

/// Malformed shapes are skipped: this validates commands, and shape is the
/// settings schema's concern.
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

    if let Some(name) = stripped.strip_prefix(crate::init::wire::HOOK_COMMAND_PREFIX) {
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
