// SPDX-FileCopyrightText: 2026 Igor Santos
// SPDX-License-Identifier: MIT

//! Session identity and per-session state paths. Ports `session_id`
//! (hooks/lib/common.py:81), `session_dir` (:86), and `abspath` (:110).

use crate::common::payload::Payload;
use std::fs;
use std::path::{Path, PathBuf};

/// Return the `session_id` from the hook payload, or empty if absent.
pub fn session_id(payload: &Payload) -> String {
    payload.field(".session_id")
}

/// Resolve the current user's home directory, falling back to the system
/// passwd database when `$HOME` is unset (`std::env::home_dir`'s Unix
/// behaviour), matching python's `os.path.expanduser("~")`
/// (hooks/lib/common.py:22). `std::env::var("HOME").unwrap_or_default()`
/// silently yields an empty string when `HOME` is unset, and joining a path
/// onto that produces a path relative to the process's current directory
/// instead of the real home. Empty only when even the passwd lookup fails.
/// Never panics.
pub fn home_dir() -> PathBuf {
    std::env::home_dir().unwrap_or_default()
}

/// Return the per-session state directory under `$HOME/.claude/runtime`,
/// creating it on demand with mode 0700. Empty when no session id is present.
pub fn session_dir(payload: &Payload) -> String {
    session_dir_in(&runtime_root(), payload)
}

/// Stays `.claude`-rooted, unlike `paths::runtime_root()`: other consumers
/// of a session's runtime files still resolve this same old root today.
fn runtime_root() -> PathBuf {
    home_dir().join(".claude").join("runtime")
}

/// Core of `session_dir`, taking the runtime root explicitly so tests can
/// point it at a scratch directory instead of mutating the real `$HOME`.
fn session_dir_in(root: &Path, payload: &Payload) -> String {
    let sid = session_id(payload);
    if sid.is_empty() {
        return String::new();
    }
    let dir = root.join(&sid);
    if !dir.is_dir() {
        let mut builder = fs::DirBuilder::new();
        builder.recursive(true);
        // Session dirs hold per-session state, so they are owner-only. The
        // mode bits are unix-only; on Windows the directory inherits the
        // parent ACL instead, and the runtime root already sits under the
        // user's own profile. Gated rather than dropped so the guarantee is
        // not silently weakened on the platform that still has it.
        #[cfg(unix)]
        {
            use std::os::unix::fs::DirBuilderExt;
            builder.mode(0o700);
        }
        let _ = builder.create(&dir);
    }
    dir.to_string_lossy().into_owned()
}

/// Resolve a path to absolute. For a directory, returns the realpath. For a
/// non-existent path or a file, resolves the parent directory's realpath and
/// re-appends the basename, so a leaf symlink stays unresolved (matches
/// common.sh: callers key on the path the tool referenced, not its target).
/// Tolerates non-existent paths. Empty input returns empty.
pub fn abspath(p: &str) -> String {
    if p.is_empty() {
        return String::new();
    }
    if Path::new(p).is_dir() {
        return fs::canonicalize(p)
            .map(|real| real.to_string_lossy().into_owned())
            .unwrap_or_else(|_| p.to_string());
    }
    let (dir, base) = split_dir_base(p);
    if Path::new(dir).is_dir() {
        if let Ok(real_dir) = fs::canonicalize(dir) {
            return format!("{}/{}", real_dir.to_string_lossy(), base);
        }
    }
    p.to_string()
}

/// POSIX-style dirname/basename split on the last `/`, mirroring
/// `os.path.dirname`/`os.path.basename` rather than `std::path::Path`'s
/// component semantics, since common.py and common.sh both operate on the
/// raw string.
fn split_dir_base(p: &str) -> (&str, &str) {
    match p.rsplit_once('/') {
        Some(("", base)) => ("/", base),
        Some((dir, base)) => (dir, base),
        None => (".", p),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::common::test_support::scratch_dir;
    #[cfg(unix)]
    use std::os::unix::fs::PermissionsExt;

    #[test]
    fn session_id_extracts_value() {
        // Arrange
        let payload = Payload::parse(r#"{"session_id":"sid-xyz"}"#);

        // Act
        let got = session_id(&payload);

        // Assert
        assert_eq!(got, "sid-xyz");
    }

    #[test]
    fn session_id_empty_when_missing() {
        // Arrange
        let payload = Payload::parse("{}");

        // Act
        let got = session_id(&payload);

        // Assert
        assert_eq!(got, "");
    }

    #[test]
    fn session_dir_in_empty_when_no_session_id() {
        // Arrange
        let root = scratch_dir("session-dir-empty");
        let payload = Payload::parse("{}");

        // Act
        let got = session_dir_in(&root, &payload);

        // Assert
        assert_eq!(got, "");
    }

    #[test]
    fn session_dir_in_creates_and_returns_expected_path() {
        // Arrange
        let root = scratch_dir("session-dir-create");
        let payload = Payload::parse(r#"{"session_id":"testsid"}"#);

        // Act
        let got = session_dir_in(&root, &payload);

        // Assert
        let expected = root.join("testsid");
        assert_eq!(got, expected.to_string_lossy());
        assert!(expected.is_dir());
        // The 0700 guarantee only exists where mode bits do.
        #[cfg(unix)]
        {
            let mode = fs::metadata(&expected).unwrap().permissions().mode() & 0o777;
            assert_eq!(
                mode, 0o700,
                "session dir should be created with mode 0700, got {mode:o}"
            );
        }

        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn session_dir_reads_the_real_home_env_var() {
        // Arrange: the only test in this suite that touches the real HOME
        // env var, so there is nothing else running in this process to race
        // with it.
        let root = scratch_dir("session-dir-home");
        let payload = Payload::parse(r#"{"session_id":"envsid"}"#);
        let previous_home = std::env::var("HOME").ok();
        unsafe {
            std::env::set_var("HOME", &root);
        }

        // Act
        let got = session_dir(&payload);

        // Assert
        let expected = root.join(".claude").join("runtime").join("envsid");
        assert_eq!(got, expected.to_string_lossy());
        assert!(expected.is_dir());

        match previous_home {
            Some(value) => unsafe { std::env::set_var("HOME", value) },
            None => unsafe { std::env::remove_var("HOME") },
        }
        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn abspath_empty_input_returns_empty() {
        // Arrange, Act
        let got = abspath("");

        // Assert
        assert_eq!(got, "");
    }

    #[test]
    fn abspath_directory_resolves_realpath() {
        // Arrange
        let root = scratch_dir("abspath-dir");
        fs::create_dir_all(&root).unwrap();
        let expected = fs::canonicalize(&root).unwrap();

        // Act
        let got = abspath(root.to_str().unwrap());

        // Assert
        assert_eq!(got, expected.to_string_lossy());

        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn abspath_nonexistent_file_resolves_parent_and_basename() {
        // Arrange
        let root = scratch_dir("abspath-leaf");
        fs::create_dir_all(&root).unwrap();
        let leaf = root.join("leaf.txt");
        let expected_dir = fs::canonicalize(&root).unwrap();

        // Act
        let got = abspath(leaf.to_str().unwrap());

        // Assert
        assert_eq!(got, format!("{}/leaf.txt", expected_dir.to_string_lossy()));

        let _ = fs::remove_dir_all(&root);
    }

    /// Unix only: `std::os::unix::fs::symlink` has no portable equivalent,
    /// and Windows symlink creation needs elevation or developer mode.
    #[cfg(unix)]
    #[test]
    fn abspath_leaf_symlink_stays_unresolved() {
        // Arrange: a symlink whose target is a real file in the same
        // directory. The doc comment on `abspath` claims a leaf symlink is
        // returned as itself, not resolved to what it points at.
        let root = scratch_dir("abspath-symlink");
        fs::create_dir_all(&root).unwrap();
        let target = root.join("target.txt");
        fs::write(&target, "hello").unwrap();
        let link = root.join("link.txt");
        std::os::unix::fs::symlink(&target, &link).unwrap();
        let expected_dir = fs::canonicalize(&root).unwrap();

        // Act
        let got = abspath(link.to_str().unwrap());

        // Assert: the symlink's own basename survives, "target.txt" does not
        // appear anywhere in the result.
        assert_eq!(got, format!("{}/link.txt", expected_dir.to_string_lossy()));

        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn abspath_missing_parent_falls_back_to_input_unchanged() {
        // Arrange: neither `p` nor its parent directory exists, so both the
        // `Path::new(p).is_dir()` and `Path::new(dir).is_dir()` checks fail
        // and the function must fall through to the final `p.to_string()`
        // arm.
        let root = scratch_dir("abspath-missing-parent");
        let missing = root.join("does-not-exist").join("leaf.txt");

        // Act
        let got = abspath(missing.to_str().unwrap());

        // Assert
        assert_eq!(got, missing.to_string_lossy());
    }
}
