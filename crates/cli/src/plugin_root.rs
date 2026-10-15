// SPDX-FileCopyrightText: 2026 Igor Santos
// SPDX-License-Identifier: Apache-2.0

//! The one place that finds the installed `playbook` plugin directory
//! (ADR 0034). The order is the `installPath` Claude Code recorded, then
//! `CLAUDE_PLUGIN_ROOT`, then the newest usable cache directory. The binary
//! version plays no part, because the plugin has its own version line.

use std::path::{Path, PathBuf};

/// The command a user runs when no plugin directory is found.
pub const INSTALL_HINT: &str = "claude plugin install playbook@pragmatic-engineer";

/// An explicit override, for a dev checkout. It wins over every other source.
pub const OVERRIDE_ENV: &str = "PLAYBOOK_PLUGIN_DIR";

const CACHE_DIR: &str = "plugins/cache/pragmatic-engineer/playbook";
const ORPHAN_MARKER: &str = ".orphaned_at";
const PLUGIN_NAME: &str = "playbook";

/// Where a resolved directory came from.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Source {
    Override,
    InstallPath,
    Env,
    Cache,
}

impl Source {
    pub fn label(self) -> &'static str {
        match self {
            Source::Override => OVERRIDE_ENV,
            Source::InstallPath => "installPath in installed_plugins.json",
            Source::Env => "CLAUDE_PLUGIN_ROOT",
            Source::Cache => "the plugin cache",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Resolved {
    pub root: PathBuf,
    pub source: Source,
}

/// Nothing resolved. Each entry names one place tried and why it was skipped.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NotFound {
    pub tried: Vec<String>,
}

impl std::fmt::Display for NotFound {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "playbook plugin not found (tried {}); run `{INSTALL_HINT}`",
            self.tried.join(", ")
        )
    }
}

/// What the resolver reads, passed in so tests never touch the real home.
pub struct Inputs<'a> {
    pub claude_home: &'a Path,
    /// `PLAYBOOK_PLUGIN_DIR`.
    pub override_env: Option<&'a str>,
    /// `CLAUDE_PLUGIN_ROOT`.
    pub env: Option<&'a str>,
    pub cwd: &'a Path,
}

/// Finds the plugin directory. See the module doc for the order.
pub fn resolve(inputs: &Inputs) -> Result<Resolved, NotFound> {
    let mut tried = Vec::new();

    if let Some(root) = inputs
        .override_env
        .filter(|s| !s.is_empty())
        .map(PathBuf::from)
    {
        if is_plugin_dir(&root) {
            return Ok(Resolved {
                root,
                source: Source::Override,
            });
        }
        tried.push(format!("{OVERRIDE_ENV} {} (not usable)", root.display()));
    }

    let listed = install_paths(inputs.claude_home, inputs.cwd);
    match listed.iter().find(|p| is_plugin_dir(p)) {
        Some(root) => {
            return Ok(Resolved {
                root: root.clone(),
                source: Source::InstallPath,
            })
        }
        None if listed.is_empty() => tried.push("installPath (not listed)".to_string()),
        None => tried.push(format!("installPath {} (not usable)", listed[0].display())),
    }

    match inputs.env.filter(|s| !s.is_empty()).map(PathBuf::from) {
        Some(root) if is_plugin_dir(&root) => {
            return Ok(Resolved {
                root,
                source: Source::Env,
            })
        }
        Some(root) => tried.push(format!(
            "CLAUDE_PLUGIN_ROOT {} (not usable)",
            root.display()
        )),
        None => tried.push("CLAUDE_PLUGIN_ROOT (unset)".to_string()),
    }

    let cache = inputs.claude_home.join(CACHE_DIR);
    if let Some(root) = newest_cached(&cache) {
        return Ok(Resolved {
            root,
            source: Source::Cache,
        });
    }
    tried.push(format!("{} (no usable version)", cache.display()));
    Err(NotFound { tried })
}

/// `resolve` for this process: the real home, `CLAUDE_PLUGIN_ROOT` and the
/// current directory.
pub fn resolve_here() -> Result<Resolved, NotFound> {
    let claude_home = crate::common::home_dir().join(".claude");
    let env = std::env::var("CLAUDE_PLUGIN_ROOT").ok();
    let override_env = std::env::var(OVERRIDE_ENV).ok();
    let cwd = std::env::current_dir().unwrap_or_default();
    resolve(&Inputs {
        claude_home: &claude_home,
        override_env: override_env.as_deref(),
        env: env.as_deref(),
        cwd: &cwd,
    })
}

/// The resolved directory, or `None`. For callers that treat a missing plugin
/// as "no plugin files", never as an error.
pub fn find(env: Option<&str>, claude_home: &Path) -> Option<PathBuf> {
    let cwd = std::env::current_dir().unwrap_or_default();
    let override_env = std::env::var(OVERRIDE_ENV).ok();
    resolve(&Inputs {
        claude_home,
        override_env: override_env.as_deref(),
        env,
        cwd: &cwd,
    })
    .ok()
    .map(|r| r.root)
}

/// A directory with plugin components, not marked orphaned by Claude Code,
/// whose manifest (when present) names `playbook`. The manifest check keeps
/// out the `playbook-variants` session plugin.
pub fn is_plugin_dir(dir: &Path) -> bool {
    dir.is_dir()
        && ["agents", "commands", "skills"]
            .iter()
            .any(|d| dir.join(d).is_dir())
        && is_ours(dir)
}

/// Not marked orphaned by Claude Code, and a manifest (when present) that
/// names `playbook`. Rejects the old `playbook-hooks` plugin and the
/// `playbook-variants` session plugin.
pub fn is_ours(dir: &Path) -> bool {
    !dir.join(ORPHAN_MARKER).exists() && manifest_name(dir).is_none_or(|name| name == PLUGIN_NAME)
}

/// The `version` in the directory's `.claude-plugin/plugin.json`.
pub fn plugin_version(root: &Path) -> Option<String> {
    manifest_field(root, "version")
}

fn manifest_name(dir: &Path) -> Option<String> {
    manifest_field(dir, "name")
}

fn manifest_field(dir: &Path, key: &str) -> Option<String> {
    let text = std::fs::read_to_string(dir.join(".claude-plugin/plugin.json")).ok()?;
    let value: serde_json::Value = serde_json::from_str(&text).ok()?;
    value.get(key)?.as_str().map(str::to_string)
}

/// The `installPath` values Claude Code lists for the plugin, best first: a
/// project entry whose `projectPath` holds `cwd` (deepest first), then user
/// entries, then the rest.
fn install_paths(claude_home: &Path, cwd: &Path) -> Vec<PathBuf> {
    let Ok(text) = std::fs::read_to_string(claude_home.join("plugins/installed_plugins.json"))
    else {
        return Vec::new();
    };
    let Ok(value) = serde_json::from_str::<serde_json::Value>(&text) else {
        return Vec::new();
    };
    let key = crate::common::plugin_state::PLUGIN_KEY;
    let entry = value
        .get("plugins")
        .and_then(|p| p.get(key))
        .or_else(|| value.get(key));
    let entries: Vec<&serde_json::Value> = match entry {
        Some(serde_json::Value::Array(list)) => list.iter().collect(),
        Some(obj @ serde_json::Value::Object(_)) => vec![obj],
        _ => return Vec::new(),
    };
    let mut ranked: Vec<(usize, PathBuf)> = entries
        .into_iter()
        .filter_map(|e| {
            let path = PathBuf::from(e.get("installPath")?.as_str()?);
            let rank = match e.get("projectPath").and_then(|p| p.as_str()) {
                Some(project) if cwd.starts_with(project) => {
                    usize::MAX - Path::new(project).components().count()
                }
                Some(_) => usize::MAX - 1,
                None if e.get("scope").and_then(|s| s.as_str()) == Some("user") => usize::MAX - 2,
                None => usize::MAX - 1,
            };
            Some((rank, path))
        })
        .collect();
    // Lower rank first: deepest matching project, then user, then the rest.
    ranked.sort_by_key(|(rank, _)| *rank);
    ranked.into_iter().map(|(_, p)| p).collect()
}

/// The highest usable version directory directly under `cache`. Names that do
/// not parse as `major.minor.patch` (with an optional pre-release suffix) are
/// skipped, and a pre-release sorts below its release.
fn newest_cached(cache: &Path) -> Option<PathBuf> {
    std::fs::read_dir(cache)
        .ok()?
        .filter_map(Result::ok)
        .filter(|e| is_plugin_dir(&e.path()))
        .filter_map(|e| {
            let name = e.file_name().into_string().ok()?;
            Some((version_key(&name)?, e.path()))
        })
        .max_by(|a, b| a.0.cmp(&b.0))
        .map(|(_, path)| path)
}

fn version_key(name: &str) -> Option<(u64, u64, u64, bool)> {
    let (core, pre) = match name.split_once('-') {
        Some((core, _)) => (core, true),
        None => (name, false),
    };
    let mut parts = core.split('.').map(|p| p.parse::<u64>().ok());
    let key = (parts.next()??, parts.next()??, parts.next()??, !pre);
    parts.next().is_none().then_some(key)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::common::test_support::scratch_dir;
    use std::fs;

    fn plugin_at(dir: &Path, name: &str, version: &str) -> PathBuf {
        fs::create_dir_all(dir.join("skills")).unwrap();
        fs::create_dir_all(dir.join(".claude-plugin")).unwrap();
        fs::write(
            dir.join(".claude-plugin/plugin.json"),
            format!(r#"{{"name":"{name}","version":"{version}"}}"#),
        )
        .unwrap();
        dir.to_path_buf()
    }

    fn cached(claude_home: &Path, version: &str) -> PathBuf {
        plugin_at(
            &claude_home.join(CACHE_DIR).join(version),
            "playbook",
            version,
        )
    }

    fn list_install(claude_home: &Path, entries: &str) {
        fs::create_dir_all(claude_home.join("plugins")).unwrap();
        fs::write(
            claude_home.join("plugins/installed_plugins.json"),
            format!(r#"{{"version":2,"plugins":{{"playbook@pragmatic-engineer":[{entries}]}}}}"#),
        )
        .unwrap();
    }

    fn run(claude_home: &Path, env: Option<&str>) -> Result<Resolved, NotFound> {
        resolve(&Inputs {
            claude_home,
            override_env: None,
            env,
            cwd: Path::new("/nowhere"),
        })
    }

    #[test]
    fn the_override_wins_over_the_install_path() {
        // Arrange
        let home = scratch_dir("plugin-root-override");
        let active = cached(&home, "0.28.0");
        list_install(
            &home,
            &format!(r#"{{"scope":"user","installPath":"{}"}}"#, active.display()),
        );
        let checkout = plugin_at(&home.join("checkout"), "playbook", "0.31.0");

        // Act
        let got = resolve(&Inputs {
            claude_home: &home,
            override_env: checkout.to_str(),
            env: None,
            cwd: Path::new("/nowhere"),
        })
        .unwrap();

        // Assert
        assert_eq!(got.root, checkout);
        assert_eq!(got.source, Source::Override);
    }

    #[test]
    fn a_version_1_install_entry_is_read_too() {
        // Arrange
        let home = scratch_dir("plugin-root-v1");
        let active = cached(&home, "0.25.0");
        cached(&home, "0.30.0");
        fs::write(
            home.join("plugins/installed_plugins.json"),
            format!(
                r#"{{"version":1,"plugins":{{"playbook@pragmatic-engineer":{{"version":"0.25.0","installPath":"{}"}}}}}}"#,
                active.display()
            ),
        )
        .unwrap();

        // Act and assert
        assert_eq!(run(&home, None).unwrap().root, active);
    }

    #[test]
    fn the_install_path_wins_over_the_env_var_and_a_newer_cache_dir() {
        // Arrange
        let home = scratch_dir("plugin-root-install");
        let active = cached(&home, "0.28.0");
        cached(&home, "0.30.0");
        let env_dir = plugin_at(&home.join("checkout"), "playbook", "0.31.0");
        list_install(
            &home,
            &format!(r#"{{"scope":"user","installPath":"{}"}}"#, active.display()),
        );

        // Act
        let got = run(&home, env_dir.to_str()).unwrap();

        // Assert
        assert_eq!(got.root, active);
        assert_eq!(got.source, Source::InstallPath);
    }

    #[test]
    fn a_project_entry_for_the_current_directory_beats_the_user_entry() {
        // Arrange
        let home = scratch_dir("plugin-root-project");
        let user = cached(&home, "0.28.0");
        let project = cached(&home, "0.29.0");
        list_install(
            &home,
            &format!(
                r#"{{"scope":"user","installPath":"{}"}},{{"scope":"project","projectPath":"/work/repo","installPath":"{}"}}"#,
                user.display(),
                project.display()
            ),
        );

        // Act
        let got = resolve(&Inputs {
            claude_home: &home,
            override_env: None,
            env: None,
            cwd: Path::new("/work/repo/src"),
        })
        .unwrap();

        // Assert
        assert_eq!(got.root, project);
    }

    #[test]
    fn a_missing_install_path_falls_back_to_the_env_var() {
        // Arrange
        let home = scratch_dir("plugin-root-stale-install");
        list_install(&home, r#"{"scope":"user","installPath":"/gone/0.25.0"}"#);
        let env_dir = plugin_at(&home.join("checkout"), "playbook", "0.31.0");

        // Act
        let got = run(&home, env_dir.to_str()).unwrap();

        // Assert
        assert_eq!(got.root, env_dir);
        assert_eq!(got.source, Source::Env);
    }

    #[test]
    fn an_env_var_without_components_or_naming_another_plugin_is_skipped() {
        // Arrange
        let home = scratch_dir("plugin-root-env-skip");
        let empty = home.join("hooks-only");
        fs::create_dir_all(&empty).unwrap();
        let variants = plugin_at(&home.join("variants"), "playbook-variants", "0.0.0");
        let cache = cached(&home, "0.30.0");

        // Act and assert
        assert_eq!(run(&home, empty.to_str()).unwrap().root, cache);
        assert_eq!(run(&home, variants.to_str()).unwrap().root, cache);
    }

    #[test]
    fn the_cache_skips_orphaned_dirs_and_picks_the_newest_by_number() {
        // Arrange
        let home = scratch_dir("plugin-root-cache");
        cached(&home, "0.9.0");
        let want = cached(&home, "0.15.0");
        let orphan = cached(&home, "0.40.0");
        fs::write(orphan.join(ORPHAN_MARKER), "1").unwrap();
        fs::create_dir_all(home.join(CACHE_DIR).join("not-a-version/skills")).unwrap();

        // Act
        let got = run(&home, None).unwrap();

        // Assert
        assert_eq!(got.root, want);
        assert_eq!(got.source, Source::Cache);
    }

    #[test]
    fn the_binary_version_dir_gets_no_special_treatment() {
        // Arrange
        let home = scratch_dir("plugin-root-no-binary-version");
        cached(&home, env!("CARGO_PKG_VERSION"));
        let newer = cached(&home, "999.0.0");

        // Act and assert
        assert_eq!(run(&home, None).unwrap().root, newer);
    }

    #[test]
    fn a_release_beats_its_own_prerelease() {
        // Arrange
        let home = scratch_dir("plugin-root-pre");
        cached(&home, "1.0.0-rc.1");
        let release = cached(&home, "1.0.0");

        // Act and assert
        assert_eq!(run(&home, None).unwrap().root, release);
    }

    #[test]
    fn nothing_found_names_every_place_tried_in_one_line() {
        // Arrange
        let home = scratch_dir("plugin-root-none");

        // Act
        let err = run(&home, None).unwrap_err().to_string();

        // Assert
        assert!(!err.contains('\n'), "{err}");
        for part in [
            "installPath",
            "CLAUDE_PLUGIN_ROOT (unset)",
            CACHE_DIR,
            INSTALL_HINT,
        ] {
            assert!(err.contains(part), "{part} missing: {err}");
        }
    }

    #[test]
    fn a_malformed_install_list_is_treated_as_not_listed() {
        // Arrange
        let home = scratch_dir("plugin-root-bad-json");
        fs::create_dir_all(home.join("plugins")).unwrap();
        fs::write(home.join("plugins/installed_plugins.json"), "{not json").unwrap();
        let cache = cached(&home, "0.30.0");

        // Act and assert
        assert_eq!(run(&home, None).unwrap().root, cache);
    }

    #[test]
    fn the_plugin_version_comes_from_the_manifest() {
        let home = scratch_dir("plugin-root-version");
        let root = plugin_at(&home.join("p"), "playbook", "0.30.1");
        assert_eq!(plugin_version(&root).as_deref(), Some("0.30.1"));
        assert_eq!(plugin_version(&home.join("missing")), None);
    }
}
