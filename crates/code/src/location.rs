// SPDX-FileCopyrightText: 2026 Igor Santos
// SPDX-License-Identifier: Apache-2.0

//! Where the repo root and the index live, and how user supplied paths become
//! repo relative paths with `/` separators.

use super::host;
use std::path::{Component, Path, PathBuf};

/// Why a user supplied path was refused.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PathError {
    /// The path does not exist (or cannot be resolved).
    NotFound(String),
    /// The path resolves outside the repo root, possibly through a symlink.
    Outside(String),
    /// The path is the repo root itself, which is not a file.
    IsRoot,
}

impl std::fmt::Display for PathError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            PathError::NotFound(p) => write!(f, "no such file or directory: {p}"),
            PathError::Outside(p) => write!(f, "path is outside the repository: {p}"),
            PathError::IsRoot => f.write_str("path is the repository root, not a file"),
        }
    }
}

impl std::error::Error for PathError {}

/// The work tree root that contains `cwd`.
pub fn repo_root(cwd: &Path) -> Result<PathBuf, String> {
    playbook_common::gitfacts::toplevel(cwd)
        .ok_or_else(|| "not inside a git repository".to_string())
}

/// `<data dir>/code/<owner>/<repo>/<worktree-id>.db` for the current repo.
/// Moves an index kept at the old worktree state path on first use.
pub fn index_path() -> Result<PathBuf, String> {
    host::index_db()
}

/// Replaces `\` with `/`. Pure, so it is tested on strings on every platform.
pub fn to_slash(path: &str) -> String {
    path.replace('\\', "/")
}

/// Joins the normal components of a relative path with `/`.
fn slash_join(rel: &Path) -> String {
    rel.components()
        .filter_map(|c| match c {
            Component::Normal(part) => Some(part.to_string_lossy().into_owned()),
            _ => None,
        })
        .collect::<Vec<_>>()
        .join("/")
}

/// Resolves `input` (relative to `cwd` when relative) and returns its path
/// relative to `root`, with `/` separators. Symlinks are resolved first, so a
/// link that points outside the root is refused.
pub fn to_repo_relative(root: &Path, cwd: &Path, input: &str) -> Result<String, PathError> {
    let raw = Path::new(input);
    let joined = if raw.is_absolute() {
        raw.to_path_buf()
    } else {
        cwd.join(raw)
    };
    let canonical_root = root
        .canonicalize()
        .map_err(|_| PathError::NotFound(root.display().to_string()))?;
    let Ok(resolved) = joined.canonicalize() else {
        // Name the path as the user wrote it. A missing path outside the
        // root is still an outside path, so decide that lexically.
        return Err(if lexically_outside(&canonical_root, &joined) {
            PathError::Outside(to_slash(input))
        } else {
            PathError::NotFound(to_slash(input))
        });
    };
    let rel = resolved
        .strip_prefix(&canonical_root)
        .map_err(|_| PathError::Outside(to_slash(input)))?;
    if rel.as_os_str().is_empty() {
        return Err(PathError::IsRoot);
    }
    Ok(slash_join(rel))
}

fn lexically_outside(canonical_root: &Path, joined: &Path) -> bool {
    let mut stack: Vec<Component> = Vec::new();
    for c in joined.components() {
        match c {
            Component::ParentDir => {
                stack.pop();
            }
            Component::CurDir => {}
            other => stack.push(other),
        }
    }
    let normal: PathBuf = stack.iter().collect();
    !normal.starts_with(canonical_root)
}

#[cfg(test)]
mod tests {
    use super::*;
    use playbook_common::test_support::scratch_dir;
    use std::fs;

    fn scratch(tag: &str) -> PathBuf {
        let dir = scratch_dir(tag);
        fs::create_dir_all(&dir).expect("dir");
        dir.canonicalize().expect("canonical")
    }

    #[test]
    fn a_parent_traversal_and_an_outside_absolute_path_are_rejected() {
        // Arrange
        let base = scratch("loc-outside");
        let root = base.join("repo");
        fs::create_dir_all(&root).expect("root");
        fs::write(base.join("x"), "x").expect("outside file");

        // Act
        let parent = to_repo_relative(&root, &root, "../x");
        let absolute = to_repo_relative(&root, &root, &base.join("x").display().to_string());

        // Assert
        assert!(matches!(parent, Err(PathError::Outside(_))), "{parent:?}");
        assert!(
            matches!(absolute, Err(PathError::Outside(_))),
            "{absolute:?}"
        );
        let _ = fs::remove_dir_all(base);
    }

    #[cfg(unix)]
    #[test]
    fn a_symlink_out_of_the_root_is_rejected_and_one_inside_is_accepted() {
        // Arrange
        let base = scratch("loc-symlink");
        let root = base.join("repo");
        fs::create_dir_all(&root).expect("root");
        fs::write(base.join("secret"), "s").expect("secret");
        fs::write(root.join("real.txt"), "r").expect("real");
        std::os::unix::fs::symlink(base.join("secret"), root.join("escape")).expect("link out");
        std::os::unix::fs::symlink(root.join("real.txt"), root.join("alias")).expect("link in");

        // Act
        let escape = to_repo_relative(&root, &root, "escape");
        let alias = to_repo_relative(&root, &root, "alias");

        // Assert
        assert!(matches!(escape, Err(PathError::Outside(_))), "{escape:?}");
        assert_eq!(alias, Ok("real.txt".to_string()));
        let _ = fs::remove_dir_all(base);
    }

    #[test]
    fn the_root_itself_and_a_missing_path_have_their_documented_results() {
        // Arrange
        let root = scratch("loc-root");

        // Act
        let at_root = to_repo_relative(&root, &root, ".");
        let missing = to_repo_relative(&root, &root, "nope/file.rs");

        // Assert
        assert_eq!(at_root, Err(PathError::IsRoot));
        assert_eq!(
            missing,
            Err(PathError::NotFound("nope/file.rs".to_string()))
        );
        assert!(missing
            .expect_err("error")
            .to_string()
            .contains("nope/file.rs"));
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn a_relative_input_resolves_against_the_cwd_then_becomes_repo_relative() {
        // Arrange
        let root = scratch("loc-cwd");
        fs::create_dir_all(root.join("lib/payment")).expect("dirs");
        fs::write(root.join("lib/payment/service.ts"), "x").expect("file");
        let cwd = root.join("lib");

        // Act
        let got = to_repo_relative(&root, &cwd, "payment/service.ts");

        // Assert
        assert_eq!(got, Ok("lib/payment/service.ts".to_string()));
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn backslashes_become_slashes_in_the_pure_separator_function() {
        // Arrange, Act
        let got = to_slash("a\\b\\c.rs");

        // Assert
        assert_eq!(got, "a/b/c.rs");
    }
}
