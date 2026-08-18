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

/// GIT_CONFIG_GLOBAL/GIT_CONFIG_SYSTEM are process-wide, and cargo runs the
/// tests in this binary on parallel threads, so mutating them needs one
/// shared lock rather than a per-module one.
static ENV_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

fn lock_env() -> std::sync::MutexGuard<'static, ()> {
    ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner())
}

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

mod base_branch_detection {
    use super::*;

    /// A remote-tracking ref, created without a network by pointing a local ref
    /// at a commit. `origin/HEAD` is what a real clone gets from the server.
    fn with_remote_refs(tag: &str, publish_head: Option<&str>, branches: &[&str]) -> PathBuf {
        let dir = repo(tag);
        fs::write(dir.join("f.txt"), "x").expect("write");
        for args in [vec!["add", "f.txt"], vec!["commit", "-qm", "init"]] {
            Command::new("git")
                .arg("-C")
                .arg(&dir)
                .args(&args)
                .output()
                .expect("git");
        }
        let sha = String::from_utf8_lossy(
            &Command::new("git")
                .arg("-C")
                .arg(&dir)
                .args(["rev-parse", "HEAD"])
                .output()
                .expect("git")
                .stdout,
        )
        .trim()
        .to_string();

        for branch in branches {
            Command::new("git")
                .arg("-C")
                .arg(&dir)
                .args(["update-ref", &format!("refs/remotes/origin/{branch}"), &sha])
                .output()
                .expect("git");
        }
        if let Some(head) = publish_head {
            Command::new("git")
                .arg("-C")
                .arg(&dir)
                .args([
                    "symbolic-ref",
                    "refs/remotes/origin/HEAD",
                    &format!("refs/remotes/origin/{head}"),
                ])
                .output()
                .expect("git");
        }
        dir
    }

    /// What the remote publishes wins, so a repo whose default is `trunk` is
    /// never silently treated as `main`.
    #[test]
    fn the_published_head_wins_over_the_candidates() {
        let dir = with_remote_refs("published", Some("trunk"), &["main", "trunk"]);
        assert_eq!(worktree::base_branch(&dir), "origin/trunk");
    }

    #[test]
    fn without_a_published_head_the_candidates_are_tried_in_order() {
        let dir = with_remote_refs("candidates", None, &["develop", "master"]);
        assert_eq!(
            worktree::base_branch(&dir),
            "origin/master",
            "master precedes develop in the candidate order"
        );
    }

    #[test]
    fn a_repo_with_no_remote_refs_falls_back() {
        let dir = repo("no-remote");
        assert_eq!(worktree::base_branch(&dir), "origin/master");
    }
}

mod staleness_decision {
    use super::*;
    use playbook::cc::worktree::WorktreeStatus;

    const NOW: i64 = 1_800_000_000;
    const DAY: i64 = 86_400;

    fn status(path: &Path) -> WorktreeStatus<'_> {
        WorktreeStatus {
            path,
            branch: "feature",
            is_target: false,
            in_use: false,
            has_open_pr: false,
            merged_into_base: false,
            last_commit_epoch: NOW,
        }
    }

    #[test]
    fn a_merged_branch_is_stale() {
        let p = Path::new("/tmp/wt");
        let s = WorktreeStatus {
            merged_into_base: true,
            ..status(p)
        };
        assert!(worktree::is_stale(&s, NOW));
    }

    #[test]
    fn an_old_unmerged_branch_is_stale_and_a_recent_one_is_not() {
        let p = Path::new("/tmp/wt");
        let old = WorktreeStatus {
            last_commit_epoch: NOW - 31 * DAY,
            ..status(p)
        };
        assert!(worktree::is_stale(&old, NOW), "31 days is past the cutoff");

        let recent = WorktreeStatus {
            last_commit_epoch: NOW - 29 * DAY,
            ..status(p)
        };
        assert!(!worktree::is_stale(&recent, NOW), "29 days is inside it");
    }

    /// Each skip is absolute and beats both staleness reasons. An old branch
    /// with an open pull request is still wanted, which is the case that would
    /// hurt most if the ordering were wrong.
    #[test]
    fn every_skip_beats_both_staleness_reasons() {
        let p = Path::new("/tmp/wt");
        let base = WorktreeStatus {
            merged_into_base: true,
            last_commit_epoch: NOW - 400 * DAY,
            ..status(p)
        };

        for (label, s) in [
            (
                "target",
                WorktreeStatus {
                    is_target: true,
                    ..base
                },
            ),
            (
                "in use",
                WorktreeStatus {
                    in_use: true,
                    ..base
                },
            ),
            (
                "open PR",
                WorktreeStatus {
                    has_open_pr: true,
                    ..base
                },
            ),
            ("no branch", WorktreeStatus { branch: "", ..base }),
        ] {
            assert!(
                !worktree::is_stale(&s, NOW),
                "{label} must never be reaped, even when merged and ancient"
            );
        }
    }
}

mod cleanup_rate_limit {
    use super::*;

    const NOW: i64 = 1_800_000_000;

    /// A repo that has never been cleaned should be.
    #[test]
    fn an_absent_marker_means_due() {
        assert!(worktree::cleanup_due(None, NOW));
    }

    #[test]
    fn it_runs_at_most_once_a_day() {
        assert!(
            !worktree::cleanup_due(Some(NOW - 3600), NOW),
            "an hour ago is too soon"
        );
        assert!(
            !worktree::cleanup_due(Some(NOW - 86_399), NOW),
            "just under a day is too soon"
        );
        assert!(
            worktree::cleanup_due(Some(NOW - 86_400), NOW),
            "exactly a day is due"
        );
        assert!(worktree::cleanup_due(Some(NOW - 200_000), NOW));
    }
}

mod node_modules_reuse {
    use super::*;

    /// A source checkout with an installed node_modules and a lockfile.
    fn source(tag: &str, lock: &str) -> PathBuf {
        let dir = scratch(tag);
        fs::write(dir.join("package-lock.json"), lock).expect("write");
        fs::create_dir_all(dir.join("node_modules/dep")).expect("mkdir");
        dir
    }

    fn worktree(tag: &str, lock: Option<&str>) -> PathBuf {
        let dir = scratch(tag);
        fs::write(dir.join("package.json"), "{}").expect("write");
        if let Some(lock) = lock {
            fs::write(dir.join("package-lock.json"), lock).expect("write");
        }
        dir
    }

    #[test]
    fn matching_lockfiles_allow_reuse() {
        let src = source("src-match", "{\"v\":1}");
        let wt = worktree("wt-match", Some("{\"v\":1}"));
        assert!(worktree::node_modules_reusable(&wt, &src));
    }

    /// The whole point of the check: differing lockfiles mean the installed
    /// tree is wrong for this worktree, so reusing it would ship stale deps.
    #[test]
    fn differing_lockfiles_block_reuse() {
        let src = source("src-diff", "{\"v\":1}");
        let wt = worktree("wt-diff", Some("{\"v\":2}"));
        assert!(!worktree::node_modules_reusable(&wt, &src));
    }

    /// A worktree with no lockfile of its own inherits the source's, which is
    /// the copy the shell made before comparing.
    #[test]
    fn a_worktree_without_a_lockfile_inherits_and_matches() {
        let src = source("src-inherit", "{\"v\":1}");
        let wt = worktree("wt-inherit", None);
        assert!(worktree::node_modules_reusable(&wt, &src));
    }

    #[test]
    fn a_non_node_worktree_is_skipped() {
        let src = source("src-nonnode", "{\"v\":1}");
        let wt = scratch("wt-nonnode");
        assert!(
            !worktree::node_modules_reusable(&wt, &src),
            "no package.json means this is not a node project"
        );
    }

    #[test]
    fn a_source_without_installed_modules_is_skipped() {
        let src = scratch("src-bare");
        fs::write(src.join("package-lock.json"), "{\"v\":1}").expect("write");
        let wt = worktree("wt-bare", Some("{\"v\":1}"));
        assert!(
            !worktree::node_modules_reusable(&wt, &src),
            "there is nothing to reuse without an installed node_modules"
        );
    }

    #[test]
    fn a_source_without_a_lockfile_is_skipped() {
        let src = scratch("src-nolock");
        fs::create_dir_all(src.join("node_modules")).expect("mkdir");
        let wt = worktree("wt-nolock", None);
        assert!(!worktree::node_modules_reusable(&wt, &src));
    }
}

/// `reuse_node_modules`: the imperative half (worktree.sh:174-197). Helpers
/// are duplicated rather than shared with `node_modules_reuse` above,
/// per-binary test compilation makes that the simpler option.
mod node_modules_apply {
    use super::*;

    /// A source checkout with an installed node_modules and a lockfile. The
    /// installed content is a lone top-level file rather than a package-like
    /// subdirectory: a real `npm install`, if one happens to run during
    /// `reuse_node_modules`, reconciles node_modules against package.json and
    /// prunes any subdirectory it does not recognise as a declared
    /// dependency, which would make an assertion on copied content depend on
    /// whichever npm happens to be on the machine running this suite.
    fn source(tag: &str, lock: &str) -> PathBuf {
        let dir = scratch(tag);
        fs::write(dir.join("package-lock.json"), lock).expect("write");
        fs::create_dir_all(dir.join("node_modules")).expect("mkdir");
        fs::write(dir.join("node_modules/marker.txt"), "source-content").expect("write");
        dir
    }

    fn worktree(tag: &str, lock: Option<&str>) -> PathBuf {
        let dir = scratch(tag);
        fs::write(dir.join("package.json"), "{}").expect("write");
        if let Some(lock) = lock {
            fs::write(dir.join("package-lock.json"), lock).expect("write");
        }
        dir
    }

    /// Hides `npm` from `PATH` for the duration of `f`, so the best-effort
    /// refresh `reuse_node_modules` runs after a successful copy
    /// (worktree.sh:196) never actually executes. Real npm's behaviour, its
    /// presence, its version, and network access are not what is under test
    /// here; only the copy ladder is. `/bin:/usr/bin` still resolves `cp`,
    /// which the ladder does need.
    fn with_npm_hidden<T>(f: impl FnOnce() -> T) -> T {
        let _guard = lock_env();
        let prev = std::env::var_os("PATH");
        std::env::set_var("PATH", "/bin:/usr/bin");
        let out = f();
        match prev {
            Some(v) => std::env::set_var("PATH", v),
            None => std::env::remove_var("PATH"),
        }
        out
    }

    /// The load-bearing case: a worktree with no lockfile of its own gets one
    /// seeded (worktree.sh:184), and only then does the copy proceed.
    #[test]
    fn a_missing_worktree_lockfile_is_seeded_and_reuse_proceeds() {
        with_npm_hidden(|| {
            let src = source("apply-seed-src", "{\"v\":1}");
            let wt = worktree("apply-seed-wt", None);

            let copied = worktree::reuse_node_modules(&wt, &src);

            assert!(
                copied,
                "a freshly seeded lockfile matches the source by construction"
            );
            assert_eq!(
                fs::read(wt.join("package-lock.json")).expect("lockfile should have been seeded"),
                fs::read(src.join("package-lock.json")).expect("source lockfile")
            );
            assert_eq!(
                fs::read(wt.join("node_modules/marker.txt")).expect("content should be copied"),
                b"source-content"
            );
        });
    }

    /// Proves the destructive step happens only after the decision: a
    /// mismatch refuses the reuse, and the worktree's existing node_modules
    /// is never touched to get there.
    #[test]
    fn a_differing_lockfile_refuses_reuse_and_leaves_existing_node_modules_untouched() {
        let src = source("apply-diff-src", "{\"v\":1}");
        let wt = worktree("apply-diff-wt", Some("{\"v\":2}"));
        fs::create_dir_all(wt.join("node_modules")).expect("mkdir");
        fs::write(wt.join("node_modules/keep.txt"), "keep-me").expect("write");

        let copied = worktree::reuse_node_modules(&wt, &src);

        assert!(!copied);
        assert_eq!(
            fs::read(wt.join("node_modules/keep.txt")).expect("existing content should survive"),
            b"keep-me"
        );
    }

    /// An existing `node_modules` directory is discarded and replaced with a
    /// copy of the source's.
    #[test]
    fn an_existing_node_modules_directory_is_replaced_with_the_sources_content() {
        with_npm_hidden(|| {
            let src = source("apply-replace-src", "{\"v\":1}");
            let wt = worktree("apply-replace-wt", Some("{\"v\":1}"));
            fs::create_dir_all(wt.join("node_modules")).expect("mkdir");
            fs::write(wt.join("node_modules/stale.txt"), "stale").expect("write");

            let copied = worktree::reuse_node_modules(&wt, &src);

            assert!(copied);
            assert!(
                !wt.join("node_modules/stale.txt").exists(),
                "the old tree should have been discarded, not merged into"
            );
            assert_eq!(
                fs::read(wt.join("node_modules/marker.txt")).expect("content should match source"),
                fs::read(src.join("node_modules/marker.txt")).expect("source content")
            );
        });
    }

    /// Shadows `PATH` with a `cp` shell script for the duration of `f`, and
    /// restores it afterwards. Guarded by `lock_env` since `PATH` is
    /// process-wide. `/bin:/usr/bin` stays reachable behind the stub, so a
    /// script that `exec`s the real `/bin/cp` still works, and so `npm`
    /// remains unreachable the way `with_npm_hidden` also arranges.
    #[cfg(unix)]
    fn with_stub_cp<T>(tag: &str, script_body: &str, f: impl FnOnce() -> T) -> T {
        use std::os::unix::fs::PermissionsExt;

        let _guard = lock_env();
        let fakebin = scratch(tag);
        let script = fakebin.join("cp");
        fs::write(&script, script_body).expect("write fake cp");
        let mut perms = fs::metadata(&script).expect("metadata").permissions();
        perms.set_mode(0o755);
        fs::set_permissions(&script, perms).expect("chmod");

        let prev_path = std::env::var_os("PATH");
        let mut fake_path = fakebin.into_os_string();
        fake_path.push(":/bin:/usr/bin");
        std::env::set_var("PATH", &fake_path);

        let out = f();

        match prev_path {
            Some(v) => std::env::set_var("PATH", v),
            None => std::env::remove_var("PATH"),
        }
        out
    }

    /// The shell repeats `rm -rf node_modules` before every retry, not just
    /// once (worktree.sh:192-194), because a `cp` that fails partway can
    /// still leave a partial destination behind (per `man cp`, `-R` "will
    /// continue copying even if errors are detected"). The real `cp -c` on
    /// this machine falls back to a plain copy internally on failure and so
    /// essentially never fails outright, which means the fallback tiers are
    /// unreachable with a real `cp`. This stub stands in: tier 1 always
    /// fails after creating an empty destination, and tier 2 always succeeds
    /// for real, so whether the content lands at the right depth depends
    /// entirely on whether that empty destination was removed first. Left
    /// uncleaned, `cp -R` copies INTO an existing directory rather than
    /// replacing it, nesting the result one level too deep.
    #[cfg(unix)]
    #[test]
    fn the_ladder_removes_a_failed_attempts_partial_result_before_retrying() {
        let copied = with_stub_cp(
            "apply-ladder-fakebin",
            "#!/bin/sh\n\
             if [ \"$1\" = \"-cR\" ]; then\n\
             \tmkdir -p \"$3\"\n\
             \texit 1\n\
             elif [ \"$1\" = \"-R\" ] && [ \"$2\" = \"--reflink=auto\" ]; then\n\
             \texec /bin/cp -R \"$3\" \"$4\"\n\
             elif [ \"$1\" = \"-R\" ]; then\n\
             \texec /bin/cp -R \"$2\" \"$3\"\n\
             fi\n\
             exit 1\n",
            || {
                let src = source("apply-ladder-src", "{\"v\":1}");
                let wt = worktree("apply-ladder-wt", Some("{\"v\":1}"));
                let copied = worktree::reuse_node_modules(&wt, &src);
                assert_eq!(
                    fs::read(wt.join("node_modules/marker.txt")).expect(
                        "content should be directly under node_modules, not nested under a \
                         leftover tier-1 directory"
                    ),
                    b"source-content"
                );
                copied
            },
        );

        assert!(
            copied,
            "tier 2 should still succeed for real once tier 1's partial result is out of the way"
        );
    }

    /// The shell's `[[ -e ]]`, not `[[ -d ]]` (worktree.sh:191): a plain FILE
    /// named `node_modules` is removed too, not just a stray directory.
    ///
    /// A real `cp -cR` refuses to write over an existing file regardless of
    /// whether `reuse_node_modules` cleared it first, and the ladder's own
    /// between-retries cleanup then quietly repairs a missed precondition
    /// check on the very next tier, so a real `cp` cannot tell these two
    /// implementations apart. This stub can: it "succeeds" without copying
    /// anything whenever the destination already exists, so the file
    /// surviving into tier 1 shows up directly as a `node_modules` that is
    /// still a plain file instead of the copied directory.
    #[cfg(unix)]
    #[test]
    fn a_file_named_node_modules_is_removed_and_replaced_by_the_copy() {
        let (copied, is_dir, content) = with_stub_cp(
            "apply-file-fakebin",
            "#!/bin/sh\n\
             if [ \"$1\" = \"-cR\" ]; then\n\
             \tif [ -e \"$3\" ]; then\n\
             \t\texit 0\n\
             \tfi\n\
             \texec /bin/cp -R \"$2\" \"$3\"\n\
             fi\n\
             exit 1\n",
            || {
                let src = source("apply-file-src", "{\"v\":1}");
                let wt = worktree("apply-file-wt", Some("{\"v\":1}"));
                fs::write(wt.join("node_modules"), "not a directory").expect("write");

                let copied = worktree::reuse_node_modules(&wt, &src);
                let dest = wt.join("node_modules");
                (copied, dest.is_dir(), fs::read(dest.join("marker.txt")))
            },
        );

        assert!(copied);
        assert!(
            is_dir,
            "the file should have been removed before the copy ladder ever ran"
        );
        assert_eq!(
            content.expect("content should match source"),
            b"source-content"
        );
    }

    /// A worktree with no `package.json` at all fails the very first
    /// precondition. This is also the case that would break under a literal
    /// seed-then-decide port: deciding first means this worktree is never
    /// written to at all, matching the shell, which returns before ever
    /// reaching its own seed line.
    #[test]
    fn missing_preconditions_return_false_and_change_nothing_on_disk() {
        let src = source("apply-missing-src", "{\"v\":1}");
        let wt = scratch("apply-missing-wt");

        let copied = worktree::reuse_node_modules(&wt, &src);

        assert!(!copied);
        assert!(
            !wt.join("package-lock.json").exists(),
            "nothing should be seeded into a non-node worktree"
        );
        assert!(!wt.join("node_modules").exists());
    }

    #[test]
    fn restore_stash_does_nothing_when_no_stash_was_applied() {
        let main = scratch("apply-restore-stash-false");
        fs::write(main.join("marker.txt"), "untouched").expect("write");

        worktree::restore_stash(&main, false);

        assert_eq!(
            fs::read(main.join("marker.txt")).expect("marker should survive"),
            b"untouched"
        );
        let entries: Vec<_> = fs::read_dir(&main).expect("read_dir").collect();
        assert_eq!(entries.len(), 1, "nothing else should have been created");
    }
}

mod worktree_lookup {
    use super::*;

    const PORCELAIN: &str = "worktree /repo\nHEAD abc\nbranch refs/heads/main\n\
                             \nworktree /repo/.worktrees/feat\nHEAD def\n\
                             branch refs/heads/feature-x\n\
                             \nworktree /repo/.worktrees/detached\nHEAD 999\ndetached\n";

    #[test]
    fn finds_the_worktree_holding_a_branch() {
        assert_eq!(
            worktree::worktree_for_branch(PORCELAIN, "feature-x").as_deref(),
            Some("/repo/.worktrees/feat")
        );
        assert_eq!(
            worktree::worktree_for_branch(PORCELAIN, "main").as_deref(),
            Some("/repo")
        );
    }

    #[test]
    fn an_unheld_branch_is_none() {
        assert!(worktree::worktree_for_branch(PORCELAIN, "nope").is_none());
    }

    /// A prefix must not match: `feat` is not `feature-x`, and returning the
    /// wrong worktree would send the recovery path somewhere unrelated.
    #[test]
    fn a_branch_prefix_does_not_match() {
        assert!(worktree::worktree_for_branch(PORCELAIN, "feat").is_none());
        assert!(worktree::worktree_for_branch(PORCELAIN, "ain").is_none());
    }

    #[test]
    fn a_detached_worktree_is_never_returned() {
        assert!(worktree::worktree_for_branch(PORCELAIN, "detached").is_none());
        assert!(worktree::worktree_for_branch(PORCELAIN, "999").is_none());
    }

    #[test]
    fn empty_input_is_none() {
        assert!(worktree::worktree_for_branch("", "main").is_none());
    }
}

mod recovery_plan {
    use playbook::cc::worktree::{plan_for_target, TargetState, WorktreePlan};

    fn occupied<'a>(current: &'a str, on_remote: bool) -> TargetState<'a> {
        TargetState {
            target_exists: true,
            registered: true,
            current_branch: Some(current),
            wanted_branch: "wanted",
            current_branch_on_remote: on_remote,
            existing_for_wanted: None,
        }
    }

    #[test]
    fn a_target_already_on_the_wanted_branch_is_reused() {
        let s = TargetState {
            current_branch: Some("wanted"),
            ..occupied("wanted", true)
        };
        assert_eq!(plan_for_target(&s), WorktreePlan::ReuseTarget);
    }

    /// The guard that protects unfinished work. Recycling removes the worktree
    /// AND deletes its branch, so a branch still on the remote must stop the
    /// operation. This is the pairing the blueprint asks to pin.
    #[test]
    fn an_occupying_branch_still_on_the_remote_is_refused_not_recycled() {
        let s = occupied("other-feature", true);
        assert_eq!(
            plan_for_target(&s),
            WorktreePlan::RefuseOccupied("other-feature".to_string()),
            "a branch that still exists remotely must never be destroyed"
        );
    }

    #[test]
    fn an_occupying_branch_gone_from_the_remote_is_recycled() {
        let s = occupied("merged-feature", false);
        assert_eq!(
            plan_for_target(&s),
            WorktreePlan::RecycleTarget("merged-feature".to_string())
        );
    }

    #[test]
    fn a_detached_registered_worktree_is_recovered() {
        let s = TargetState {
            current_branch: None,
            ..occupied("x", false)
        };
        assert_eq!(plan_for_target(&s), WorktreePlan::RecoverDetached);
    }

    /// A directory git does not know about is not a worktree, so recovering it
    /// makes no sense; it is cleaned and rebuilt.
    #[test]
    fn an_unregistered_directory_is_treated_as_an_orphan() {
        let s = TargetState {
            current_branch: None,
            registered: false,
            ..occupied("x", false)
        };
        assert_eq!(plan_for_target(&s), WorktreePlan::CleanOrphanAndCreate);
    }

    #[test]
    fn an_absent_target_creates_or_reuses_an_existing_worktree() {
        let base = TargetState {
            target_exists: false,
            registered: false,
            current_branch: None,
            wanted_branch: "wanted",
            current_branch_on_remote: false,
            existing_for_wanted: None,
        };
        assert_eq!(plan_for_target(&base), WorktreePlan::Create);

        let reuse = TargetState {
            existing_for_wanted: Some(("/repo/.worktrees/elsewhere", true)),
            ..base
        };
        assert_eq!(
            plan_for_target(&reuse),
            WorktreePlan::ReuseExisting("/repo/.worktrees/elsewhere".to_string())
        );

        // Registered but the directory is gone: prune the stale entry first,
        // since git refuses to add a worktree for a branch it thinks is taken.
        let stale = TargetState {
            existing_for_wanted: Some(("/repo/.worktrees/gone", false)),
            ..base
        };
        assert_eq!(plan_for_target(&stale), WorktreePlan::PruneStaleAndCreate);
    }

    /// Detachment is checked before the branch comparison, so a detached
    /// worktree is recovered rather than mistaken for a mismatch and recycled.
    #[test]
    fn detachment_is_decided_before_any_branch_comparison() {
        let s = TargetState {
            current_branch: None,
            registered: true,
            current_branch_on_remote: true,
            ..occupied("ignored", true)
        };
        assert_eq!(plan_for_target(&s), WorktreePlan::RecoverDetached);
    }
}

mod folder_naming {
    use playbook::cc::worktree::{branch_leaf, folder_for_branch, jira_key, sanitize_branch};

    /// A carriage return survives a paste from a Windows-authored ticket or a
    /// CI variable, and git then rejects a branch that looks identical to the
    /// one asked for.
    #[test]
    fn sanitize_strips_carriage_returns_and_whitespace() {
        assert_eq!(sanitize_branch("  feat/x  "), "feat/x");
        assert_eq!(sanitize_branch("feat/x\r"), "feat/x");
        assert_eq!(sanitize_branch("\r\n feat/x \r\n"), "feat/x");
        assert_eq!(sanitize_branch("feat/x"), "feat/x");
    }

    #[test]
    fn a_jira_key_is_found_anywhere_and_uppercased() {
        assert_eq!(jira_key("feature/PROJ-123-add"), Some("PROJ-123".into()));
        assert_eq!(jira_key("PROJ-123"), Some("PROJ-123".into()));
        assert_eq!(
            jira_key("fix/abc-45"),
            Some("ABC-45".into()),
            "case insensitive"
        );
        assert_eq!(jira_key("x/AB-1/y"), Some("AB-1".into()));
    }

    #[test]
    fn the_first_key_wins_when_several_appear() {
        assert_eq!(jira_key("PROJ-1-then-OTHER-2"), Some("PROJ-1".into()));
    }

    /// A single leading letter is not a project key, so `a-1` must not be
    /// treated as one and collapse unrelated branches into one folder.
    #[test]
    fn near_misses_are_not_keys() {
        for branch in [
            "a-1",
            "main",
            "feature/no-digits-here",
            "PROJ-",
            "-123",
            "PROJ123",
        ] {
            assert_eq!(jira_key(branch), None, "{branch} should not be a key");
        }
    }

    #[test]
    fn the_leaf_is_everything_after_the_last_slash() {
        assert_eq!(branch_leaf("feat/deep/name"), "name");
        assert_eq!(branch_leaf("flat"), "flat");
        assert_eq!(branch_leaf("trailing/"), "");
    }

    /// One ticket, one worktree: two branches carrying the same key share a
    /// folder, which is the behaviour the naming exists for.
    #[test]
    fn branches_sharing_a_key_share_a_folder() {
        assert_eq!(folder_for_branch("feature/PROJ-1/spike", None), "PROJ-1");
        assert_eq!(folder_for_branch("fix/PROJ-1", None), "PROJ-1");
    }

    #[test]
    fn a_keyless_branch_uses_its_leaf() {
        assert_eq!(folder_for_branch("feature/add-thing", None), "add-thing");
        assert_eq!(folder_for_branch("hotfix", None), "hotfix");
    }

    /// When the key's folder already holds a DIFFERENT branch, falling back to
    /// the leaf avoids two branches fighting over one directory.
    #[test]
    fn an_occupied_key_folder_falls_back_to_the_leaf() {
        assert_eq!(
            folder_for_branch("feature/PROJ-1/second", Some("feature/PROJ-1/first")),
            "second"
        );
    }

    /// Occupied by the SAME branch is not a conflict, it is a resume, so the
    /// key folder is kept rather than a second one created alongside it.
    #[test]
    fn a_folder_occupied_by_the_same_branch_keeps_the_key() {
        assert_eq!(
            folder_for_branch("feature/PROJ-1", Some("feature/PROJ-1")),
            "PROJ-1"
        );
    }
}

mod rebase_eligibility {
    use playbook::cc::worktree::{should_rebase, RebaseContext};

    fn mine(branch: &str) -> RebaseContext<'_> {
        RebaseContext {
            current_branch: branch,
            git_user: "Igor",
            branch_author: "Igor",
            gh_user: "igorjs",
            wanted_branch: branch,
            base_ref: "main",
        }
    }

    #[test]
    fn a_branch_i_authored_may_be_rebased() {
        assert!(should_rebase(&mine("feature/x")));
    }

    /// The hole this list closes: without it, authoring the last commit on
    /// `develop` would make the heuristic treat shared history as personal and
    /// rebase it onto base.
    #[test]
    fn protected_branches_are_never_rebased_even_when_i_authored_them() {
        for branch in [
            "main",
            "master",
            "trunk",
            "develop",
            "dev",
            "staging",
            "release",
            "hotfix",
            "release/2.0",
            "hotfix/urgent",
        ] {
            assert!(
                !should_rebase(&mine(branch)),
                "{branch} is shared history and must never auto-rebase"
            );
        }
    }

    /// Protection is checked before ownership, so every ownership signal being
    /// true cannot override it.
    #[test]
    fn protection_outranks_every_ownership_signal() {
        let ctx = RebaseContext {
            current_branch: "release/1.0",
            git_user: "Igor",
            branch_author: "Igor",
            gh_user: "igorjs",
            wanted_branch: "feature/igorjs/release/1.0",
            base_ref: "main",
        };
        assert!(!should_rebase(&ctx));
    }

    #[test]
    fn someone_elses_branch_is_left_alone() {
        let ctx = RebaseContext {
            branch_author: "Someone Else",
            gh_user: "igorjs",
            wanted_branch: "feature/theirs",
            ..mine("feature/theirs")
        };
        assert!(
            !should_rebase(&ctx),
            "rebasing rewrites history, so decline"
        );
    }

    /// A login embedded in the branch name is the second ownership signal, for
    /// branches created by tooling that does not set the commit author.
    #[test]
    fn a_login_in_the_branch_name_counts_as_ownership() {
        let ctx = RebaseContext {
            branch_author: "Someone Else",
            wanted_branch: "feature/igorjs/thing",
            ..mine("feature/igorjs/thing")
        };
        assert!(should_rebase(&ctx));
    }

    /// An empty signal must never match. With an unset git user, an empty
    /// author would otherwise compare equal and claim every branch.
    #[test]
    fn empty_identity_signals_never_match() {
        let no_git_user = RebaseContext {
            git_user: "",
            branch_author: "",
            gh_user: "",
            ..mine("feature/x")
        };
        assert!(!should_rebase(&no_git_user));

        let no_gh_user = RebaseContext {
            git_user: "Igor",
            branch_author: "Someone Else",
            gh_user: "",
            ..mine("feature/x")
        };
        assert!(
            !should_rebase(&no_gh_user),
            "an empty login is in every string"
        );
    }

    #[test]
    fn base_and_detached_head_are_skipped() {
        assert!(!should_rebase(&mine("main")));
        let detached = RebaseContext {
            current_branch: "HEAD",
            ..mine("feature/x")
        };
        assert!(!should_rebase(&detached));
    }

    /// Ported quirk: the shell's `case` matches case-sensitively and the script
    /// sets no `nocasematch`, so `Main` is an ordinary branch there. Normalising
    /// case here would protect branches the shell rebases.
    #[test]
    fn protection_is_case_sensitive_like_the_shell() {
        for branch in ["Main", "MAIN", "Develop", "Release/2.0"] {
            assert!(
                should_rebase(&mine(branch)),
                "{branch} is not protected in the shell, so it must not be here"
            );
        }
    }

    /// Ported quirk: the shell's login test is `[[ "$BRANCH" == *"$gh_user"* ]]`,
    /// and quoting the expansion inside the pattern makes it literal, so a login
    /// containing glob metacharacters never matches as a wildcard.
    #[test]
    fn a_login_with_glob_metacharacters_is_matched_literally() {
        let ctx = RebaseContext {
            branch_author: "Someone Else",
            gh_user: "a*b",
            wanted_branch: "feature/aYYb",
            ..mine("feature/aYYb")
        };
        assert!(!should_rebase(&ctx), "the pattern is literal, not a glob");

        let literal = RebaseContext {
            branch_author: "Someone Else",
            gh_user: "a*b",
            wanted_branch: "feature/a*b",
            ..mine("feature/a*b")
        };
        assert!(should_rebase(&literal));
    }

    /// The shell's second signal reads `$BRANCH`, the branch that was asked for,
    /// not the one currently checked out. Collapsing the two fields would change
    /// who owns a branch.
    #[test]
    fn ownership_by_login_reads_the_wanted_branch_not_the_current_one() {
        let ctx = RebaseContext {
            current_branch: "feature/anon",
            branch_author: "Someone Else",
            wanted_branch: "feature/igorjs/thing",
            ..mine("feature/anon")
        };
        assert!(should_rebase(&ctx));
    }
}

mod upstream {
    use playbook::cc::worktree::{upstream_action, UpstreamAction};

    #[test]
    fn correct_tracking_is_left_alone() {
        assert_eq!(
            upstream_action(Some("origin/feat"), "origin/feat", true, false),
            UpstreamAction::None
        );
    }

    #[test]
    fn an_existing_remote_branch_is_tracked_without_pushing() {
        assert_eq!(
            upstream_action(None, "origin/feat", true, false),
            UpstreamAction::SetTracking
        );
        assert_eq!(
            upstream_action(Some("origin/wrong"), "origin/feat", true, false),
            UpstreamAction::SetTracking
        );
    }

    #[test]
    fn an_absent_remote_branch_is_pushed_unless_no_push() {
        assert_eq!(
            upstream_action(None, "origin/feat", false, false),
            UpstreamAction::PushAndTrack
        );
        assert_eq!(
            upstream_action(None, "origin/feat", false, true),
            UpstreamAction::SkipNoPush
        );
    }

    /// no_push must not suppress plain tracking, which touches nothing remote.
    #[test]
    fn no_push_only_suppresses_the_push() {
        assert_eq!(
            upstream_action(None, "origin/feat", true, true),
            UpstreamAction::SetTracking
        );
    }
}

/// `worktree_add_args`: the shell builds `[-b <new_branch>] <dest> <ref>`
/// once, then only prepends `-f` on the retry, so `-f` must land BEFORE `-b`.
mod worktree_add_args_ordering {
    use super::*;
    use std::ffi::OsString;

    fn args(strs: &[&str]) -> Vec<OsString> {
        strs.iter().map(OsString::from).collect()
    }

    #[test]
    fn no_branch_no_force_is_just_dest_and_ref() {
        assert_eq!(
            worktree::worktree_add_args(Path::new("/wt/dest"), "origin/main", None, false),
            args(&["/wt/dest", "origin/main"])
        );
    }

    #[test]
    fn a_new_branch_without_force_puts_dash_b_first() {
        assert_eq!(
            worktree::worktree_add_args(Path::new("/wt/dest"), "origin/main", Some("feat"), false),
            args(&["-b", "feat", "/wt/dest", "origin/main"])
        );
    }

    #[test]
    fn force_without_a_new_branch_puts_dash_f_first() {
        assert_eq!(
            worktree::worktree_add_args(Path::new("/wt/dest"), "origin/main", None, true),
            args(&["-f", "/wt/dest", "origin/main"])
        );
    }

    /// The order that matters most: `-f` before `-b`, matching the shell's
    /// `git worktree add -f "${cmd_args[@]}"` where cmd_args already starts
    /// with `-b`. Swapping them changes nothing about how git parses the
    /// command today, but a line-for-line port exists so that is not left to
    /// chance.
    #[test]
    fn force_with_a_new_branch_puts_dash_f_before_dash_b() {
        assert_eq!(
            worktree::worktree_add_args(Path::new("/wt/dest"), "origin/main", Some("feat"), true),
            args(&["-f", "-b", "feat", "/wt/dest", "origin/main"])
        );
    }
}

/// `fallback_lookup_branch`: the shell's `refs/heads/${new_branch:-$ref}`
/// (worktree.sh:113). A quirk, not a bug: without a new branch it looks up the
/// REFERENCE ITSELF as a local branch name, which only resolves when the ref
/// happens to be one.
mod fallback_branch_quirk {
    use super::*;

    #[test]
    fn a_new_branch_is_looked_up_over_the_reference() {
        assert_eq!(
            worktree::fallback_lookup_branch("origin/main", Some("feat")),
            "feat"
        );
    }

    /// The quirk itself: no new branch means the reference is looked up as a
    /// local branch name, even though `origin/main` never was one. Preserved
    /// because the shell does the same thing, not because it is correct.
    #[test]
    fn no_new_branch_falls_back_to_the_reference_even_when_it_is_not_a_local_branch_name() {
        assert_eq!(
            worktree::fallback_lookup_branch("origin/main", None),
            "origin/main"
        );
    }

    /// The other direction: when the reference DOES happen to name a local
    /// branch, the quirk resolves correctly by coincidence.
    #[test]
    fn no_new_branch_resolves_correctly_when_the_reference_is_a_local_branch_name() {
        assert_eq!(
            worktree::fallback_lookup_branch("feature-x", None),
            "feature-x"
        );
    }

    /// Bash treats empty and unset alike in both places this value is read, so
    /// an empty name must behave exactly like `None`. `Option` alone does not
    /// draw that line, which is why the port normalises it.
    #[test]
    fn an_empty_branch_name_is_treated_as_absent_like_in_bash() {
        assert_eq!(
            worktree::worktree_add_args(Path::new("/tmp/d"), "origin/main", Some(""), false),
            worktree::worktree_add_args(Path::new("/tmp/d"), "origin/main", None, false),
            "an empty name must not produce a -b flag"
        );
        assert_eq!(
            worktree::fallback_lookup_branch("origin/main", Some("")),
            "origin/main",
            "${{new_branch:-$ref}} falls back on empty, not just unset"
        );
    }
}

/// `create_worktree`: integration tests against a real git repo, since the
/// function's whole job is driving three real `git worktree` invocations in
/// sequence.
mod create_worktree_ladder {
    use super::*;
    use playbook::cc::worktree::{create_worktree, CreateOutcome};

    /// Disables the machine's global/system git config for the duration of
    /// `f`. Per-repo identity is already set by `seeded_repo`, but a global
    /// `core.hooksPath` could still fire during `worktree add`/`prune`/
    /// `repair` and make the test depend on the machine running it.
    fn with_isolated_git_env<T>(f: impl FnOnce() -> T) -> T {
        let _guard = lock_env();
        let prev_global = std::env::var_os("GIT_CONFIG_GLOBAL");
        let prev_system = std::env::var_os("GIT_CONFIG_SYSTEM");
        std::env::set_var("GIT_CONFIG_GLOBAL", "/dev/null");
        std::env::set_var("GIT_CONFIG_SYSTEM", "/dev/null");
        let out = f();
        match prev_global {
            Some(v) => std::env::set_var("GIT_CONFIG_GLOBAL", v),
            None => std::env::remove_var("GIT_CONFIG_GLOBAL"),
        }
        match prev_system {
            Some(v) => std::env::set_var("GIT_CONFIG_SYSTEM", v),
            None => std::env::remove_var("GIT_CONFIG_SYSTEM"),
        }
        out
    }

    fn git(repo_path: &Path, args: &[&str]) -> std::process::Output {
        Command::new("git")
            .arg("-C")
            .arg(repo_path)
            .args(args)
            .output()
            .expect("git command should spawn")
    }

    fn git_stdout(repo_path: &Path, args: &[&str]) -> String {
        let output = git(repo_path, args);
        String::from_utf8_lossy(&output.stdout).trim().to_string()
    }

    /// A repo with one commit on `main`, so `git worktree add` has a ref to
    /// check out. Self-contained rather than reusing `repo()`, so the branch
    /// name is explicit instead of depending on git's default-branch config.
    fn seeded_repo(tag: &str) -> PathBuf {
        let dir = scratch(tag);
        for args in [
            vec!["init", "-q", "-b", "main"],
            vec!["config", "user.email", "t@t"],
            vec!["config", "user.name", "T"],
        ] {
            git(&dir, &args);
        }
        fs::write(dir.join("README.md"), "seed\n").expect("write");
        git(&dir, &["add", "."]);
        git(&dir, &["commit", "-q", "-m", "seed"]);
        dir
    }

    /// The plain first attempt succeeds outright: no prune, repair, or retry
    /// needed.
    #[test]
    fn a_clean_creation_succeeds_on_the_first_attempt() {
        with_isolated_git_env(|| {
            let repo_root = seeded_repo("clean-create");
            let dest = repo_root.join("wt-feature-a");

            let outcome = create_worktree(&repo_root, &dest, "main", Some("feature-a"));

            assert_eq!(outcome, CreateOutcome::Created(dest.clone()));
            assert_eq!(
                git_stdout(&dest, &["rev-parse", "--abbrev-ref", "HEAD"]),
                "feature-a"
            );
        });
    }

    /// Both the plain and `-f` attempts fail because the destination is the
    /// exact directory the branch is already checked out in (a real,
    /// non-empty path collision, which `-f` does not override). The ladder
    /// falls back to reporting that existing path rather than failing.
    #[test]
    fn an_occupied_destination_falls_back_to_already_at() {
        with_isolated_git_env(|| {
            let repo_root = seeded_repo("occupied-fallback");
            let existing = repo_root.join("wt-feature-b");
            let out = git(
                &repo_root,
                &[
                    "worktree",
                    "add",
                    "-q",
                    "-b",
                    "feature-b",
                    existing.to_str().expect("utf8 path"),
                    "main",
                ],
            );
            assert!(out.status.success(), "fixture worktree should be created");

            let outcome = create_worktree(&repo_root, &existing, "feature-b", None);

            // git registers worktrees by their resolved path, and on macOS
            // /tmp is a symlink into /private/tmp, so both sides must be
            // canonicalized before comparing or this fails for a reason that
            // has nothing to do with the port.
            let expected = existing.canonicalize().expect("existing should resolve");
            match outcome {
                CreateOutcome::AlreadyAt(path) => {
                    assert_eq!(
                        path.canonicalize().expect("returned path should resolve"),
                        expected
                    );
                }
                other => panic!("expected AlreadyAt, got {other:?}"),
            }
        });
    }

    /// The fallback's `-d "$existing"` check: `git worktree list --porcelain`
    /// still reports a locked-but-missing worktree for the branch (locking
    /// keeps `worktree prune` from clearing it), so a path IS found, but it no
    /// longer exists as a directory. Dropping the directory check would wrongly
    /// report `AlreadyAt` here.
    #[test]
    fn a_found_but_missing_fallback_path_is_not_treated_as_already_at() {
        with_isolated_git_env(|| {
            let repo_root = seeded_repo("missing-fallback-dir");
            let stale = repo_root.join("wt-feature-e");
            git(
                &repo_root,
                &[
                    "worktree",
                    "add",
                    "-q",
                    "-b",
                    "feature-e",
                    stale.to_str().expect("utf8 path"),
                    "main",
                ],
            );
            git(
                &repo_root,
                &["worktree", "lock", stale.to_str().expect("utf8 path")],
            );
            fs::remove_dir_all(&stale).expect("remove the worktree dir out from under git");

            // A second, unrelated, non-empty destination: both the plain and
            // `-f` attempts fail on THIS path already existing, so the ladder
            // reaches the fallback lookup for `feature-e`.
            let dest = repo_root.join("wt-blocked");
            fs::create_dir_all(&dest).expect("mkdir");
            fs::write(dest.join("junk"), "x").expect("write");

            let outcome = create_worktree(&repo_root, &dest, "feature-e", None);

            assert_eq!(outcome, CreateOutcome::Failed);
        });
    }

    /// Both attempts fail and the fallback lookup finds nothing at all: the
    /// branch was never actually created, since neither `git worktree add`
    /// attempt got past the destination already existing.
    #[test]
    fn a_true_failure_with_no_fallback_returns_failed() {
        with_isolated_git_env(|| {
            let repo_root = seeded_repo("no-fallback");
            let dest = repo_root.join("wt-blocked");
            fs::create_dir_all(&dest).expect("mkdir");
            fs::write(dest.join("junk"), "x").expect("write");

            let outcome = create_worktree(&repo_root, &dest, "main", Some("orphan-branch"));

            assert_eq!(outcome, CreateOutcome::Failed);
        });
    }
}

/// Unit tests on the off-by-one guard in isolation, against synthetic
/// `git worktree list --porcelain` text, no real git involved.
///
/// This exists as its own module because [`cleanup_stale_execution`]'s
/// real-git "main worktree survives" test, despite being the most obviously
/// important one to read, cannot actually distinguish correct code from the
/// off-by-one bug: `git worktree remove` already refuses to remove the main
/// working tree by itself, so a live-git test observes the identical outcome
/// either way. This module is the one that actually fails if the skip is
/// ever dropped or miscounted.
mod cleanup_candidate_paths {
    use super::*;

    const PORCELAIN: &str = "worktree /repo\nHEAD abc\nbranch refs/heads/main\n\
                             \nworktree /repo/.worktrees/a\nHEAD def\n\
                             branch refs/heads/a\n\
                             \nworktree /repo/.worktrees/b\nHEAD ghi\n\
                             branch refs/heads/b\n";

    #[test]
    fn drops_only_the_first_worktree_entry() {
        assert_eq!(
            worktree::cleanup_candidates(PORCELAIN),
            vec![
                "/repo/.worktrees/a".to_string(),
                "/repo/.worktrees/b".to_string(),
            ]
        );
    }

    #[test]
    fn a_single_worktree_yields_no_candidates() {
        assert!(
            worktree::cleanup_candidates("worktree /repo\nHEAD abc\nbranch refs/heads/main\n")
                .is_empty()
        );
    }

    #[test]
    fn empty_input_yields_no_candidates() {
        assert!(worktree::cleanup_candidates("").is_empty());
    }
}

/// Integration tests for `cleanup_stale_with`, the imperative driver of
/// `_wt_cleanup_stale` (worktree.sh:201-242), against real temp git repos
/// with real `git worktree add`.
///
/// `cleanup_stale` (the outer entry point that additionally gathers a real
/// `/tmp/.git-wt-cleanup-*` marker path and a real `gh pr list`) is
/// deliberately NOT exercised here: doing so would either touch this
/// machine's actual marker file or depend on `gh` being installed and
/// authenticated, exactly what this module must not do. Every test below
/// injects its own scratch marker and PR list into `cleanup_stale_with`
/// instead.
mod cleanup_stale_execution {
    use super::*;
    use playbook::cc::worktree::cleanup_stale_with;
    use std::time::{SystemTime, UNIX_EPOCH};

    const NOW: i64 = 1_800_000_000;

    /// Disables the machine's global/system git config for the duration of
    /// `f`. Duplicated rather than shared with `create_worktree_ladder`, per
    /// this file's own convention that each module owns its harness.
    fn with_isolated_git_env<T>(f: impl FnOnce() -> T) -> T {
        let _guard = lock_env();
        let prev_global = std::env::var_os("GIT_CONFIG_GLOBAL");
        let prev_system = std::env::var_os("GIT_CONFIG_SYSTEM");
        std::env::set_var("GIT_CONFIG_GLOBAL", "/dev/null");
        std::env::set_var("GIT_CONFIG_SYSTEM", "/dev/null");
        let out = f();
        match prev_global {
            Some(v) => std::env::set_var("GIT_CONFIG_GLOBAL", v),
            None => std::env::remove_var("GIT_CONFIG_GLOBAL"),
        }
        match prev_system {
            Some(v) => std::env::set_var("GIT_CONFIG_SYSTEM", v),
            None => std::env::remove_var("GIT_CONFIG_SYSTEM"),
        }
        out
    }

    fn git(repo_path: &Path, args: &[&str]) -> std::process::Output {
        Command::new("git")
            .arg("-C")
            .arg(repo_path)
            .args(args)
            .output()
            .expect("git command should spawn")
    }

    fn git_ok(repo_path: &Path, args: &[&str]) {
        let out = git(repo_path, args);
        assert!(
            out.status.success(),
            "git {args:?} failed: {}",
            String::from_utf8_lossy(&out.stderr)
        );
    }

    fn git_stdout(repo_path: &Path, args: &[&str]) -> String {
        let output = git(repo_path, args);
        String::from_utf8_lossy(&output.stdout).trim().to_string()
    }

    fn branch_exists(repo_root: &Path, branch: &str) -> bool {
        git(
            repo_root,
            &[
                "rev-parse",
                "--verify",
                "--quiet",
                &format!("refs/heads/{branch}"),
            ],
        )
        .status
        .success()
    }

    /// A repo with one commit on `main`, and a hand-built
    /// `refs/remotes/origin/*` (no real remote, no network) so `base_branch`
    /// resolves to `origin/main` the same way it would against a real clone.
    ///
    /// Canonicalized before returning: macOS resolves `/tmp` through
    /// `/private/tmp`, and `git worktree list` reports the resolved path, so
    /// every path comparison in these tests needs to start from a canonical
    /// root or it fails for a reason that has nothing to do with the port.
    fn seeded_repo(tag: &str) -> PathBuf {
        let dir = scratch(tag);
        for args in [
            vec!["init", "-q", "-b", "main"],
            vec!["config", "user.email", "t@t"],
            vec!["config", "user.name", "T"],
        ] {
            git_ok(&dir, &args);
        }
        fs::write(dir.join("README.md"), "seed\n").expect("write");
        git_ok(&dir, &["add", "."]);
        git_ok(&dir, &["commit", "-q", "-m", "seed"]);
        let sha = git_stdout(&dir, &["rev-parse", "HEAD"]);
        git_ok(&dir, &["update-ref", "refs/remotes/origin/main", &sha]);
        git_ok(
            &dir,
            &[
                "symbolic-ref",
                "refs/remotes/origin/HEAD",
                "refs/remotes/origin/main",
            ],
        );
        dir.canonicalize().expect("seeded repo should resolve")
    }

    /// Adds a linked worktree on a new branch created from `main`, so it
    /// starts out merged into `origin/main`, i.e. it would look stale by
    /// default unless something else spares it. Returns its canonical path.
    fn add_worktree(repo_root: &Path, branch: &str) -> PathBuf {
        let dest = repo_root.join(format!("wt-{branch}"));
        git_ok(
            repo_root,
            &[
                "worktree",
                "add",
                "-q",
                "-b",
                branch,
                dest.to_str().expect("utf8 path"),
                "main",
            ],
        );
        dest.canonicalize().expect("worktree should resolve")
    }

    /// A marker path under the repo's own scratch dir, absent by default (so
    /// `cleanup_due` reads it as due), never the shared `/tmp` path a real
    /// run would use.
    fn due_marker(repo_root: &Path) -> PathBuf {
        repo_root.join(".cleanup-marker-test")
    }

    fn real_now_epoch() -> i64 {
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("clock should be after the epoch")
            .as_secs() as i64
    }

    /// The behavioural guarantee that matters most to a user, proven against
    /// real git: even a main worktree that LOOKS stale (merged into base,
    /// nothing else sparing it) survives a cleanup run. Backstopped twice
    /// over: by `cleanup_candidates` (see the sibling module, which is what
    /// actually fails if that skip regresses) and, independently, by git
    /// itself refusing to remove a main working tree.
    #[test]
    fn the_main_worktree_is_never_removed_even_when_it_would_otherwise_look_stale() {
        with_isolated_git_env(|| {
            let repo_root = seeded_repo("main-survives");
            let target = repo_root.join("does-not-exist");
            let marker = due_marker(&repo_root);

            let removed = cleanup_stale_with(&repo_root, &target, &marker, &[], NOW);

            assert_eq!(removed, 0);
            assert!(
                repo_root.is_dir(),
                "the main worktree's directory must survive"
            );
            assert!(branch_exists(&repo_root, "main"));
        });
    }

    #[test]
    fn the_target_worktree_is_never_removed() {
        with_isolated_git_env(|| {
            let repo_root = seeded_repo("target-never-removed");
            // Merged into base and otherwise unprotected: it would be
            // removed if it were not the run's own target.
            let target = add_worktree(&repo_root, "being-created");
            let marker = due_marker(&repo_root);

            let removed = cleanup_stale_with(&repo_root, &target, &marker, &[], NOW);

            assert_eq!(removed, 0);
            assert!(target.is_dir());
            assert!(branch_exists(&repo_root, "being-created"));
        });
    }

    #[test]
    fn a_branch_with_an_open_pr_is_kept() {
        with_isolated_git_env(|| {
            let repo_root = seeded_repo("open-pr-kept");
            let wt = add_worktree(&repo_root, "reviewed");
            let target = repo_root.join("does-not-exist");
            let marker = due_marker(&repo_root);
            let open_prs = vec!["reviewed".to_string()];

            let removed = cleanup_stale_with(&repo_root, &target, &marker, &open_prs, NOW);

            assert_eq!(removed, 0);
            assert!(wt.is_dir());
            assert!(branch_exists(&repo_root, "reviewed"));
        });
    }

    /// `grep -qxF` is whole-line, so an open-PR entry of `feat-two` must not
    /// spare a branch named `feat` merely because `feat` is a substring of it.
    #[test]
    fn an_open_pr_entry_does_not_spare_a_branch_it_is_a_superstring_of() {
        with_isolated_git_env(|| {
            let repo_root = seeded_repo("whole-line-superstring");
            let wt = add_worktree(&repo_root, "feat");
            let target = repo_root.join("does-not-exist");
            let marker = due_marker(&repo_root);
            let open_prs = vec!["feat-two".to_string()];

            let removed = cleanup_stale_with(&repo_root, &target, &marker, &open_prs, NOW);

            assert_eq!(
                removed, 1,
                "'feat-two' must not spare 'feat' via a substring match"
            );
            assert!(!wt.exists());
            assert!(!branch_exists(&repo_root, "feat"));
        });
    }

    /// The reverse direction: an open-PR entry of `feat` must not spare a
    /// branch named `feat-two` merely because `feat` is a substring of it.
    #[test]
    fn an_open_pr_entry_does_not_spare_a_branch_it_is_a_substring_of() {
        with_isolated_git_env(|| {
            let repo_root = seeded_repo("whole-line-substring");
            let wt = add_worktree(&repo_root, "feat-two");
            let target = repo_root.join("does-not-exist");
            let marker = due_marker(&repo_root);
            let open_prs = vec!["feat".to_string()];

            let removed = cleanup_stale_with(&repo_root, &target, &marker, &open_prs, NOW);

            assert_eq!(
                removed, 1,
                "'feat' must not spare 'feat-two' via a substring match"
            );
            assert!(!wt.exists());
            assert!(!branch_exists(&repo_root, "feat-two"));
        });
    }

    #[test]
    fn a_merged_worktree_is_removed_and_its_branch_deleted() {
        with_isolated_git_env(|| {
            let repo_root = seeded_repo("merged-removed");
            let wt = add_worktree(&repo_root, "done");
            let target = repo_root.join("does-not-exist");
            let marker = due_marker(&repo_root);

            let removed = cleanup_stale_with(&repo_root, &target, &marker, &[], NOW);

            assert_eq!(removed, 1);
            assert!(!wt.exists());
            assert!(!branch_exists(&repo_root, "done"));
        });
    }

    /// `git worktree lock` makes a single `--force` remove fail
    /// deterministically (`fatal: cannot remove a locked working tree`),
    /// standing in for any real-world removal failure. The branch must
    /// survive: deleting it while its worktree still exists on disk would
    /// leave that worktree pointing at a gone branch.
    #[test]
    fn a_worktree_whose_removal_fails_keeps_its_branch() {
        with_isolated_git_env(|| {
            let repo_root = seeded_repo("remove-fails");
            let wt = add_worktree(&repo_root, "locked");
            git_ok(
                &repo_root,
                &["worktree", "lock", wt.to_str().expect("utf8 path")],
            );
            let target = repo_root.join("does-not-exist");
            let marker = due_marker(&repo_root);

            let removed = cleanup_stale_with(&repo_root, &target, &marker, &[], NOW);

            assert_eq!(removed, 0, "a failed remove must not be counted");
            assert!(branch_exists(&repo_root, "locked"));
        });
    }

    #[test]
    fn a_not_due_marker_returns_zero_and_removes_nothing() {
        with_isolated_git_env(|| {
            let repo_root = seeded_repo("not-due");
            let wt = add_worktree(&repo_root, "would-be-stale");
            let target = repo_root.join("does-not-exist");
            let marker = due_marker(&repo_root);
            let now = real_now_epoch();
            fs::write(&marker, now.to_string()).expect("seed a fresh marker");

            let removed = cleanup_stale_with(&repo_root, &target, &marker, &[], now);

            assert_eq!(removed, 0);
            assert!(
                wt.is_dir(),
                "not due means nothing gets touched, even a mergeable worktree"
            );
            assert!(branch_exists(&repo_root, "would-be-stale"));
        });
    }

    /// The marker is written BEFORE any git work, not after, so a run that
    /// fails partway through still rate-limits the next one. Pinned by
    /// forcing an early failure (a `repo_root` that is not a git repository
    /// at all, so `git worktree list --porcelain` fails and the function
    /// returns before ever reaching its loop) and checking that the marker
    /// was still written.
    #[test]
    fn the_marker_is_touched_before_any_work_even_when_the_run_fails_early() {
        with_isolated_git_env(|| {
            let repo_root = scratch("touch-before-work"); // not a git repo
            let target = repo_root.join("does-not-exist");
            let marker = due_marker(&repo_root);
            assert!(!marker.exists(), "marker must start absent");

            let removed = cleanup_stale_with(&repo_root, &target, &marker, &[], NOW);

            assert_eq!(removed, 0, "a non-repo root has nothing valid to clean");
            assert!(
                marker.exists(),
                "the marker must be written before the failing git work, not after, \
                 so a crash partway through still rate-limits the next run"
            );
        });
    }
}

/// `make_plan`: which ref a new worktree checks out, and whether that means
/// creating a branch. Pure, so covered exhaustively over the three cases.
mod make_source_selection {
    use playbook::cc::worktree::{make_plan, MakePlan};

    #[test]
    fn an_existing_local_branch_is_checked_out_directly() {
        assert_eq!(
            make_plan("feat/x", "origin", "origin/main", true, false),
            MakePlan {
                reference: "feat/x".to_string(),
                new_branch: None,
                unset_upstream: false,
            }
        );
    }

    /// A local branch wins even when the remote also has one, matching the
    /// shell's if/elif order.
    #[test]
    fn a_local_branch_outranks_its_remote_counterpart() {
        let plan = make_plan("feat/x", "origin", "origin/main", true, true);
        assert_eq!(plan.new_branch, None, "nothing to create, it exists");
        assert_eq!(plan.reference, "feat/x");
    }

    #[test]
    fn a_remote_only_branch_is_started_from_its_remote_ref() {
        assert_eq!(
            make_plan("feat/x", "upstream", "origin/main", false, true),
            MakePlan {
                reference: "refs/remotes/upstream/feat/x".to_string(),
                new_branch: Some("feat/x".to_string()),
                unset_upstream: false,
            }
        );
    }

    #[test]
    fn a_brand_new_branch_is_started_from_the_base_ref() {
        assert_eq!(
            make_plan("feat/x", "origin", "origin/trunk", false, false),
            MakePlan {
                reference: "origin/trunk".to_string(),
                new_branch: Some("feat/x".to_string()),
                unset_upstream: true,
            }
        );
    }

    /// The upstream is cleared in exactly one case. Branching off the base ref
    /// inherits the BASE's tracking, so a later bare `git push` would target
    /// the base branch; the other two already track correctly and clearing
    /// them would break that.
    #[test]
    fn only_a_branch_cut_from_base_has_its_upstream_cleared() {
        assert!(!make_plan("b", "origin", "origin/main", true, false).unset_upstream);
        assert!(!make_plan("b", "origin", "origin/main", false, true).unset_upstream);
        assert!(make_plan("b", "origin", "origin/main", false, false).unset_upstream);
    }

    /// The remote name is interpolated rather than assumed to be `origin`, so
    /// a fork workflow resolves against the right remote.
    #[test]
    fn the_remote_name_is_not_hardcoded() {
        let plan = make_plan("b", "fork", "origin/main", false, true);
        assert_eq!(plan.reference, "refs/remotes/fork/b");
    }
}

/// Tests for the execution half of `_wt_maybe_rebase` (worktree.sh:424-458):
/// `rebase_args`, `rebase_onto`, `abort_rebase`, and `recover_detached_head`.
///
/// Every scenario below `rebase_args_ordering` runs against real git repos
/// with a local bare repo standing in for `origin`, so `git fetch`/`git push`
/// exercise the real plumbing without ever touching the network.
mod rebase_execution {
    use super::*;
    use playbook::cc::worktree::{
        abort_rebase, rebase_args, rebase_onto, recover_detached_head, RebaseOutcome,
    };

    /// Disables the machine's global/system git config for the duration of
    /// `f`. Duplicated rather than shared, per this file's convention that
    /// each module owns its harness.
    fn with_isolated_git_env<T>(f: impl FnOnce() -> T) -> T {
        let _guard = lock_env();
        let prev_global = std::env::var_os("GIT_CONFIG_GLOBAL");
        let prev_system = std::env::var_os("GIT_CONFIG_SYSTEM");
        std::env::set_var("GIT_CONFIG_GLOBAL", "/dev/null");
        std::env::set_var("GIT_CONFIG_SYSTEM", "/dev/null");
        let out = f();
        match prev_global {
            Some(v) => std::env::set_var("GIT_CONFIG_GLOBAL", v),
            None => std::env::remove_var("GIT_CONFIG_GLOBAL"),
        }
        match prev_system {
            Some(v) => std::env::set_var("GIT_CONFIG_SYSTEM", v),
            None => std::env::remove_var("GIT_CONFIG_SYSTEM"),
        }
        out
    }

    fn git(repo_path: &Path, args: &[&str]) -> std::process::Output {
        Command::new("git")
            .arg("-C")
            .arg(repo_path)
            .args(args)
            .output()
            .expect("git command should spawn")
    }

    fn git_ok(repo_path: &Path, args: &[&str]) {
        let out = git(repo_path, args);
        assert!(
            out.status.success(),
            "git {args:?} failed: {}",
            String::from_utf8_lossy(&out.stderr)
        );
    }

    fn git_stdout(repo_path: &Path, args: &[&str]) -> String {
        let output = git(repo_path, args);
        String::from_utf8_lossy(&output.stdout).trim().to_string()
    }

    fn commit(repo_path: &Path, file: &str, contents: &str, message: &str) {
        fs::write(repo_path.join(file), contents).expect("write");
        git_ok(repo_path, &["add", "."]);
        git_ok(repo_path, &["commit", "-q", "-m", message]);
    }

    /// A bare repo standing in for `origin`. `git fetch`/`git push` against a
    /// local filesystem path need no network, unlike a real remote.
    fn bare_remote(tag: &str) -> PathBuf {
        let dir = scratch(tag);
        git_ok(&dir, &["init", "-q", "--bare", "-b", "main"]);
        dir.canonicalize().expect("bare remote should resolve")
    }

    /// A repo with `origin` pointed at a local bare repo, and one commit
    /// already pushed to `main` on both sides.
    fn repo_with_remote(tag: &str) -> (PathBuf, PathBuf) {
        let remote = bare_remote(&format!("{tag}-remote"));
        let dir = scratch(tag);
        for args in [
            vec!["init", "-q", "-b", "main"],
            vec!["config", "user.email", "t@t"],
            vec!["config", "user.name", "T"],
            vec![
                "remote",
                "add",
                "origin",
                remote.to_str().expect("utf8 path"),
            ],
        ] {
            git_ok(&dir, &args);
        }
        commit(&dir, "README.md", "seed\n", "seed");
        git_ok(&dir, &["push", "-q", "origin", "main"]);
        (dir.canonicalize().expect("repo should resolve"), remote)
    }

    /// A second, throwaway clone of `remote` that advances `main` with one
    /// commit, simulating someone else pushing while `dir` was left behind.
    /// Writes to `file` so callers control whether the advance later
    /// conflicts with a local change to the same file.
    fn advance_remote(remote: &Path, tag: &str, file: &str, contents: &str) {
        let advancer = scratch(tag);
        for args in [
            vec!["init", "-q", "-b", "main"],
            vec!["config", "user.email", "adv@t"],
            vec!["config", "user.name", "Advancer"],
            vec![
                "remote",
                "add",
                "origin",
                remote.to_str().expect("utf8 path"),
            ],
        ] {
            git_ok(&advancer, &args);
        }
        git_ok(&advancer, &["fetch", "-q", "origin", "main"]);
        git_ok(&advancer, &["checkout", "-q", "-b", "main", "origin/main"]);
        commit(&advancer, file, contents, "remote-advance");
        git_ok(&advancer, &["push", "-q", "origin", "main"]);
    }

    /// Whether a rebase is currently in progress on disk, resolved through
    /// `git rev-parse --git-path` rather than hardcoding `.git/rebase-merge`,
    /// since which of the two backends (`rebase-merge`/`rebase-apply`) git
    /// picks is an implementation detail this test should not assume.
    fn rebase_in_progress(repo_path: &Path) -> bool {
        ["rebase-merge", "rebase-apply"].iter().any(|marker| {
            let relative = git_stdout(repo_path, &["rev-parse", "--git-path", marker]);
            repo_path.join(relative).exists()
        })
    }

    mod rebase_args_ordering {
        use super::*;

        #[test]
        fn without_merge_commits_only_upstream_and_quiet_are_present() {
            assert_eq!(
                rebase_args("origin/main", false),
                vec!["origin/main", "--quiet"]
            );
        }

        #[test]
        fn with_merge_commits_rebase_merges_is_appended_last() {
            assert_eq!(
                rebase_args("origin/main", true),
                vec!["origin/main", "--quiet", "--rebase-merges"]
            );
        }
    }

    #[test]
    fn already_up_to_date_returns_up_to_date_and_does_not_rebase() {
        with_isolated_git_env(|| {
            let (dir, _remote) = repo_with_remote("up-to-date");
            let before = git_stdout(&dir, &["rev-parse", "HEAD"]);

            let outcome = rebase_onto(&dir, "origin", "main");

            assert_eq!(outcome, RebaseOutcome::UpToDate);
            assert_eq!(
                git_stdout(&dir, &["rev-parse", "HEAD"]),
                before,
                "an up-to-date branch must not be touched"
            );
        });
    }

    #[test]
    fn a_clean_divergence_is_rebased_and_history_moves() {
        with_isolated_git_env(|| {
            let (dir, remote) = repo_with_remote("clean-divergence");
            advance_remote(
                &remote,
                "clean-divergence-advancer",
                "remote-only.txt",
                "remote\n",
            );
            commit(&dir, "local-only.txt", "local\n", "local-diverge");
            let before = git_stdout(&dir, &["rev-parse", "HEAD"]);

            let outcome = rebase_onto(&dir, "origin", "main");

            assert_eq!(outcome, RebaseOutcome::Rebased);
            let after = git_stdout(&dir, &["rev-parse", "HEAD"]);
            assert_ne!(after, before, "the rebase must actually move history");
            let log = git_stdout(&dir, &["log", "--oneline"]);
            assert!(
                log.contains("remote-advance"),
                "the rebased branch must sit on top of the remote's commit:\n{log}"
            );
        });
    }

    #[test]
    fn a_genuine_conflict_returns_conflicted_and_leaves_the_rebase_in_progress() {
        with_isolated_git_env(|| {
            let (dir, remote) = repo_with_remote("conflict");
            advance_remote(
                &remote,
                "conflict-advancer",
                "shared.txt",
                "remote-change\n",
            );
            commit(&dir, "shared.txt", "local-change\n", "local-conflict");

            let outcome = rebase_onto(&dir, "origin", "main");

            assert_eq!(outcome, RebaseOutcome::Conflicted);
            assert!(
                rebase_in_progress(&dir),
                "a Conflicted outcome must leave the rebase in progress on disk, \
                 since a caller still needs to resolve or abort it"
            );
        });
    }

    #[test]
    fn abort_rebase_clears_an_in_progress_rebase() {
        with_isolated_git_env(|| {
            let (dir, remote) = repo_with_remote("abort");
            advance_remote(&remote, "abort-advancer", "shared.txt", "remote-change\n");
            commit(&dir, "shared.txt", "local-change\n", "local-conflict");
            let outcome = rebase_onto(&dir, "origin", "main");
            assert_eq!(outcome, RebaseOutcome::Conflicted);
            assert!(rebase_in_progress(&dir), "fixture should start mid-rebase");

            abort_rebase(&dir);

            assert!(
                !rebase_in_progress(&dir),
                "abort_rebase must clear the in-progress rebase"
            );
        });
    }

    #[test]
    fn recover_detached_head_restores_the_branch_from_a_genuinely_detached_head() {
        with_isolated_git_env(|| {
            let (dir, _remote) = repo_with_remote("detached-recovery");
            let sha = git_stdout(&dir, &["rev-parse", "HEAD"]);
            git_ok(&dir, &["checkout", &sha]);
            assert_eq!(
                git_stdout(&dir, &["rev-parse", "--abbrev-ref", "HEAD"]),
                "HEAD",
                "fixture should start detached"
            );

            let acted = recover_detached_head(&dir, "main", "main");

            assert!(acted);
            assert_eq!(
                git_stdout(&dir, &["rev-parse", "--abbrev-ref", "HEAD"]),
                "main"
            );
        });
    }

    #[test]
    fn recover_detached_head_does_nothing_when_head_is_attached() {
        with_isolated_git_env(|| {
            let (dir, _remote) = repo_with_remote("attached-noop");
            let before = git_stdout(&dir, &["rev-parse", "HEAD"]);

            let acted = recover_detached_head(&dir, "main", "main");

            assert!(!acted, "an attached HEAD is nothing to recover from");
            assert_eq!(
                git_stdout(&dir, &["rev-parse", "--abbrev-ref", "HEAD"]),
                "main"
            );
            assert_eq!(git_stdout(&dir, &["rev-parse", "HEAD"]), before);
        });
    }

    /// The fallback order, half one: when `current_branch` no longer exists,
    /// recovery falls back to `wanted_branch`. This alone cannot tell a
    /// correct fallback from a swapped order, since only one candidate exists
    /// here; see the sibling test below for the half that can.
    #[test]
    fn falls_back_to_wanted_branch_when_current_branch_no_longer_exists() {
        with_isolated_git_env(|| {
            let (dir, _remote) = repo_with_remote("fallback-order");
            let sha = git_stdout(&dir, &["rev-parse", "HEAD"]);
            git_ok(&dir, &["checkout", &sha]);

            let acted = recover_detached_head(&dir, "does-not-exist", "main");

            assert!(acted);
            assert_eq!(
                git_stdout(&dir, &["rev-parse", "--abbrev-ref", "HEAD"]),
                "main",
                "current_branch does not exist, so recovery must fall back to wanted_branch"
            );
        });
    }

    /// The fallback order, half two: when BOTH branches exist, `current_branch`
    /// wins. The shell's `||` chain fixes that order (current branch first,
    /// wanted branch second), and this is the test that fails if the two ever
    /// get swapped.
    #[test]
    fn current_branch_wins_over_wanted_branch_when_both_exist() {
        with_isolated_git_env(|| {
            let (dir, _remote) = repo_with_remote("fallback-order-both-exist");
            git_ok(&dir, &["checkout", "-q", "-b", "other"]);
            let sha = git_stdout(&dir, &["rev-parse", "HEAD"]);
            git_ok(&dir, &["checkout", &sha]);

            let acted = recover_detached_head(&dir, "other", "main");

            assert!(acted);
            assert_eq!(
                git_stdout(&dir, &["rev-parse", "--abbrev-ref", "HEAD"]),
                "other",
                "current_branch must be tried before wanted_branch"
            );
        });
    }
}
