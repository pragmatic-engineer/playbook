// SPDX-FileCopyrightText: 2026 Igor Santos
// SPDX-License-Identifier: Apache-2.0

//! Maps the plugin folder to the list of skill folders to write. Pure
//! reading: it never writes.

use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};

/// Prefix on every synced folder, so they never clash with a user's own
/// skills in the shared folder.
pub const NAME_PREFIX: &str = "playbook-";
/// Codex rejects a skill name longer than this.
pub const MAX_NAME_LEN: usize = 64;

const SKILLS_DIR: &str = "skills";
const COMMANDS_DIR: &str = "commands";
const COMMAND_EXT: &str = "md";

/// What a source file is, because commands need rendering into a skill.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Source {
    SkillFile,
    Command,
}

/// One folder to write: its name and the plugin files it comes from, as
/// paths relative to the plugin folder, sorted.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Target {
    pub name: String,
    pub files: Vec<(PathBuf, Source)>,
}

/// Plans one target per skill folder and one per command file.
pub fn plan(plugin: &Path) -> Result<Vec<Target>, String> {
    let mut targets: BTreeMap<String, Target> = BTreeMap::new();
    for skill in sorted_entries(&plugin.join(SKILLS_DIR))? {
        if !skill.is_dir() {
            continue;
        }
        let rel = Path::new(SKILLS_DIR).join(file_name(&skill));
        let mut files = Vec::new();
        collect_files(plugin, &rel, &mut files)?;
        let files = files.into_iter().map(|f| (f, Source::SkillFile)).collect();
        insert(&mut targets, &file_name(&skill), files)?;
    }
    for command in sorted_entries(&plugin.join(COMMANDS_DIR))? {
        if command.extension().and_then(|e| e.to_str()) != Some(COMMAND_EXT) {
            continue;
        }
        let stem = command
            .file_stem()
            .and_then(|s| s.to_str())
            .unwrap_or_default()
            .to_string();
        let rel = Path::new(COMMANDS_DIR).join(file_name(&command));
        insert(&mut targets, &stem, vec![(rel, Source::Command)])?;
    }
    Ok(targets.into_values().collect())
}

fn insert(
    targets: &mut BTreeMap<String, Target>,
    base: &str,
    files: Vec<(PathBuf, Source)>,
) -> Result<(), String> {
    let name = format!("{NAME_PREFIX}{base}");
    if name.len() > MAX_NAME_LEN {
        return Err(format!(
            "{name} is {} characters long, the limit is {MAX_NAME_LEN}",
            name.len()
        ));
    }
    if targets.contains_key(&name) {
        return Err(format!(
            "a skill and a command are both named {base}, so both would become {name}"
        ));
    }
    targets.insert(name.clone(), Target { name, files });
    Ok(())
}

fn file_name(path: &Path) -> String {
    path.file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_default()
}

/// Entries of `dir` in name order; a missing folder reads as empty. Hidden
/// entries (`.DS_Store` and the like) are not plugin content.
fn sorted_entries(dir: &Path) -> Result<Vec<PathBuf>, String> {
    let read = match fs::read_dir(dir) {
        Ok(read) => read,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(e) => return Err(format!("cannot read {}: {e}", dir.display())),
    };
    let mut entries: Vec<PathBuf> = read
        .filter_map(Result::ok)
        .map(|e| e.path())
        .filter(|p| !file_name(p).starts_with('.'))
        .collect();
    entries.sort();
    Ok(entries)
}

/// Pushes every file under `plugin/rel`, as paths relative to `plugin`.
fn collect_files(plugin: &Path, rel: &Path, out: &mut Vec<PathBuf>) -> Result<(), String> {
    for entry in sorted_entries(&plugin.join(rel))? {
        let child = rel.join(file_name(&entry));
        if entry.is_dir() {
            collect_files(plugin, &child, out)?;
        } else {
            out.push(child);
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::common::test_support::scratch_dir;

    fn write(root: &Path, rel: &str) {
        let path = root.join(rel);
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(path, "text").unwrap();
    }

    fn fixture(tag: &str, files: &[&str]) -> PathBuf {
        let plugin = scratch_dir(tag);
        fs::create_dir_all(&plugin).unwrap();
        for f in files {
            write(&plugin, f);
        }
        plugin
    }

    #[test]
    fn two_skills_and_two_commands_map_to_four_targets() {
        // Arrange
        let plugin = fixture(
            "plan-four",
            &[
                "skills/alpha/SKILL.md",
                "skills/alpha/references/deep.md",
                "skills/beta/SKILL.md",
                "commands/fix.md",
                "commands/ship.md",
            ],
        );
        let skill = |p: &str| (PathBuf::from(p), Source::SkillFile);
        let command = |p: &str| (PathBuf::from(p), Source::Command);

        // Act
        let targets = plan(&plugin).unwrap();

        // Assert
        let expected = vec![
            Target {
                name: "playbook-alpha".into(),
                files: vec![
                    skill("skills/alpha/SKILL.md"),
                    skill("skills/alpha/references/deep.md"),
                ],
            },
            Target {
                name: "playbook-beta".into(),
                files: vec![skill("skills/beta/SKILL.md")],
            },
            Target {
                name: "playbook-fix".into(),
                files: vec![command("commands/fix.md")],
            },
            Target {
                name: "playbook-ship".into(),
                files: vec![command("commands/ship.md")],
            },
        ];
        assert_eq!(targets, expected);
    }

    #[test]
    fn a_name_over_the_limit_is_an_error() {
        // Arrange
        let long = "x".repeat(MAX_NAME_LEN - NAME_PREFIX.len() + 1);
        let plugin = fixture("plan-long", &[&format!("commands/{long}.md")]);

        // Act
        let err = plan(&plugin).unwrap_err();

        // Assert
        assert!(err.contains("the limit is 64"), "{err}");
    }

    #[test]
    fn a_name_at_the_limit_is_accepted() {
        // Arrange
        let exact = "x".repeat(MAX_NAME_LEN - NAME_PREFIX.len());
        let plugin = fixture("plan-exact", &[&format!("commands/{exact}.md")]);

        // Act
        let targets = plan(&plugin).unwrap();

        // Assert
        assert_eq!(targets.len(), 1);
    }

    #[test]
    fn a_skill_and_a_command_with_the_same_name_are_an_error() {
        // Arrange
        let plugin = fixture("plan-clash", &["skills/fix/SKILL.md", "commands/fix.md"]);

        // Act
        let err = plan(&plugin).unwrap_err();

        // Assert
        assert!(err.contains("both named fix"), "{err}");
    }

    #[test]
    fn the_real_plugin_plans_without_error() {
        // Arrange
        let plugin = Path::new(env!("PLAYBOOK_PLUGIN_ROOT"));

        // Act
        let targets = plan(plugin).unwrap();

        // Assert
        assert!(!targets.is_empty());
    }
}
