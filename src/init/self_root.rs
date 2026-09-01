// SPDX-FileCopyrightText: 2026 Igor Santos
// SPDX-License-Identifier: MIT

//! Resolves `InitPaths::self_root`: `CLAUDE_PLUGIN_ROOT` when Claude Code set
//! it, or the marketplace cache directory a direct terminal run has neither.

use std::path::{Path, PathBuf};

/// An empty or absent `claude_plugin_root_env` falls back to the versioned
/// marketplace cache path under `claude_home`; `None` when neither resolves.
pub fn resolve(claude_plugin_root_env: Option<&str>, claude_home: &Path) -> Option<PathBuf> {
    if let Some(root) = claude_plugin_root_env.filter(|s| !s.is_empty()) {
        return Some(PathBuf::from(root));
    }
    let cached = claude_home
        .join("plugins/cache/pragmatic-engineer/playbook")
        .join(env!("CARGO_PKG_VERSION"));
    cached.is_dir().then_some(cached)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    fn scratch(tag: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "playbook-self-root-{tag}-{}-{:?}",
            std::process::id(),
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

    #[test]
    fn a_cache_dir_for_a_different_version_does_not_match() {
        // Arrange
        let claude_home = scratch("wrong-version");
        let stale_cache = claude_home
            .join("plugins/cache/pragmatic-engineer/playbook")
            .join("0.1.0-not-the-running-version");
        fs::create_dir_all(&stale_cache).expect("cache dir");

        // Act
        let got = resolve(None, &claude_home);

        // Assert
        assert_eq!(got, None);
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
}
