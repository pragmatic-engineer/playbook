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
