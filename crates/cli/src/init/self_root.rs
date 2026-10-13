// SPDX-FileCopyrightText: 2026 Igor Santos
// SPDX-License-Identifier: Apache-2.0

//! Resolves `InitPaths::self_root`: `CLAUDE_PLUGIN_ROOT` when Claude Code set
//! it, or the marketplace cache directory a direct terminal run has neither.

use std::path::{Path, PathBuf};

/// An empty or absent `claude_plugin_root_env` falls back to the marketplace
/// cache under `claude_home`. The cache directory for the running version
/// wins. When it is missing (a package manager upgraded the binary before the
/// plugin cache caught up), the newest cached version is used instead, since
/// the shipped template and prompt files are still the best source available.
/// `None` when neither resolves.
pub fn resolve(claude_plugin_root_env: Option<&str>, claude_home: &Path) -> Option<PathBuf> {
    if let Some(root) = claude_plugin_root_env.filter(|s| !s.is_empty()) {
        return Some(PathBuf::from(root));
    }
    let cache = claude_home.join("plugins/cache/pragmatic-engineer/playbook");
    let exact = cache.join(env!("CARGO_PKG_VERSION"));
    if exact.is_dir() {
        return Some(exact);
    }
    newest_cached(&cache)
}

const EMBEDDED_SETTINGS: &str = include_str!("../../../../settings.shared.json");
const EMBEDDED_PROMPT: &str = include_str!("../../../../prompts/SYSTEM_PROMPT.md");
const EMBEDDED_BREWFILE: &str = include_str!("../../../../Brewfile");

/// True when `root` holds the two files `init` seeds from. The skills plugin
/// does not ship them (the hooks plugin and this binary do), so a plugin root
/// that lacks them is not a source for `init`.
pub fn has_seed_files(root: &Path) -> bool {
    root.join("settings.shared.json").is_file() && root.join("prompts/SYSTEM_PROMPT.md").is_file()
}

/// `resolve`, then the copies of the settings template and system prompt
/// compiled into this binary. A Homebrew install ships only the binary, and a
/// skills-only plugin install ships neither file, so without a root that holds
/// them nothing else would exist for `init` to seed from.
/// The embedded copies are written under `$HOME/.config/playbook/embedded/`
/// (playbook's own state, never Claude Code's) and that directory is returned.
pub fn resolve_or_embedded(
    claude_plugin_root_env: Option<&str>,
    claude_home: &Path,
    home: &Path,
) -> Option<PathBuf> {
    resolve(claude_plugin_root_env, claude_home)
        .filter(|root| has_seed_files(root))
        .or_else(|| write_embedded(home).ok())
}

/// The root that holds the shipped skills, which `init` hashes to notice later
/// edits. The seed root is the hooks plugin or the embedded copy and has no
/// `skills/`, so fall back to the `playbook` plugin's own cache directory.
pub fn skills_root(self_root: Option<&Path>, claude_home: &Path) -> Option<PathBuf> {
    self_root
        .filter(|root| root.join("skills").is_dir())
        .map(Path::to_path_buf)
        .or_else(|| resolve(None, claude_home).filter(|root| root.join("skills").is_dir()))
}

/// The embedded Brewfile, written under the same embedded directory.
pub fn embedded_brewfile(home: &Path) -> std::io::Result<PathBuf> {
    Ok(write_embedded(home)?.join("Brewfile"))
}

fn write_embedded(home: &Path) -> std::io::Result<PathBuf> {
    let root = crate::common::paths::playbook_root_from(home)
        .join("embedded")
        .join(env!("CARGO_PKG_VERSION"));
    std::fs::create_dir_all(root.join("prompts"))?;
    std::fs::write(root.join("settings.shared.json"), EMBEDDED_SETTINGS)?;
    std::fs::write(root.join("prompts/SYSTEM_PROMPT.md"), EMBEDDED_PROMPT)?;
    std::fs::write(root.join("Brewfile"), EMBEDDED_BREWFILE)?;
    Ok(root)
}

/// The highest version directory directly under `cache`. Names that do not
/// parse as `major.minor.patch` (with an optional pre-release suffix) are
/// ignored, and a pre-release sorts below its release.
fn newest_cached(cache: &Path) -> Option<PathBuf> {
    std::fs::read_dir(cache)
        .ok()?
        .filter_map(Result::ok)
        .filter(|e| e.path().is_dir())
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
    use std::fs;

    fn scratch(tag: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "playbook-self-root-{tag}-{}-{:?}",
            crate::testing::run_id(),
            std::thread::current().id()
        ));
        fs::create_dir_all(&dir).expect("scratch dir");
        dir
    }

    fn cache_dir_for(claude_home: &Path) -> PathBuf {
        claude_home
            .join("plugins/cache/pragmatic-engineer/playbook")
            .join(env!("CARGO_PKG_VERSION"))
    }

    #[test]
    fn explicit_env_var_wins_even_when_the_cache_also_matches() {
        // Arrange
        let claude_home = scratch("env-wins");
        fs::create_dir_all(cache_dir_for(&claude_home)).expect("cache dir");

        // Act
        let got = resolve(Some("/explicit/plugin/root"), &claude_home);

        // Assert
        assert_eq!(got, Some(PathBuf::from("/explicit/plugin/root")));
    }

    #[test]
    fn empty_env_var_falls_back_to_the_versioned_cache_dir() {
        // Arrange
        let claude_home = scratch("empty-env-falls-back");
        let cache_dir = cache_dir_for(&claude_home);
        fs::create_dir_all(&cache_dir).expect("cache dir");

        // Act
        let got = resolve(Some(""), &claude_home);

        // Assert
        assert_eq!(got, Some(cache_dir));
    }

    #[test]
    fn absent_env_var_falls_back_to_the_versioned_cache_dir() {
        // Arrange
        let claude_home = scratch("absent-env-falls-back");
        let cache_dir = cache_dir_for(&claude_home);
        fs::create_dir_all(&cache_dir).expect("cache dir");

        // Act
        let got = resolve(None, &claude_home);

        // Assert
        assert_eq!(got, Some(cache_dir));
    }

    #[test]
    fn no_env_var_and_no_cache_dir_resolves_to_none() {
        // Arrange
        let claude_home = scratch("nothing-resolves");

        // Act
        let got = resolve(None, &claude_home);

        // Assert
        assert_eq!(got, None);
    }

    fn cache_root(claude_home: &Path) -> PathBuf {
        claude_home.join("plugins/cache/pragmatic-engineer/playbook")
    }

    #[test]
    fn a_different_cached_version_is_used_when_the_running_one_is_missing() {
        // Arrange
        let claude_home = scratch("other-version");
        let older = cache_root(&claude_home).join("0.15.0");
        fs::create_dir_all(&older).expect("cache dir");

        // Act
        let got = resolve(None, &claude_home);

        // Assert
        assert_eq!(got, Some(older));
    }

    #[test]
    fn the_newest_cached_version_wins_by_number_not_by_text() {
        // Arrange
        let claude_home = scratch("newest-wins");
        for v in ["0.9.0", "0.15.0", "0.2.0", "not-a-version"] {
            fs::create_dir_all(cache_root(&claude_home).join(v)).expect("cache dir");
        }

        // Act
        let got = resolve(None, &claude_home);

        // Assert
        assert_eq!(got, Some(cache_root(&claude_home).join("0.15.0")));
    }

    #[test]
    fn a_release_beats_its_own_prerelease() {
        // Arrange
        let claude_home = scratch("release-over-pre");
        for v in ["1.0.0-rc.1", "1.0.0"] {
            fs::create_dir_all(cache_root(&claude_home).join(v)).expect("cache dir");
        }

        // Act
        let got = resolve(None, &claude_home);

        // Assert
        assert_eq!(got, Some(cache_root(&claude_home).join("1.0.0")));
    }

    #[test]
    fn the_running_version_wins_over_a_newer_cached_one() {
        // Arrange
        let claude_home = scratch("exact-over-newer");
        let exact = cache_dir_for(&claude_home);
        fs::create_dir_all(&exact).expect("cache dir");
        fs::create_dir_all(cache_root(&claude_home).join("999.0.0")).expect("cache dir");

        // Act
        let got = resolve(None, &claude_home);

        // Assert
        assert_eq!(got, Some(exact));
    }

    #[test]
    fn a_file_at_the_cache_path_instead_of_a_directory_does_not_match() {
        // Arrange
        let claude_home = scratch("file-not-dir");
        let cache_dir = cache_dir_for(&claude_home);
        fs::create_dir_all(cache_dir.parent().unwrap()).expect("parent dir");
        fs::write(&cache_dir, b"not a directory").expect("write file");

        // Act
        let got = resolve(None, &claude_home);

        // Assert
        assert_eq!(got, None);
    }

    #[test]
    fn with_no_plugin_the_embedded_files_are_written_and_returned() {
        // Arrange
        let home = scratch("embedded");

        // Act
        let got = resolve_or_embedded(None, &home.join(".claude"), &home).expect("embedded root");

        // Assert
        assert!(got.starts_with(home.join(".config/playbook/embedded")));
        assert_eq!(
            fs::read_to_string(got.join("settings.shared.json")).unwrap(),
            EMBEDDED_SETTINGS
        );
        assert!(got.join("prompts/SYSTEM_PROMPT.md").is_file());
        assert!(got.join("Brewfile").is_file());
    }

    #[test]
    fn a_found_plugin_root_is_preferred_over_the_embedded_files() {
        // Arrange
        let home = scratch("embedded-not-needed");
        let root = home.join("plugin-root");
        fs::create_dir_all(root.join("prompts")).expect("prompts dir");
        fs::write(root.join("settings.shared.json"), "{}").expect("settings");
        fs::write(root.join("prompts/SYSTEM_PROMPT.md"), "p").expect("prompt");

        // Act
        let got = resolve_or_embedded(root.to_str(), &home.join(".claude"), &home);

        // Assert
        assert_eq!(got, Some(root));
        assert!(!home.join(".config/playbook/embedded").exists());
    }

    #[test]
    fn the_skills_root_is_the_cached_playbook_plugin_when_the_seed_root_has_no_skills() {
        // Arrange
        let claude_home = scratch("skills-root");
        let cache = cache_dir_for(&claude_home);
        fs::create_dir_all(cache.join("skills")).expect("skills dir");
        let seed_root = scratch("skills-root-seed");

        // Act
        let got = skills_root(Some(&seed_root), &claude_home);

        // Assert
        assert_eq!(got, Some(cache));
        assert_eq!(skills_root(None, &scratch("skills-root-none")), None);
    }

    #[test]
    fn a_plugin_root_without_the_seed_files_falls_back_to_the_embedded_files() {
        // Arrange: the skills plugin ships neither file.
        let home = scratch("skills-only-root");
        let root = home.join("skills-plugin");
        fs::create_dir_all(root.join("skills")).expect("skills dir");

        // Act
        let got =
            resolve_or_embedded(root.to_str(), &home.join(".claude"), &home).expect("embedded");

        // Assert
        assert!(got.starts_with(home.join(".config/playbook/embedded")));
        assert!(has_seed_files(&got));
    }
}
