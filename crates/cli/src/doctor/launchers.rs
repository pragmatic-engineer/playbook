// SPDX-FileCopyrightText: 2026 Igor Santos
// SPDX-License-Identifier: Apache-2.0

//! Finds the old `ccc` and `ccd` launchers so `playbook doctor` and
//! `playbook init` can point at their replacements. Read only: nothing here
//! deletes or edits a user's file.

use std::ffi::OsStr;
use std::path::{Path, PathBuf};

/// Shell startup files that may define the old names.
const RC_FILES: [&str; 6] = [
    ".zshrc",
    ".zshenv",
    ".zprofile",
    ".bashrc",
    ".bash_profile",
    ".profile",
];

/// The old launcher names and what replaces each.
const OLD_NAMES: [(&str, &str); 2] = [("ccc", "playbook cc"), ("ccd", "playbook ccd")];

/// One old launcher found on this machine.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Finding {
    /// Where it lives: an executable path, or an rc file with a line number.
    pub place: String,
    /// What to run instead.
    pub replacement: &'static str,
}

/// Whether `line` defines the shell name `name` as an alias or a function.
fn defines(line: &str, name: &str) -> bool {
    let line = line.trim_start();
    if line.starts_with('#') {
        return false;
    }
    let alias = format!("alias {name}=");
    let func = format!("{name}()");
    let kw = format!("function {name}");
    line.starts_with(&alias)
        || line.starts_with(&func)
        || line
            .strip_prefix(&kw)
            .is_some_and(|r| r.is_empty() || r.starts_with([' ', '(', '{']))
}

/// The aliases and functions in `text` that define an old launcher name.
/// Lines the playbook-managed `shell-init` block owns are not matched,
/// because that block no longer defines them.
pub fn scan_text(file: &str, text: &str) -> Vec<Finding> {
    let mut out = Vec::new();
    for (i, line) in text.lines().enumerate() {
        for (name, replacement) in OLD_NAMES {
            if defines(line, name) {
                out.push(Finding {
                    place: format!("{file}:{} defines {name}", i + 1),
                    replacement,
                });
            }
        }
    }
    out
}

fn executable(path: &Path) -> bool {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        path.metadata()
            .is_ok_and(|m| m.is_file() && m.permissions().mode() & 0o111 != 0)
    }
    #[cfg(not(unix))]
    {
        path.is_file()
    }
}

/// Every old launcher under `home`'s rc files and in the `path_var` directories.
pub fn find(home: &Path, path_var: &OsStr) -> Vec<Finding> {
    let mut out = Vec::new();
    for rc in RC_FILES {
        let path = home.join(rc);
        if let Ok(text) = std::fs::read_to_string(&path) {
            out.extend(scan_text(&format!("~/{rc}"), &text));
        }
    }
    for dir in std::env::split_paths(path_var).filter(|d| d.is_absolute()) {
        for (name, replacement) in OLD_NAMES {
            let candidate: PathBuf = dir.join(name);
            if executable(&candidate) {
                out.push(Finding {
                    place: candidate.display().to_string(),
                    replacement,
                });
            }
        }
    }
    out
}

/// One line per finding, for `init` and `doctor`.
pub fn describe(finding: &Finding) -> String {
    format!(
        "old launcher at {}: run `{}` instead, then remove it yourself",
        finding.place, finding.replacement
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn alias_and_function_definitions_are_found() {
        // Arrange
        let text = "export A=1\nalias ccc='claude'\nccd() { claude; }\n";

        // Act
        let found = scan_text("~/.zshrc", text);

        // Assert
        assert_eq!(found.len(), 2);
        assert_eq!(found[0].replacement, "playbook cc");
        assert_eq!(found[1].replacement, "playbook ccd");
        assert!(found[0].place.contains(":2"));
    }

    #[test]
    fn comments_and_other_names_are_ignored() {
        // Arrange
        let text = "# alias ccc=x\nalias cccx=y\nalias cd=z\n";

        // Act
        let found = scan_text("rc", text);

        // Assert
        assert!(found.is_empty());
    }

    #[test]
    fn an_executable_on_path_is_found_and_nothing_is_deleted() {
        // Arrange
        let dir = std::env::temp_dir().join(format!("pb-launchers-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap_or_default();
        let file = dir.join("ccc");
        std::fs::write(&file, "#!/bin/sh\n").unwrap_or_default();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let _ = std::fs::set_permissions(&file, std::fs::Permissions::from_mode(0o755));
        }

        // Act
        let found = find(&dir.join("home"), dir.as_os_str());

        // Assert
        assert_eq!(found.len(), 1);
        assert!(file.exists());
        let _ = std::fs::remove_dir_all(&dir);
    }
}
