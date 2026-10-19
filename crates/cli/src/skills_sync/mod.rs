// SPDX-FileCopyrightText: 2026 Igor Santos
// SPDX-License-Identifier: Apache-2.0

//! Copies the playbook skills and commands into other agents' skill folders.
//! See `docs/internals/15-codex-skills.md`.

use std::path::Path;

pub mod plan;

/// An agent that reads skills from a shared folder.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Agent {
    Codex,
}

/// The agents found from explicit inputs, so tests never read the real
/// environment. Codex counts when `$home/.codex` is a folder or a `codex`
/// program is on `path`.
pub fn detect(home: &Path, path: &str) -> Vec<Agent> {
    let mut found = Vec::new();
    if home.join(".codex").is_dir() || on_path("codex", path) {
        found.push(Agent::Codex);
    }
    found
}

fn on_path(program: &str, path: &str) -> bool {
    std::env::split_paths(path).any(|dir| dir.join(program).is_file())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::common::test_support::scratch_dir;
    use std::fs;

    fn empty_home(tag: &str) -> std::path::PathBuf {
        let home = scratch_dir(tag);
        fs::create_dir_all(&home).unwrap();
        home
    }

    #[test]
    fn a_codex_folder_alone_finds_codex() {
        // Arrange
        let home = empty_home("detect-folder");
        fs::create_dir(home.join(".codex")).unwrap();

        // Act
        let found = detect(&home, "");

        // Assert
        assert_eq!(found, vec![Agent::Codex]);
    }

    #[test]
    fn a_codex_program_on_the_given_path_alone_finds_codex() {
        // Arrange
        let home = empty_home("detect-path-home");
        let bin = empty_home("detect-path-bin");
        fs::write(bin.join("codex"), "").unwrap();

        // Act
        let found = detect(&home, bin.to_str().unwrap());

        // Assert
        assert_eq!(found, vec![Agent::Codex]);
    }

    #[test]
    fn neither_signal_finds_nothing() {
        // Arrange
        let home = empty_home("detect-none-home");
        let bin = empty_home("detect-none-bin");

        // Act
        let found = detect(&home, bin.to_str().unwrap());

        // Assert
        assert!(found.is_empty());
    }
}
