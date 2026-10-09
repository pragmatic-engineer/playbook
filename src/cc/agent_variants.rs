// SPDX-FileCopyrightText: 2026 Igor Santos
// SPDX-License-Identifier: Apache-2.0

//! The effort-tier variants of the base agents the launcher gives a session
//! (ADR-0017). They are rendered for this session only and written into a
//! throwaway plugin directory in the temp dir, passed with `--plugin-dir` and
//! removed when the session ends. The directory has no size limit, unlike one
//! `--agents` argument (Linux caps a single argument at 128 KiB and the auto
//! set was 100 KB). If the directory cannot be written, the launcher falls
//! back to `--agents`. Nothing under `~/.claude` or the repo is touched. A
//! session started without the launcher holds the base agents only.

use crate::agents::variants::{self, Mode};
use crate::config;
use crate::effort::{self, component};
use serde_json::{Map, Value};
use std::fs;
use std::io;
use std::path::{Path, PathBuf};

/// Name of the throwaway plugin. Its agents are spawned as `playbook-variants:<variant>`.
pub const PLUGIN_NAME: &str = "playbook-variants";

/// What the launcher adds for variants.
#[derive(Debug)]
pub struct Session {
    /// The flags to pass: `--plugin-dir <dir>`, or `--agents <json>` on fallback.
    pub args: Vec<String>,
    /// The variant names, comma separated, for `PLAYBOOK_AGENT_VARIANTS`.
    pub names: String,
    /// The throwaway plugin directory to remove after the session, when used.
    pub plugin_dir: Option<PathBuf>,
}

/// The variants for this session, or `None` when the user passed their own
/// `--agents`, the key `agents.variants` is `off`, no plugin root is found, or
/// no variant applies. `temp` is where the throwaway plugin directory goes.
pub fn session(
    args: &[String],
    home: &Path,
    claude_home: &Path,
    cwd: &Path,
    plugin_root_env: Option<&str>,
    temp: &Path,
) -> Option<Session> {
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
    let agents_dir = root.join("agents");
    let map = variants::session_agents(&agents_dir, mode, &ceiling_of);
    if map.is_empty() {
        return None;
    }
    if let Ok(dir) = write_plugin(&map, temp) {
        let names = map.keys().cloned().collect::<Vec<_>>().join(",");
        return Some(Session {
            args: vec!["--plugin-dir".into(), dir.to_string_lossy().into_owned()],
            names,
            plugin_dir: Some(dir),
        });
    }
    let (json, names) = variants::session_json(&agents_dir, mode, &ceiling_of)?;
    Some(Session {
        args: vec!["--agents".into(), json],
        names: names.join(","),
        plugin_dir: None,
    })
}

/// One agent file: frontmatter from the definition, then the prompt.
fn agent_file(name: &str, def: &Value) -> String {
    let text = |key: &str| def[key].as_str().unwrap_or_default();
    let mut out = format!(
        "---\nname: {name}\ndescription: {}\n",
        Value::from(text("description"))
    );
    if let Some(tools) = def["tools"].as_array() {
        let list: Vec<&str> = tools.iter().filter_map(Value::as_str).collect();
        out.push_str(&format!("tools: {}\n", list.join(", ")));
    }
    for key in ["model", "effort"] {
        if !text(key).is_empty() {
            out.push_str(&format!("{key}: {}\n", text(key)));
        }
    }
    out.push_str("---\n\n");
    out.push_str(text("prompt"));
    out
}

/// Write the variants as a plugin under `temp` and return its directory. The
/// name carries the process id so two sessions never share one.
pub fn write_plugin(map: &Map<String, Value>, temp: &Path) -> io::Result<PathBuf> {
    let dir = temp.join(format!("{PLUGIN_NAME}-{}", std::process::id()));
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(dir.join(".claude-plugin"))?;
    fs::create_dir_all(dir.join("agents"))?;
    fs::write(
        dir.join(".claude-plugin/plugin.json"),
        format!(
            "{{\"name\":\"{PLUGIN_NAME}\",\"version\":\"0.0.0\",\"description\":\"Effort variants for one session\"}}\n"
        ),
    )?;
    for (name, def) in map {
        fs::write(
            dir.join("agents").join(format!("{name}.md")),
            agent_file(name, def),
        )?;
    }
    Ok(dir)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::common::test_support::scratch_dir;
    use serde_json::json;

    #[test]
    fn the_session_plugin_has_a_manifest_and_one_valid_file_per_variant() {
        let tmp = scratch_dir("variants-plugin");
        fs::create_dir_all(&tmp).unwrap();
        let mut map = Map::new();
        map.insert(
            "reviewer-low".into(),
            json!({"description": "reviewer at low effort: \"quoted\".", "prompt": "Body.\n",
                   "tools": ["Read", "Grep"], "model": "sonnet", "effort": "low"}),
        );
        let dir = write_plugin(&map, &tmp).unwrap();
        let manifest = fs::read_to_string(dir.join(".claude-plugin/plugin.json")).unwrap();
        assert!(
            manifest.contains("\"name\":\"playbook-variants\""),
            "{manifest}"
        );
        let text = fs::read_to_string(dir.join("agents/reviewer-low.md")).unwrap();
        assert!(text.starts_with("---\nname: reviewer-low\n"), "{text}");
        assert!(
            text.contains("description: \"reviewer at low effort: \\\"quoted\\\".\"\n"),
            "{text}"
        );
        assert!(
            text.contains("tools: Read, Grep\nmodel: sonnet\neffort: low\n---\n\nBody.\n"),
            "{text}"
        );
        assert!(dir.starts_with(&tmp));
    }

    #[test]
    fn a_second_write_replaces_the_first() {
        let tmp = scratch_dir("variants-plugin-twice");
        fs::create_dir_all(&tmp).unwrap();
        let mut map = Map::new();
        map.insert("a-low".into(), json!({"description": "d", "prompt": "p"}));
        write_plugin(&map, &tmp).unwrap();
        map.clear();
        map.insert("b-low".into(), json!({"description": "d", "prompt": "p"}));
        let dir = write_plugin(&map, &tmp).unwrap();
        assert!(!dir.join("agents/a-low.md").exists());
        assert!(dir.join("agents/b-low.md").exists());
    }
}
