// SPDX-FileCopyrightText: 2026 Igor Santos
// SPDX-License-Identifier: Apache-2.0

//! Scratch roots of the `rm` guard: which temp, worktree and configured
//! directories may be cleaned up, and every way of escaping them.
//!
//! The suite runs the guard under a fake `$HOME` so the derived roots
//! (`~/.cache`, `~/.claude`, the config store) are deterministic. Deletion
//! commands are assembled at runtime so this file never carries a literal one.

use std::fs;
use std::os::unix::fs::symlink;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::atomic::{AtomicU64, Ordering};

static COUNTER: AtomicU64 = AtomicU64::new(0);

fn rm() -> String {
    "r".to_string() + "m"
}

/// A fixture tree:
/// `base/home` (fake HOME), `base/org/repo` (git repo with a linked worktree at
/// `base/org/.worktrees/repo/wt1`), and a real directory under `/tmp`.
struct Fx {
    base: PathBuf,
    home: PathBuf,
    repo: PathBuf,
    wt_root: PathBuf,
    wt1: PathBuf,
    tmp: PathBuf,
}

impl Fx {
    fn new() -> Fx {
        let n = COUNTER.fetch_add(1, Ordering::Relaxed);
        let run = playbook::testing::run_id();
        // Cargo's per-target scratch folder: outside every root the guard
        // derives, and independent of the developer's real HOME and /tmp.
        let base = PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join(format!("rmroots-{run}-{n}"));
        fs::create_dir_all(&base).expect("base");
        let base = fs::canonicalize(base).expect("canonical base");
        let home = base.join("home");
        let repo = base.join("org/repo");
        let wt_root = base.join("org/.worktrees");
        let wt1 = wt_root.join("repo/wt1");
        fs::create_dir_all(&home).expect("home");
        fs::create_dir_all(&repo).expect("repo");
        let git = |dir: &Path, args: &[&str]| {
            let out = Command::new("git")
                .arg("-C")
                .arg(dir)
                .env("GIT_CONFIG_GLOBAL", "/dev/null")
                .env("GIT_CONFIG_NOSYSTEM", "1")
                .env_remove("GIT_DIR")
                .env_remove("GIT_WORK_TREE")
                .args(["-c", "user.name=t", "-c", "user.email=t@example.com"])
                .args(["-c", "commit.gpgsign=false"])
                .args(args)
                .output()
                .expect("git");
            assert!(out.status.success(), "git {args:?}: {out:?}");
        };
        git(&repo, &["init", "-q"]);
        git(&repo, &["commit", "-q", "--allow-empty", "-m", "init"]);
        git(
            &repo,
            &[
                "worktree",
                "add",
                "-q",
                "-b",
                "wt1",
                wt1.to_str().expect("utf8"),
            ],
        );
        let tmp = PathBuf::from(format!("/tmp/pbtest-{run}-{n}"));
        fs::create_dir_all(tmp.join("sub")).expect("tmp dir");
        Fx {
            base,
            home,
            repo,
            wt_root,
            wt1,
            tmp,
        }
    }

    fn denied(&self, command: &str, cwd: &Path, extra_env: &[(&str, &str)]) -> bool {
        let payload = serde_json::json!({ "tool_input": { "command": command } }).to_string();
        let mut spawn = Command::new(env!("CARGO_BIN_EXE_playbook"));
        spawn
            .args(["hook", "rm-workspace-guard"])
            .env("HOOK_INPUT", payload)
            .env("HOME", &self.home)
            .env_remove("PLAYBOOK_SAFE_ROOTS")
            .env_remove("WORKTREE_BASE_DIR")
            .env_remove("GIT_DIR")
            .env_remove("GIT_WORK_TREE")
            .current_dir(cwd);
        for (k, v) in extra_env {
            spawn.env(k, v);
        }
        let out = spawn.output().expect("spawn");
        assert!(out.status.success(), "a guard exits 0 even when denying");
        String::from_utf8_lossy(&out.stdout).contains(r#""permissionDecision":"deny""#)
    }

    fn config_set(&self, value: &str) -> std::process::Output {
        Command::new(env!("CARGO_BIN_EXE_playbook"))
            .args([
                "config",
                "set",
                "--global",
                "hooks.rmGuard.allowedRoots",
                value,
            ])
            .env("HOME", &self.home)
            .output()
            .expect("config set")
    }
}

impl Drop for Fx {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.tmp);
        let _ = fs::remove_dir_all(&self.base);
    }
}

fn check(fx: &Fx, cwd: &Path, cases: &[(String, bool)]) {
    for (cmd, want_denied) in cases {
        assert_eq!(
            fx.denied(cmd, cwd, &[]),
            *want_denied,
            "expected {} for: {cmd}",
            if *want_denied { "deny" } else { "allow" }
        );
    }
}

fn s(p: &Path) -> String {
    p.display().to_string()
}

#[test]
fn worktree_and_clone_leftovers_are_allowed_but_never_the_root_or_parents() {
    let fx = Fx::new();
    let (d, w) = (rm(), s(&fx.wt_root));
    let cases: Vec<(String, bool)> = vec![
        (format!("{d} -rf {w}/cargo-rm-guard"), false),
        (format!("{d} -rf {w}/new-repos/some-name"), false),
        (format!("{d} -rf {w}/repo/wt1"), false),
        (format!("{d} -rf {w}/repo/never-existed/deep"), false),
        (format!("{d} -rf {w}/cargo-rm-guard/"), false),
        (format!("{d} -rf {w}/cargo-*"), false),
        (format!("{d} -rf {w}"), true),
        (format!("{d} -rf {w}/"), true),
        (format!("{d} -rf {w}/."), true),
        (format!("{d} -rf {w}/repo/.."), true),
        (format!("{d} -rf {w}/*"), true),
        (format!("{d} -rf {w}/c*"), true),
        (format!("{d} -rf {w}/cargo-*/x"), true),
        (format!("{d} -rf {}", s(&fx.base.join("org"))), true),
        (format!("{d} -rf {w}/../repo-other"), true),
        (format!("{d} -rf {w}/../../home/.ssh"), true),
    ];
    check(&fx, &fx.repo, &cases);
    // The derivation also holds from inside the linked worktree: the folder is
    // next to the MAIN checkout, not next to the worktree.
    check(&fx, &fx.wt1, &cases);
}

#[test]
fn relative_paths_resolve_from_the_working_directory() {
    let fx = Fx::new();
    let d = rm();
    fs::create_dir_all(fx.wt1.join("build")).expect("build");
    let cases: Vec<(String, bool)> = vec![
        (format!("{d} -rf build"), false),
        (format!("{d} -rf ./build/"), false),
        (format!("{d} -rf ../../cargo-x"), false),
        (format!("{d} -rf ../.."), true),
        (format!("{d} -rf ../../.."), true),
        (format!("{d} -rf ../../../../home/.ssh"), true),
        (format!("cd {} && {d} -rf build", s(&fx.wt1)), true),
    ];
    check(&fx, &fx.wt1, &cases);
}

#[test]
fn os_temp_directories_are_allowed_but_never_the_root_or_a_bare_glob() {
    let fx = Fx::new();
    let d = rm();
    let t = s(&fx.tmp);
    let mut cases: Vec<(String, bool)> = vec![
        (format!("{d} -rf {t}/sub"), false),
        (format!("{d} -rf {t}/missing/deep"), false),
        (format!("{d} -rf {t}/sub/"), false),
        (format!("{d} -rf '{t}/sub'"), false),
        (format!("{d} -rf \"{t}/sub\""), false),
        (format!("{d} -rf {t}/*"), false),
        (format!("{d} -rf /tmp/pb-*"), false),
        (format!("{d} -rf /tmp/pbtest-*"), false),
        (
            format!("{d} -rf /private/tmp/claude-501/-Users-x/abc/scratchpad"),
            false,
        ),
        (format!("{d} -rf /private/tmp/tmp.AbC123"), false),
        (
            format!("{d} -rf /private/var/folders/ab/cdef0123/T/tmp.AbC123"),
            false,
        ),
        (
            format!("{d} -rf /private/var/folders/ab/cdef0123/T/tmp.*"),
            false,
        ),
        (format!("{d} -rf /tmp"), true),
        (format!("{d} -rf /tmp/"), true),
        (format!("{d} -rf /tmp/."), true),
        (format!("{d} -rf /private/tmp"), true),
        (format!("{d} -rf /private"), true),
        (format!("{d} -rf /"), true),
        (format!("{d} -rf /*"), true),
        (format!("{d} -rf /private/var/folders/ab/cdef0123/T"), true),
        (format!("{d} -rf /private/var/folders/ab/cdef0123/T/"), true),
        (format!("{d} -rf /private/var/folders/ab/cdef0123"), true),
        (
            format!("{d} -rf /private/var/folders/ab/cdef0123/C/x"),
            true,
        ),
        (format!("{d} -rf /private/var/folders"), true),
        (format!("{d} -rf /private/var/folders/*/*/T/*"), true),
        (format!("{d} -rf /tmp/*"), true),
        (format!("{d} -rf /private/tmp/*"), true),
        (format!("{d} -rf /tmp/p*"), true),
        (format!("{d} -rf /tmp/pb*"), true),
        (format!("{d} -rf /tmp/.*"), true),
        (format!("{d} -rf /tmp/?bc-x"), true),
        (format!("{d} -rf /tmp/{{a,b}}"), true),
        (format!("{d} -rf /tmp/*-x"), true),
        (format!("{d} -rf /tmp/pb-*/sub"), true),
        (format!("{d} -rf /tmp/pb-*/"), true),
        (format!("{d} -rf /tmp/pb-*/../x"), true),
        (format!("{d} -rf {t}/*/x"), true),
        (format!("{d} -rf /tmp/../etc"), true),
        (format!("{d} -rf {t}/sub/../../../etc"), true),
        (format!("{d} -rf /tmp/pb-*/../../etc"), true),
        (format!("{d} -rf /tmpfoo/x"), true),
        (format!("{d} -rf $TMPDIR/x"), true),
        (format!("{d} -rf \"$TMPDIR\"/x"), true),
        (format!("{d} -rf $(mktemp -d)"), true),
        (format!("{d} -rf \"/etc/x\""), true),
        (format!("{d} -rf '/tmp/ok\\ x'"), true),
        (format!("{d} -rf /tmp/with\\ space"), true),
        (format!("{d} -rf \"/tmp/two words\""), true),
    ];
    if cfg!(target_os = "macos") {
        cases.push((
            format!("{d} -rf /var/folders/ab/cdef0123/T/tmp.AbC123"),
            false,
        ));
        cases.push((format!("{d} -rf /var/folders/ab/cdef0123/T"), true));
    }
    check(&fx, &fx.repo, &cases);
}

#[test]
fn symlinks_and_dot_dot_are_judged_where_they_really_point() {
    let fx = Fx::new();
    let d = rm();
    let t = s(&fx.tmp);
    let secret = fx.home.join(".ssh");
    fs::create_dir_all(&secret).expect("secret");
    symlink(&fx.home, fx.tmp.join("to-home")).expect("link");
    symlink(&secret, fx.tmp.join("to-ssh")).expect("link");
    symlink("../..", fx.tmp.join("to-parent")).expect("rel link");
    symlink(fx.tmp.join("sub"), fx.tmp.join("to-sub")).expect("inner link");
    symlink(fx.tmp.join("loop-b"), fx.tmp.join("loop-a")).expect("loop");
    symlink(fx.tmp.join("loop-a"), fx.tmp.join("loop-b")).expect("loop");
    symlink(&fx.home, fx.wt_root.join("to-home")).expect("wt link");
    fs::create_dir_all(fx.wt_root.join("real")).expect("real");
    symlink(&fx.home, fx.repo.join("out-link")).expect("repo link");
    symlink(&fx.tmp, fx.home.join("into-tmp")).expect("home link");
    symlink("/tmp", fx.home.join("to-tmp-root")).expect("root link");
    let h = s(&fx.home);
    let cases: Vec<(String, bool)> = vec![
        (format!("{d} -rf {t}/to-sub"), false),
        (format!("{d} -rf {t}/to-sub/x"), false),
        (format!("{d} -rf {t}/to-home"), true),
        (format!("{d} -rf {t}/to-home/"), true),
        (format!("{d} -rf {t}/to-home/.ssh"), true),
        (format!("{d} -rf {t}/to-ssh"), true),
        (format!("{d} -rf {t}/to-parent/x"), true),
        (format!("{d} -rf {t}/to-parent"), true),
        (format!("{d} -rf {t}/to-home/../x"), true),
        (format!("{d} -rf {t}/loop-a"), true),
        (format!("{d} -rf {}/to-home", s(&fx.wt_root)), true),
        (format!("{d} -rf {}/to-home/.ssh", s(&fx.wt_root)), true),
        (format!("{d} -rf {}/out-link", s(&fx.repo)), true),
        (format!("{d} -rf {}/out-link/.ssh", s(&fx.repo)), true),
        (format!("{d} -rf {h}/into-tmp/sub"), false),
        (format!("{d} -rf {h}/into-tmp/"), false),
        (format!("{d} -rf {h}/into-tmp"), true),
        (format!("{d} -rf {h}/to-tmp-root"), true),
        (format!("{d} -rf {t}/to-home/* "), true),
    ];
    check(&fx, &fx.repo, &cases);
}

#[test]
fn multiple_targets_and_chained_commands_are_all_judged() {
    let fx = Fx::new();
    let (d, t, h) = (rm(), s(&fx.tmp), s(&fx.home));
    let cases: Vec<(String, bool)> = vec![
        (format!("{d} -rf {t}/sub {t}/other"), false),
        (format!("{d} -rf {t}/sub {h}/.ssh"), true),
        (format!("{d} -rf {h}/.ssh {t}/sub"), true),
        (format!("{d} -rf {t}/sub && {d} -rf {h}/.ssh"), true),
        (
            format!("{d} -rf {t}/sub && {d} -rf {}/cargo-x", s(&fx.wt_root)),
            false,
        ),
        (format!("echo hi ; {d} -rf {t}/sub ; ls {h}"), false),
        (format!("{d} -rf {t}/sub /etc/x"), true),
        // Only `rm` is inspected: pinned so it is not re-reported later.
        (format!("rmdir {h}/.ssh"), false),
    ];
    check(&fx, &fx.repo, &cases);
}

#[test]
fn home_expansion_covers_only_the_callers_home() {
    let fx = Fx::new();
    let d = rm();
    let cases: Vec<(String, bool)> = vec![
        (format!("{d} -rf ~/.cache/some-tool"), false),
        (format!("{d} -rf ~/.claude/cache/x"), false),
        (format!("{d} -rf ~/.cache"), true),
        (format!("{d} -rf ~/.cache/"), true),
        (format!("{d} -rf ~/.ssh"), true),
        (format!("{d} -rf ~"), true),
        (format!("{d} -rf ~/"), true),
        (format!("{d} -rf ~root/x"), true),
        (format!("{d} -rf ~+/x"), true),
        (format!("{d} -rf ~/.cache/../.ssh"), true),
    ];
    check(&fx, &fx.repo, &cases);
}

#[test]
fn configured_roots_are_contents_only_and_validated() {
    let fx = Fx::new();
    let d = rm();
    let ext = fx.base.join("ext/out");
    fs::create_dir_all(&ext).expect("ext");
    let before = vec![
        (format!("{d} -rf ~/scratch/builds/x"), true),
        (format!("{d} -rf {}/x", s(&ext)), true),
    ];
    check(&fx, &fx.repo, &before);

    let value = format!(r#"["~/scratch/builds", "{}"]"#, s(&ext));
    let out = fx.config_set(&value);
    assert!(out.status.success(), "{out:?}");
    let after = vec![
        (format!("{d} -rf ~/scratch/builds/x"), false),
        (format!("{d} -rf {}/x/y", s(&ext)), false),
        (format!("{d} -rf ~/scratch/builds/pbx-*"), false),
        (format!("{d} -rf ~/scratch/builds/*"), true),
        (format!("{d} -rf ~/scratch/builds"), true),
        (format!("{d} -rf ~/scratch"), true),
        (format!("{d} -rf ~/scratch/builds/.."), true),
        (format!("{d} -rf ~/scratch/other"), true),
        (format!("{d} -rf {}", s(&ext)), true),
        (format!("{d} -rf {}", s(&fx.base.join("ext"))), true),
    ];
    check(&fx, &fx.repo, &after);
}

#[test]
fn a_configured_root_that_holds_home_is_dropped_at_run_time() {
    let fx = Fx::new();
    let d = rm();
    // Two levels deep, so the writer accepts it; the guard still refuses to
    // treat home, or a parent of home, as scratch.
    for root in [s(&fx.home), s(&fx.base)] {
        let out = fx.config_set(&format!(r#"["{root}"]"#));
        assert!(out.status.success(), "{out:?}");
        for cmd in [
            format!("{d} -rf {}/.ssh", s(&fx.home)),
            format!("{d} -rf {}/anything", s(&fx.home)),
            format!("{d} -rf {}/org/other", s(&fx.base)),
        ] {
            assert!(fx.denied(&cmd, &fx.repo, &[]), "should deny: {cmd}");
        }
    }
}

#[test]
fn the_config_key_rejects_dangerous_or_malformed_values() {
    let fx = Fx::new();
    for bad in [
        r#"["/"]"#,
        r#"["~"]"#,
        r#"["~/"]"#,
        r#"["/Users"]"#,
        r#"["/tmp"]"#,
        r#"["relative/dir"]"#,
        r#"["/a/b/../c"]"#,
        r#"["~/a/../.."]"#,
        r#"["/a/b/*"]"#,
        r#"["~//etc"]"#,
        r#"["$HOME/x"]"#,
        r#"["/opt/$USER/x"]"#,
        r#"["/opt/a`b`/x"]"#,
        r#"["/opt/it's/x"]"#,
        r#"["/opt/a\\b/x"]"#,
        r#"["/opt/a\"b/x"]"#,
        r#"["/opt/a{b,c}/x"]"#,
        r#"[1]"#,
    ] {
        let out = fx.config_set(bad);
        assert!(!out.status.success(), "should reject {bad}: {out:?}");
    }
    for good in [
        r#"["~/a", "/opt/data/scratch"]"#,
        "~/a:/opt/data/scratch",
        "",
    ] {
        let out = fx.config_set(good);
        assert!(out.status.success(), "should accept {good:?}: {out:?}");
    }
}

#[test]
fn a_worktree_base_dir_from_the_environment_is_allowed_contents_only() {
    let fx = Fx::new();
    let d = rm();
    let base = fx.base.join("wtbase");
    fs::create_dir_all(&base).expect("wtbase");
    let env = [("WORKTREE_BASE_DIR", base.to_str().expect("utf8"))];
    let b = s(&base);
    for (cmd, want) in [
        (format!("{d} -rf {b}/repo/x"), false),
        (format!("{d} -rf {b}"), true),
        (format!("{d} -rf {b}/*"), true),
    ] {
        assert_eq!(fx.denied(&cmd, &fx.repo, &env), want, "{cmd}");
    }
    // A hostile value must not widen anything.
    // A relative value must be ignored, not resolved against the cwd.
    let relative = [("WORKTREE_BASE_DIR", "../sibling")];
    let cmd = format!("{d} -rf {}/x", s(&fx.base.join("org/sibling")));
    assert!(
        fx.denied(&cmd, &fx.repo, &relative),
        "relative base is ignored"
    );
    for bad in ["/", &s(&fx.home)] {
        let env = [("WORKTREE_BASE_DIR", bad)];
        let cmd = format!("{d} -rf {}/.ssh", s(&fx.home));
        assert!(fx.denied(&cmd, &fx.repo, &env), "base {bad:?}");
    }
}

#[test]
fn the_denial_message_names_the_scratch_roots_and_the_config_key() {
    let fx = Fx::new();
    let payload =
        serde_json::json!({ "tool_input": { "command": format!("{} -rf /etc/x", rm()) } })
            .to_string();
    let out = Command::new(env!("CARGO_BIN_EXE_playbook"))
        .args(["hook", "rm-workspace-guard"])
        .env("HOOK_INPUT", payload)
        .env("HOME", &fx.home)
        .env_remove("PLAYBOOK_SAFE_ROOTS")
        .current_dir(&fx.repo)
        .output()
        .expect("spawn");
    let text = String::from_utf8_lossy(&out.stdout);
    for needle in [
        "/tmp/*",
        "/private/var/folders",
        ".worktrees/*",
        "hooks.rmGuard.allowedRoots",
    ] {
        assert!(
            text.contains(needle),
            "message should mention {needle}: {text}"
        );
    }
}

#[test]
fn a_colon_separated_value_is_split_into_roots() {
    let fx = Fx::new();
    let d = rm();
    assert!(fx.config_set("~/a:/opt/data/scratch").status.success());
    for (cmd, want) in [
        (format!("{d} -rf ~/a/x"), false),
        (format!("{d} -rf /opt/data/scratch/x"), false),
        (format!("{d} -rf /opt/data/scratch"), true),
        (format!("{d} -rf /opt/data"), true),
        (format!("{d} -rf ~/a"), true),
    ] {
        assert_eq!(fx.denied(&cmd, &fx.repo, &[]), want, "{cmd}");
    }
}

#[test]
fn a_configured_root_that_links_to_home_or_the_filesystem_root_is_dropped() {
    let fx = Fx::new();
    let d = rm();
    for target in [s(&fx.home), "/".to_string()] {
        let link = fx.home.join("scratch-link");
        let _ = fs::remove_file(&link);
        symlink(&target, &link).expect("link");
        assert!(fx.config_set(r#"["~/scratch-link"]"#).status.success());
        for cmd in [
            format!("{d} -rf {}/.ssh", s(&fx.home)),
            format!("{d} -rf ~/scratch-link/.ssh"),
            format!("{d} -rf /etc/x"),
        ] {
            assert!(fx.denied(&cmd, &fx.repo, &[]), "{target}: {cmd}");
        }
    }
}

#[test]
fn quoting_braces_and_shell_syntax_cannot_smuggle_a_path_out() {
    let fx = Fx::new();
    let (d, t, r) = (rm(), s(&fx.tmp), s(&fx.repo));
    let cases: Vec<(String, bool)> = vec![
        (format!("{d} -rf {r}/{{a,/etc/x}}"), true),
        (format!("{d} -rf {r}/{{a,b}}"), false),
        (format!("{d} -rf {r}/{{a,../../x}}"), true),
        (format!("{d} -rf {t}/pbx-{{a,/etc/x}}"), true),
        (format!("{d} -rf \"~/.cache/x\""), true),
        (format!("{d} -rf '~'"), true),
        (format!("{d} -rf {t}/sub(:h)"), true),
        (format!("{d} -rf {t}/sub<x"), true),
        (format!("cd {t} && {d} -rf \"{t}/sub\""), false),
        (format!("cd {t} && {d} -rf sub"), true),
    ];
    check(&fx, &fx.repo, &cases);
}
