// SPDX-FileCopyrightText: 2026 Igor Santos
// SPDX-License-Identifier: Apache-2.0

//! Copies the playbook skills and commands into other agents' skill folders.
//! See `docs/internals/15-codex-skills.md`.

use std::collections::BTreeSet;
use std::fs;
use std::path::{Path, PathBuf};

pub mod apply;
pub mod plan;
pub mod render;

/// An agent that reads skills from a shared folder.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Agent {
    Codex,
}

impl Agent {
    /// The name used in ownership record keys.
    pub fn key(self) -> &'static str {
        match self {
            Agent::Codex => "codex",
        }
    }
}

const SKILL_FILE: &str = "SKILL.md";

/// Renders every planned target from the plugin folder into the files to
/// write: markdown gets its references rewritten, `SKILL.md` and commands
/// are rendered and split when large, and other files are copied as they are.
pub fn render_targets(
    plugin: &Path,
    targets: &[plan::Target],
) -> Result<Vec<apply::RenderedFolder>, String> {
    let known: BTreeSet<String> = targets
        .iter()
        .filter_map(|t| t.name.strip_prefix(plan::NAME_PREFIX))
        .map(str::to_string)
        .collect();
    targets
        .iter()
        .map(|target| render_target(plugin, target, &known))
        .collect()
}

fn render_target(
    plugin: &Path,
    target: &plan::Target,
    known: &BTreeSet<String>,
) -> Result<apply::RenderedFolder, String> {
    let mut files: Vec<(PathBuf, Vec<u8>)> = Vec::new();
    for (rel, source) in &target.files {
        let path = plugin.join(rel);
        let bytes = fs::read(&path).map_err(|e| format!("cannot read {}: {e}", path.display()))?;
        let is_md = rel.extension().and_then(|e| e.to_str()) == Some("md");
        match source {
            plan::Source::Command => {
                let text = utf8(&path, bytes)?;
                let md = render::render_command(&text, &target.name, known)?;
                push_split(&mut files, PathBuf::from(SKILL_FILE), &md);
            }
            plan::Source::SkillFile => {
                // Skill files sit under skills/<name>/, the folder keeps the rest.
                let inner: PathBuf = rel.components().skip(2).collect();
                if inner == Path::new(SKILL_FILE) {
                    let md = render::render_skill_md(&utf8(&path, bytes)?, &target.name, known)?;
                    push_split(&mut files, inner, &md);
                } else if is_md {
                    let text = render::rewrite_refs(&utf8(&path, bytes)?, known);
                    files.push((inner, text.into_bytes()));
                } else {
                    files.push((inner, bytes));
                }
            }
        }
    }
    Ok(apply::RenderedFolder {
        name: target.name.clone(),
        files,
    })
}

fn push_split(files: &mut Vec<(PathBuf, Vec<u8>)>, skill_path: PathBuf, md: &str) {
    let (stub, body) = render::split_if_large(md);
    files.push((skill_path, stub.into_bytes()));
    if let Some(body) = body {
        files.push((PathBuf::from(render::BODY_PATH), body.into_bytes()));
    }
}

fn utf8(path: &Path, bytes: Vec<u8>) -> Result<String, String> {
    String::from_utf8(bytes).map_err(|_| format!("{} is not valid UTF-8 text", path.display()))
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
    fn render_targets_renders_markdown_and_copies_other_files() {
        // Arrange
        let plugin = empty_home("render-targets");
        let skill = plugin.join("skills/fix");
        fs::create_dir_all(skill.join("references")).unwrap();
        fs::write(
            skill.join("SKILL.md"),
            "---\nname: fix\n---\nsee /playbook:fix\n",
        )
        .unwrap();
        fs::write(skill.join("references/x.bin"), [0u8, 159, 255]).unwrap();
        let targets = plan::plan(&plugin).unwrap();

        // Act
        let folders = render_targets(&plugin, &targets).unwrap();

        // Assert
        let files: std::collections::BTreeMap<_, _> = folders[0].files.iter().cloned().collect();
        let md = String::from_utf8(files[Path::new("SKILL.md")].clone()).unwrap();
        assert!(
            md.contains("name: playbook-fix") && md.contains("$playbook-fix"),
            "{md}"
        );
        assert_eq!(files[Path::new("references/x.bin")], vec![0u8, 159, 255]);
    }

    #[test]
    fn a_codex_folder_alone_finds_codex() {
        // Arrange
        let home = empty_home("detect-folder");
        let bin = empty_home("detect-folder-bin");
        fs::create_dir(home.join(".codex")).unwrap();

        // Act
        let found = detect(&home, bin.to_str().unwrap());

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
