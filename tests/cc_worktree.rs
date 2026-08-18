// SPDX-FileCopyrightText: 2026 Igor Santos
// SPDX-License-Identifier: MIT

//! Tests for the worktree decision and path logic ported in WU-17.
//!
//! The `.env` guard gets the heaviest coverage because it is a secret-leak
//! control: an unignored `.env` copied into a fresh worktree is a file that
//! `git add` will stage. Both directions are asserted, since a guard proven
//! only to refuse could be refusing everything.

use playbook::cc::worktree::{self, ConflictAction, EnvCopy};
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::atomic::{AtomicU64, Ordering};

static COUNTER: AtomicU64 = AtomicU64::new(0);

fn scratch(tag: &str) -> PathBuf {
    let n = COUNTER.fetch_add(1, Ordering::Relaxed);
    let dir = std::env::temp_dir().join(format!("playbook-wt-{tag}-{}-{n}", std::process::id()));
    fs::create_dir_all(&dir).expect("scratch");
    dir
}

/// A real repo, because the guard asks git whether the path is ignored and a
/// stubbed answer would not exercise that.
fn repo(tag: &str) -> PathBuf {
    let dir = scratch(tag);
    for args in [
        vec!["init", "-q"],
        vec!["config", "user.email", "t@t"],
        vec!["config", "user.name", "T"],
    ] {
        Command::new("git")
            .arg("-C")
            .arg(&dir)
            .args(&args)
            .output()
            .expect("git");
    }
    dir
}

mod conflict_decision {
    use super::*;

    #[test]
    fn silent_mode_always_spawns() {
        assert_eq!(
            worktree::conflict_action(true, false, ""),
            ConflictAction::Spawn
        );
        assert_eq!(
            worktree::conflict_action(true, true, "n"),
            ConflictAction::Spawn
        );
    }

    /// A non-tty must abort rather than assume yes. The alternative is an
    /// unattended run rewriting someone's conflicts without consent.
    #[test]
    fn a_non_tty_aborts_even_on_an_affirmative_answer() {
        assert_eq!(
            worktree::conflict_action(false, false, "y"),
            ConflictAction::Abort
        );
        assert_eq!(
            worktree::conflict_action(false, false, ""),
            ConflictAction::Abort
        );
    }

    #[test]
    fn an_interactive_terminal_honours_the_answer() {
        for yes in ["", "y", "Y", "yes", "YES", " y "] {
            assert_eq!(
                worktree::conflict_action(false, true, yes),
                ConflictAction::Spawn,
                "{yes:?} should spawn"
            );
        }
        for no in ["n", "N", "no", "q", "anything"] {
            assert_eq!(
                worktree::conflict_action(false, true, no),
                ConflictAction::Abort,
                "{no:?} should abort"
            );
        }
    }
}

mod base_resolution {
    use super::*;

    #[test]
    fn a_relative_base_sits_under_the_repo_parent() {
        let base = worktree::resolve_base(
            Path::new("/work/myrepo"),
            Path::new("/work"),
            Some(".worktrees"),
        );
        assert_eq!(base, PathBuf::from("/work/.worktrees/myrepo"));
    }

    #[test]
    fn an_absolute_base_is_used_as_given() {
        let base = worktree::resolve_base(
            Path::new("/work/myrepo"),
            Path::new("/work"),
            Some("/tmp/trees"),
        );
        assert_eq!(base, PathBuf::from("/tmp/trees/myrepo"));
    }

    /// A base of "." would put worktrees inside the repo itself, so it falls
    /// back to the default instead.
    #[test]
    fn a_dot_base_never_collapses_into_the_repo_root() {
        for configured in [Some("."), None, Some("")] {
            let base =
                worktree::resolve_base(Path::new("/work/myrepo"), Path::new("/work"), configured);
            assert_eq!(
                base,
                PathBuf::from("/work/.worktrees/myrepo"),
                "{configured:?} should use the default"
            );
        }
    }

    /// The repo leaf is what keeps same-named branches in sibling repos apart.
    #[test]
    fn the_repo_name_is_the_final_component() {
        let a = worktree::resolve_base(Path::new("/w/alpha"), Path::new("/w"), None);
        let b = worktree::resolve_base(Path::new("/w/beta"), Path::new("/w"), None);
        assert_ne!(a, b);
    }
}

mod env_base_discovery {
    use super::*;

    #[test]
    fn a_root_env_is_reported_as_dot() {
        let dir = scratch("root-env");
        fs::write(dir.join(".env"), "TOKEN=x\n").expect("write");
        assert_eq!(worktree::find_env_base(&dir, None), Some(".".to_string()));
    }

    #[test]
    fn a_nested_env_is_found_one_level_down() {
        let dir = scratch("nested-env");
        fs::create_dir_all(dir.join("api")).expect("mkdir");
        fs::write(dir.join("api/.env"), "TOKEN=x\n").expect("write");
        assert_eq!(worktree::find_env_base(&dir, None), Some("api".to_string()));
    }

    /// Two levels down is out of range, matching the shell's maxdepth of 2.
    #[test]
    fn a_deeper_env_is_not_found() {
        let dir = scratch("deep-env");
        fs::create_dir_all(dir.join("a/b")).expect("mkdir");
        fs::write(dir.join("a/b/.env"), "TOKEN=x\n").expect("write");
        assert_eq!(worktree::find_env_base(&dir, None), None);
    }

    #[test]
    fn an_explicit_hint_is_honoured_and_verified() {
        let dir = scratch("hint");
        fs::create_dir_all(dir.join("svc")).expect("mkdir");
        fs::write(dir.join("svc/.env"), "TOKEN=x\n").expect("write");
        assert_eq!(
            worktree::find_env_base(&dir, Some("svc")),
            Some("svc".to_string())
        );
        // A hint pointing at nothing yields nothing rather than falling back.
        assert_eq!(worktree::find_env_base(&dir, Some("nope")), None);
    }

    #[test]
    fn no_env_anywhere_is_none() {
        let dir = scratch("no-env");
        assert_eq!(worktree::find_env_base(&dir, None), None);
    }
}

mod env_copy_guard {
    use super::*;

    /// The refusal case, and the reason the guard exists: a TRACKED .env copied
    /// into a worktree is a file `git add` will stage.
    #[test]
    fn an_unignored_env_is_refused_and_named() {
        let src = repo("unignored");
        let dest = scratch("unignored-dest");
        fs::write(src.join(".env"), "TOKEN=secret\n").expect("write");

        match worktree::copy_env(&src, &dest, Some(".")) {
            EnvCopy::RefusedNotGitignored(rel) => assert_eq!(rel, ".env"),
            other => panic!("expected a refusal, got {other:?}"),
        }
        assert!(
            !dest.join(".env").exists(),
            "nothing may be written when the copy is refused"
        );
    }

    /// The permit case. Without this the guard could pass while refusing
    /// everything, which would quietly break a working feature.
    #[test]
    fn a_gitignored_env_is_copied() {
        let src = repo("ignored");
        let dest = scratch("ignored-dest");
        fs::write(src.join(".gitignore"), ".env\n").expect("write");
        fs::write(src.join(".env"), "TOKEN=secret\n").expect("write");

        match worktree::copy_env(&src, &dest, Some(".")) {
            EnvCopy::Copied(path) => {
                assert_eq!(path, dest.join(".env"));
                assert_eq!(
                    fs::read_to_string(path).expect("read"),
                    "TOKEN=secret\n",
                    "the contents must arrive intact"
                );
            }
            other => panic!("expected a copy, got {other:?}"),
        }
    }

    #[test]
    fn a_nested_gitignored_env_is_copied_into_the_same_subdirectory() {
        let src = repo("nested-ignored");
        let dest = scratch("nested-dest");
        fs::create_dir_all(src.join("api")).expect("mkdir");
        fs::write(src.join(".gitignore"), "api/.env\n").expect("write");
        fs::write(src.join("api/.env"), "TOKEN=x\n").expect("write");

        match worktree::copy_env(&src, &dest, Some("api")) {
            EnvCopy::Copied(path) => assert_eq!(path, dest.join("api/.env")),
            other => panic!("expected a copy, got {other:?}"),
        }
    }

    /// No-clobber, matching `cp -n`: an existing destination file wins.
    #[test]
    fn an_existing_destination_env_is_not_overwritten() {
        let src = repo("noclobber");
        let dest = scratch("noclobber-dest");
        fs::write(src.join(".gitignore"), ".env\n").expect("write");
        fs::write(src.join(".env"), "FROM=source\n").expect("write");
        fs::write(dest.join(".env"), "FROM=destination\n").expect("write");

        worktree::copy_env(&src, &dest, Some("."));
        assert_eq!(
            fs::read_to_string(dest.join(".env")).expect("read"),
            "FROM=destination\n",
            "the existing file must survive"
        );
    }

    #[test]
    fn no_env_base_and_a_missing_source_are_both_quiet() {
        let src = repo("quiet");
        let dest = scratch("quiet-dest");
        assert_eq!(
            worktree::copy_env(&src, &dest, None),
            EnvCopy::NoEnvConfigured
        );
        assert_eq!(
            worktree::copy_env(&src, &dest, Some(".")),
            EnvCopy::SourceMissing
        );
    }

    /// Outside a repo, `check-ignore` cannot answer, and an unanswerable guard
    /// must refuse rather than assume the file is safe to copy.
    #[test]
    fn a_non_repo_source_refuses_the_copy() {
        let src = scratch("not-a-repo");
        let dest = scratch("not-a-repo-dest");
        fs::write(src.join(".env"), "TOKEN=x\n").expect("write");
        match worktree::copy_env(&src, &dest, Some(".")) {
            EnvCopy::RefusedNotGitignored(_) => {}
            other => panic!("expected a refusal outside a repo, got {other:?}"),
        }
    }
}
