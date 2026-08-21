// SPDX-FileCopyrightText: 2026 Igor Santos
// SPDX-License-Identifier: MIT

//! Port of `shell/check-manifest.sh`, guarding the tracked-file allowlist so
//! runtime state or a re-tracked personal settings.json cannot leak into the
//! public repo. Every tracked path must be an allowlisted top-level file or
//! live under an allowlisted top-level directory.
//!
//! One deliberate divergence from the shell: it required `jq` on PATH before
//! doing anything (`check-manifest.sh:17`) but never called it, and this port
//! does no JSON work, so the dead requirement is dropped rather than ported.
//!
//! The shell's optional REPO_ROOT argument IS preserved: omitting it falls
//! back to `git rev-parse --show-toplevel` (see [`toplevel`]), so running the
//! subcommand bare from anywhere inside the repo behaves as the script did.

use std::path::{Path, PathBuf};
use std::process::Command;

/// Top-level files the manifest allows verbatim.
const ALLOW_FILES: [&str; 16] = [
    ".gitignore",
    "README.md",
    "LICENSE",
    "Brewfile",
    "justfile",
    "install.sh",
    "uninstall.sh",
    "settings.shared.json",
    "permissions.shared.json",
    "statusline.sh",
    "ruff.toml",
    "CODE_OF_CONDUCT.md",
    "CONTRIBUTING.md",
    "SECURITY.md",
    "Cargo.toml",
    "Cargo.lock",
];

/// Top-level directories any tracked file may live under.
const ALLOW_DIRS: [&str; 12] = [
    "prompts",
    "skills",
    "commands",
    "agents",
    "hooks",
    "shell",
    "docs",
    "output-styles",
    ".github",
    ".claude-plugin",
    "src",
    "tests",
];

/// Belt-and-suspenders: the owner's personal settings.json must never track,
/// even though nothing in `ALLOW_FILES` would otherwise admit it. Returns the
/// offender's report line, or `None` if `path` is allowlisted.
fn offender(path: &str) -> Option<String> {
    if path == "settings.json" {
        return Some(format!(
            "{path}  (personal settings.json must not be tracked)"
        ));
    }
    let allowed = match path.split_once('/') {
        Some((top, _rest)) => ALLOW_DIRS.contains(&top),
        None => ALLOW_FILES.contains(&path),
    };
    (!allowed).then(|| path.to_string())
}

/// Pure decision: given the tracked paths `git ls-files` reports, return the
/// offenders in encounter order. No filesystem or git access, so every
/// allowlist case is exercised directly against a plain list, no repo needed.
fn violations(tracked: &[String]) -> Vec<String> {
    tracked
        .iter()
        .filter(|path| !path.is_empty())
        .filter_map(|path| offender(path))
        .collect()
}

/// Runs `git ls-files` against `repo_root` and checks every tracked path
/// against the allowlist. Mirrors the shell original: success lists the
/// tracked-file total, failure lists every offender.
pub fn check(repo_root: &Path) -> Result<String, String> {
    if !repo_root.is_dir() {
        return Err(format!(
            "repo root is not a directory: {}",
            repo_root.display()
        ));
    }

    let output = Command::new("git")
        .arg("-C")
        .arg(repo_root)
        .arg("ls-files")
        .output()
        .map_err(|err| format!("failed to run git ls-files: {err}"))?;
    if !output.status.success() {
        return Err(format!("not a git repository: {}", repo_root.display()));
    }

    let tracked: Vec<String> = String::from_utf8_lossy(&output.stdout)
        .lines()
        .map(str::to_string)
        .collect();
    let total = tracked.len();
    let offenders = violations(&tracked);

    if offenders.is_empty() {
        return Ok(format!(
            "check-manifest: OK ({total} tracked files, all allowlisted)"
        ));
    }

    let mut message = format!("{} tracked file(s) outside the allowlist:", offenders.len());
    for item in &offenders {
        message.push_str(&format!("\n  {item}"));
    }
    Err(message)
}

/// The enclosing repo root, mirroring the shell's default when no `REPO_ROOT`
/// argument was given (check-manifest.sh:21-22). `None` when the cwd is not
/// inside a git repository.
pub fn toplevel() -> Option<PathBuf> {
    let out = Command::new("git")
        .args(["-C", ".", "rev-parse", "--show-toplevel"])
        .output()
        .ok()?;
    if !out.status.success() {
        return None;
    }
    let path = String::from_utf8_lossy(&out.stdout).trim().to_string();
    (!path.is_empty()).then(|| PathBuf::from(path))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn allowlisted_top_level_file_passes() {
        let tracked = vec!["README.md".to_string()];
        assert!(violations(&tracked).is_empty());
    }

    #[test]
    fn allowlisted_top_level_directory_passes() {
        let tracked = vec!["shell/worktree.zsh".to_string()];
        assert!(violations(&tracked).is_empty());
    }

    #[test]
    fn disallowed_top_level_file_is_an_offender() {
        let tracked = vec!["stray.txt".to_string()];
        assert_eq!(violations(&tracked), vec!["stray.txt".to_string()]);
    }

    #[test]
    fn new_disallowed_top_level_directory_is_an_offender() {
        // This is the exact failure the validator exists to catch: a brand
        // new top-level directory that was never allowlisted.
        let tracked = vec!["sessions/leaked.json".to_string()];
        assert_eq!(
            violations(&tracked),
            vec!["sessions/leaked.json".to_string()]
        );
    }

    #[test]
    fn tracked_personal_settings_json_is_an_offender() {
        let tracked = vec!["settings.json".to_string()];
        let result = violations(&tracked);
        assert_eq!(result.len(), 1);
        assert!(result[0].contains("settings.json"));
        assert!(result[0].contains("personal settings.json must not be tracked"));
    }

    #[test]
    fn clean_skeleton_has_no_offenders() {
        let tracked = vec![
            ".gitignore".to_string(),
            "README.md".to_string(),
            "LICENSE".to_string(),
            "settings.shared.json".to_string(),
            "permissions.shared.json".to_string(),
            "statusline.sh".to_string(),
            "shell/worktree.zsh".to_string(),
            "hooks/session-init.sh".to_string(),
            "docs/index.md".to_string(),
            ".github/workflows/ci.yml".to_string(),
        ];
        assert!(violations(&tracked).is_empty());
    }

    #[test]
    fn empty_lines_are_skipped_not_counted_as_offenders() {
        let tracked = vec![String::new(), "README.md".to_string(), String::new()];
        assert!(violations(&tracked).is_empty());
    }
}
