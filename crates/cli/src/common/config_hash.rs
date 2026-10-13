// SPDX-FileCopyrightText: 2026 Igor Santos
// SPDX-License-Identifier: Apache-2.0

//! The config hash: "did Claude Code's runtime config change".
//!
//! It is the first 16 hex characters of the SHA-256 over `~/.claude/settings.json`
//! followed by every hook script under `~/.claude/hooks` (`*.sh` and `*.py`,
//! tests excluded, symlinks skipped), in byte order of their paths. This is the
//! same value the retired shell `config_hash` function produced, so a hash stored
//! by an older version still compares equal.
//!
//! An unreadable file counts as absent, like the shell's `2>/dev/null`.

use sha2::{Digest, Sha256};
use std::fs;
use std::path::{Path, PathBuf};

/// The hash for the config under `claude_home` (normally `~/.claude`).
pub fn config_hash(claude_home: &Path) -> String {
    let mut hasher = Sha256::new();
    if let Ok(settings) = fs::read(claude_home.join("settings.json")) {
        hasher.update(&settings);
    }
    let mut scripts = Vec::new();
    collect_scripts(&claude_home.join("hooks"), &mut scripts);
    scripts.sort_by(|a, b| path_bytes(a).cmp(path_bytes(b)));
    for script in scripts {
        if let Ok(body) = fs::read(&script) {
            hasher.update(&body);
        }
    }
    let hex: String = hasher
        .finalize()
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect();
    hex[..16].to_string()
}

#[cfg(unix)]
fn path_bytes(p: &Path) -> &[u8] {
    use std::os::unix::ffi::OsStrExt;
    p.as_os_str().as_bytes()
}

#[cfg(not(unix))]
fn path_bytes(p: &Path) -> &[u8] {
    p.as_os_str().to_str().unwrap_or("").as_bytes()
}

fn is_hashed(name: &str) -> bool {
    let script = name.ends_with(".sh") || name.ends_with(".py");
    let test =
        name.ends_with(".test.sh") || name.ends_with(".test.py") || name.ends_with("_test.py");
    script && !test
}

/// Every regular file (not a symlink) below `dir` that `is_hashed` accepts.
fn collect_scripts(dir: &Path, out: &mut Vec<PathBuf>) {
    let Ok(entries) = fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let Ok(kind) = entry.file_type() else {
            continue;
        };
        let path = entry.path();
        if kind.is_dir() {
            collect_scripts(&path, out);
        } else if kind.is_file() && is_hashed(&entry.file_name().to_string_lossy()) {
            out.push(path);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::common::test_support::scratch_dir;

    fn home(tag: &str) -> PathBuf {
        let h = scratch_dir(tag);
        fs::create_dir_all(h.join("hooks")).unwrap();
        h
    }

    #[test]
    fn an_empty_config_hashes_the_empty_input() {
        assert_eq!(config_hash(&home("ch-empty")), "e3b0c44298fc1c14");
    }

    /// Known answers computed with the retired shell function on the same tree.
    #[test]
    fn matches_the_values_the_shell_function_produced() {
        let h = home("ch-known");
        fs::write(h.join("settings.json"), "{\"a\":1}\n").unwrap();
        fs::write(h.join("hooks/b.sh"), "echo b\n").unwrap();
        fs::write(h.join("hooks/a.py"), "print('a')\n").unwrap();
        fs::create_dir_all(h.join("hooks/sub")).unwrap();
        fs::write(h.join("hooks/sub/c.sh"), "echo c\n").unwrap();
        // Ignored: tests, other extensions.
        fs::write(h.join("hooks/x.test.sh"), "t").unwrap();
        fs::write(h.join("hooks/y.test.py"), "t").unwrap();
        fs::write(h.join("hooks/z_test.py"), "t").unwrap();
        fs::write(h.join("hooks/readme.md"), "t").unwrap();
        assert_eq!(config_hash(&h), KNOWN);
    }

    const KNOWN: &str = "409b1ddeccfb14c1";

    #[test]
    fn test_files_and_other_extensions_do_not_change_the_hash() {
        let h = home("ch-ignore");
        fs::write(h.join("hooks/a.sh"), "x").unwrap();
        let before = config_hash(&h);
        fs::write(h.join("hooks/a.test.sh"), "changed").unwrap();
        fs::write(h.join("hooks/b_test.py"), "changed").unwrap();
        fs::write(h.join("hooks/notes.txt"), "changed").unwrap();
        assert_eq!(config_hash(&h), before);
    }

    #[test]
    fn editing_settings_or_a_hook_changes_the_hash() {
        let h = home("ch-edit");
        fs::write(h.join("hooks/a.sh"), "x").unwrap();
        let base = config_hash(&h);
        fs::write(h.join("settings.json"), "{}").unwrap();
        let with_settings = config_hash(&h);
        assert_ne!(base, with_settings);
        fs::write(h.join("hooks/a.sh"), "y").unwrap();
        assert_ne!(with_settings, config_hash(&h));
    }

    #[cfg(unix)]
    #[test]
    fn a_symlinked_script_is_skipped_like_find_type_f() {
        let h = home("ch-link");
        fs::write(h.join("real.sh"), "x").unwrap();
        let base = config_hash(&h);
        std::os::unix::fs::symlink(h.join("real.sh"), h.join("hooks/link.sh")).unwrap();
        assert_eq!(config_hash(&h), base);
    }

    #[test]
    fn a_missing_home_is_the_empty_hash_not_a_panic() {
        assert_eq!(
            config_hash(Path::new("/nonexistent/playbook-ch")),
            "e3b0c44298fc1c14"
        );
    }
}
