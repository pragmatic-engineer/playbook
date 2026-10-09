// SPDX-FileCopyrightText: 2026 Igor Santos
// SPDX-License-Identifier: Apache-2.0

//! The `--agents` flag the launcher passes: effort-tier variants of the base
//! agents, rendered for this session only (ADR-0017). Nothing is written to
//! disk and no plugin file changes. A session started without the launcher
//! holds the base agents only.

use crate::agents::variants::{self, Mode};
use crate::config;
use crate::effort::{self, component};
use serde_json::Value;
use std::path::Path;

/// `["--agents", json]` and the comma separated variant names, or `None` when
/// the user passed their own `--agents`, the key `agents.variants` is `off`,
/// no plugin root is found, or no variant applies.
pub fn flags(
    args: &[String],
    home: &Path,
    claude_home: &Path,
    cwd: &Path,
    plugin_root_env: Option<&str>,
) -> Option<([String; 2], String)> {
    if args
        .iter()
        .any(|a| a == "--agents" || a.starts_with("--agents="))
    {
        return None;
    }
    let mode = match config::resolve_valid("agents.variants", home, None) {
        Ok((Value::String(v), _, _)) => Mode::parse(&v).unwrap_or(Mode::Auto),
        _ => Mode::Auto,
    };
    if mode == Mode::Off {
        return None;
    }
    let root = crate::init::self_root::resolve(plugin_root_env, claude_home)?;
    let claude = effort::claude_cap(claude_home, cwd);
    let ceiling_of = |name: &str| {
        component::resolve_with(
            component::Kind::Agent,
            name,
            home,
            claude.as_deref(),
            Some(&root),
            &[],
        )
        .ceiling
    };
    let (json, names) = variants::session_json(&root.join("agents"), mode, &ceiling_of)?;
    Some((["--agents".to_string(), json], names.join(",")))
}
