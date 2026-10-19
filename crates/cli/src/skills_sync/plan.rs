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
        if !is_dir(&skill)? {
            continue;
        }
        let name = file_name(&skill)?;
        let rel = Path::new(SKILLS_DIR).join(&name);
        let mut files = Vec::new();
        collect_files(plugin, &rel, &mut files)?;
        let files = files.into_iter().map(|f| (f, Source::SkillFile)).collect();
        insert(&mut targets, &name, files)?;
    }
    for command in sorted_entries(&plugin.join(COMMANDS_DIR))? {
        let file = file_name(&command)?;
        if command.extension().and_then(|e| e.to_str()) != Some(COMMAND_EXT) {
            continue;
        }
        is_dir(&command)?;
        let stem = command
            .file_stem()
            .and_then(|s| s.to_str())
            .ok_or_else(|| format!("{} has no usable name", command.display()))?;
        let rel = Path::new(COMMANDS_DIR).join(&file);
        insert(&mut targets, stem, vec![(rel, Source::Command)])?;
    }
    Ok(targets.into_values().collect())
}

fn insert(
    targets: &mut BTreeMap<String, Target>,
    base: &str,
    files: Vec<(PathBuf, Source)>,
) -> Result<(), String> {
    if base.is_empty() || !base.chars().all(is_name_char) {
        return Err(format!(
            "{base} is not a valid name: use only lowercase ASCII letters, digits and '-'"
        ));
    }
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

/// Lowercase only, so names that differ by case cannot merge on a case
/// insensitive file system.
fn is_name_char(c: char) -> bool {
    c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-'
}

fn file_name(path: &Path) -> Result<String, String> {
    path.file_name()
        .and_then(|n| n.to_str())
        .map(str::to_string)
        .ok_or_else(|| format!("{} is not a valid UTF-8 file name", path.display()))
}

/// Whether `path` is a folder, without following links. A symlink is an
/// error, because plugin content must be plain files and folders.
fn is_dir(path: &Path) -> Result<bool, String> {
    let kind = fs::symlink_metadata(path)
        .map_err(|e| format!("cannot read {}: {e}", path.display()))?
        .file_type();
    if kind.is_symlink() {
        return Err(format!(
            "{} is a symlink, but plugin content must be plain files and folders",
            path.display()
        ));
    }
    Ok(kind.is_dir())
}

/// Entries of `dir` in name order; a missing folder reads as empty. Hidden
/// entries (`.DS_Store` and the like) are not plugin content.
fn sorted_entries(dir: &Path) -> Result<Vec<PathBuf>, String> {
    let read = match fs::read_dir(dir) {
        Ok(read) => read,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(e) => return Err(format!("cannot read {}: {e}", dir.display())),
    };
    let mut entries = Vec::new();
    for entry in read {
        let path = entry
            .map_err(|e| format!("cannot read {}: {e}", dir.display()))?
            .path();
        if !file_name(&path)?.starts_with('.') {
            entries.push(path);
        }
    }
    entries.sort();
    Ok(entries)
}

/// Pushes every file under `plugin/rel`, as paths relative to `plugin`.
fn collect_files(plugin: &Path, rel: &Path, out: &mut Vec<PathBuf>) -> Result<(), String> {
    for entry in sorted_entries(&plugin.join(rel))? {
        let child = rel.join(file_name(&entry)?);
        if is_dir(&entry)? {
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
    fn a_symlinked_skill_folder_is_an_error_naming_it() {
        // Arrange
        let plugin = fixture("plan-link-dir", &["elsewhere/SKILL.md"]);
        fs::create_dir_all(plugin.join("skills")).unwrap();
        std::os::unix::fs::symlink(plugin.join("elsewhere"), plugin.join("skills/linked")).unwrap();

        // Act
        let err = plan(&plugin).unwrap_err();

        // Assert
        assert!(err.contains("linked"), "{err}");
        assert!(err.contains("symlink"), "{err}");
    }

    #[test]
    fn a_symlinked_file_inside_a_skill_is_an_error_naming_it() {
        // Arrange
        let plugin = fixture("plan-link-file", &["skills/alpha/SKILL.md", "outside.md"]);
        std::os::unix::fs::symlink(
            plugin.join("outside.md"),
            plugin.join("skills/alpha/ref.md"),
        )
        .unwrap();

        // Act
        let err = plan(&plugin).unwrap_err();

        // Assert
        assert!(err.contains("ref.md"), "{err}");
        assert!(err.contains("symlink"), "{err}");
    }

    #[test]
    fn a_symlinked_command_is_an_error_naming_it() {
        // Arrange
        let plugin = fixture("plan-link-cmd", &["outside.md"]);
        fs::create_dir_all(plugin.join("commands")).unwrap();
        std::os::unix::fs::symlink(plugin.join("outside.md"), plugin.join("commands/go.md"))
            .unwrap();

        // Act
        let err = plan(&plugin).unwrap_err();

        // Assert
        assert!(err.contains("go.md"), "{err}");
        assert!(err.contains("symlink"), "{err}");
    }

    #[test]
    fn a_name_with_uppercase_or_other_characters_is_an_error() {
        for bad in ["Fix", "fix_it", "fix it", "fïx"] {
            // Arrange
            let plugin = fixture("plan-badname", &[&format!("commands/{bad}.md")]);

            // Act
            let err = plan(&plugin).unwrap_err();

            // Assert
            assert!(err.contains("not a valid name"), "{bad}: {err}");
        }
    }

    // macOS file systems reject non-UTF-8 names, so only Linux can create one.
    #[cfg(target_os = "linux")]
    #[test]
    fn a_non_utf8_file_name_is_an_error() {
        use std::os::unix::ffi::OsStrExt;

        // Arrange
        let plugin = fixture("plan-utf8", &[]);
        fs::create_dir_all(plugin.join("commands")).unwrap();
        let bad = std::ffi::OsStr::from_bytes(b"f\xffx.md");
        fs::write(plugin.join("commands").join(bad), "text").unwrap();

        // Act
        let err = plan(&plugin).unwrap_err();

        // Assert
        assert!(err.contains("not a valid UTF-8 file name"), "{err}");
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
