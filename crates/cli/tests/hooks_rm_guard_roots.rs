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
        self.reason(command, cwd, extra_env).is_some()
    }

    /// The denial text, or `None` when the guard allows the command.
    fn reason(&self, command: &str, cwd: &Path, extra_env: &[(&str, &str)]) -> Option<String> {
        let payload = serde_json::json!({ "tool_input": { "command": command } }).to_string();
        let mut spawn = Command::new(env!("CARGO_BIN_EXE_playbook"));
        spawn
            .args(["hook", "rm-workspace-guard"])
            .env("HOOK_INPUT", payload)
            .env("HOME", &self.home)
            .env_remove("PLAYBOOK_SAFE_ROOTS")
            .env_remove("WORKTREE_BASE_DIR")
            .env_remove("PLAYBOOK_BIN_DIR")
            .env_remove("XDG_CACHE_HOME")
            .env_remove("GIT_DIR")
            .env_remove("GIT_WORK_TREE")
            .current_dir(cwd);
        for (k, v) in extra_env {
            spawn.env(k, v);
        }
        let out = spawn.output().expect("spawn");
        assert!(out.status.success(), "a guard exits 0 even when denying");
        let stdout = String::from_utf8_lossy(&out.stdout).to_string();
        stdout
            .contains(r#""permissionDecision":"deny""#)
            .then_some(stdout)
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
        // After a cd the target is judged where it lands.
        (format!("cd {} && {d} -rf build", s(&fx.wt1)), false),
        (format!("cd {} && {d} -rf ../..", s(&fx.wt1)), true),
        (format!("cd \"$X\" && {d} -rf build"), true),
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
        (format!("{d} -rf $OTHER/x"), true),
        (format!("{d} -rf \"$OTHER\"/x"), true),
        (format!("{d} -rf $(mktemp -d)"), true),
        (format!("{d} -rf \"/etc/x\""), true),
        (format!("{d} -rf '/tmp/ok\\ x'"), true),
        (format!("{d} -rf {t}/with\\ space"), false),
        (format!("{d} -rf \"{t}/two words\""), false),
    ];
    if cfg!(target_os = "macos") {
        cases.push((
            format!("{d} -rf /var/folders/ab/cdef0123/T/tmp.AbC123"),
            false,
        ));
        cases.push((format!("{d} -rf /var/folders/ab/cdef0123/T"), true));
    }
    check(&fx, &fx.repo, &cases);
    // `$TMPDIR` takes the hook's value, and the result is still judged.
    let env = [("TMPDIR", t.as_str())];
    assert!(!fx.denied(&format!("{d} -rf $TMPDIR/x"), &fx.repo, &env));
    assert!(fx.denied(&format!("{d} -rf $TMPDIR"), &fx.repo, &[("TMPDIR", "/tmp")]));
    assert!(fx.denied(&format!("{d} -rf $TMPDIR/x"), &fx.repo, &[("TMPDIR", "/")]));
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
        r#"["/a/*/b"]"#,
        r#"["/a/b**"]"#,
        r#"["/a/b/*.log"]"#,
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
        r#"["/a/b/*"]"#,
        "",
    ] {
        let out = fx.config_set(good);
        assert!(out.status.success(), "should accept {good:?}: {out:?}");
    }
    let out = fx.config_set("/opt/$USER/x");
    let err = String::from_utf8_lossy(&out.stderr);
    assert!(err.contains("covers everything under it"), "{err}");
    assert!(err.contains("playbook config set --global"), "{err}");
}

/// The exact form the user typed, with a trailing `/**` on each root, is
/// accepted, stored as plain paths and honoured by the guard.
#[test]
fn a_root_with_a_trailing_double_star_is_accepted_and_honoured() {
    let fx = Fx::new();
    let d = rm();
    let w = s(&fx.base.join("data/scratch"));
    let value = format!(r#"["~/code/acme/.worktrees/**","{w}/**"]"#);

    let out = fx.config_set(&value);

    assert!(out.status.success(), "{out:?}");
    let plain = format!(r#"["~/code/acme/.worktrees","{w}"]"#);
    let printed = String::from_utf8_lossy(&out.stdout);
    assert!(
        printed.contains(&plain),
        "success prints the list: {printed}"
    );
    let stored = std::process::Command::new(env!("CARGO_BIN_EXE_playbook"))
        .args(["config", "get", "hooks.rmGuard.allowedRoots"])
        .env("HOME", &fx.home)
        .output()
        .expect("config get");
    let stored = String::from_utf8_lossy(&stored.stdout);
    assert!(stored.contains("~/code/acme/.worktrees"), "{stored}");
    assert!(!stored.contains('*'), "stored without the glob: {stored}");
    assert!(!fx.denied(&format!("{d} -rf {w}/x"), &fx.repo, &[]));
    assert!(fx.denied(&format!("{d} -rf {w}"), &fx.repo, &[]));
    // A single `/*` means the same.
    assert!(fx.config_set(&format!(r#"["{w}/*"]"#)).status.success());
    assert!(!fx.denied(&format!("{d} -rf {w}/x"), &fx.repo, &[]));
    assert!(fx.denied(&format!("{d} -rf {w}"), &fx.repo, &[]));
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
    if cfg!(target_os = "macos") {
        assert!(text.contains("/private/var/folders/*/*/T/*"), "{text}");
    }
    for needle in [
        "/tmp/*",
        "/**/tmp/*",
        ".worktrees/*",
        "rule: outside the allowed roots",
        "No config root can cover this path",
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
        (format!("cd {t} && {d} -rf sub"), false),
        (format!("cd /tmp && {d} -rf ."), true),
    ];
    check(&fx, &fx.repo, &cases);
}

/// Playbook's data stays blocked even when a configured root covers it, and
/// its scratch folders are allowed with no config.
#[test]
fn playbook_data_is_protected_and_its_scratch_folders_are_allowed() {
    let fx = Fx::new();
    let d = rm();
    let pb = s(&fx.home.join(".config/playbook"));
    fs::create_dir_all(fx.home.join(".config/playbook/memory/acme/app")).expect("memory");
    fs::create_dir_all(fx.home.join(".config/playbook/data")).expect("data");
    let cases: Vec<(String, bool)> = vec![
        (format!("{d} -rf {pb}/tmp/doc-1"), false),
        (format!("{d} -rf {pb}/staging/x"), false),
        (format!("{d} -rf {pb}/runtime/s1"), true),
        (format!("{d} -f {pb}/runtime/s1/auto-cost.json"), true),
        (format!("{d} -rf {pb}/embedded/0.1.0"), false),
        (format!("{d} -rf {pb}/repos-cache/tmp/x"), false),
        (format!("{d} -rf ~/.config/playbook/tmp/doc-1"), false),
        (format!("{d} -rf {pb}/memory/tmp/x"), true),
        (format!("{d} -rf {pb}/memory/acme/app"), true),
        (format!("{d} -rf ~/.config/playbook/memory"), true),
        (format!("{d} -f {pb}/data/config.db"), true),
        (format!("{d} -f {pb}/data/memory.db"), true),
        (format!("{d} -rf {pb}/repos/acme/app/plans"), true),
        (format!("{d} -rf {pb}/runtime/handoff/x"), true),
        (format!("{d} -rf {pb}/runtime/s1/edits.jsonl.lock"), true),
        (format!("{d} -rf {pb}/runtime/s1/*.lock"), true),
        (format!("{d} -rf {pb}/embedded/ver-*"), false),
        (format!("{d} -rf {pb}/runtime/han*"), true),
        (format!("{d} -f {}/Cargo.lock", s(&fx.repo)), false),
        (format!("{d} -rf {pb}"), true),
        (format!("{d} -rf {pb}/*"), true),
        (format!("{d} -rf {pb}/tmp/../memory"), true),
    ];
    check(&fx, &fx.repo, &cases);
    assert!(fx.denied(&format!("{d} -rf {pb}/tmp"), &fx.repo, &[]));
    // A configured root over the whole folder does not open the data.
    assert!(fx.config_set(r#"["~/.config/playbook"]"#).status.success());
    check(&fx, &fx.repo, &cases);
    // Nor does a safe root over the whole home.
    let h = s(&fx.home);
    let safe = [("PLAYBOOK_SAFE_ROOTS", h.as_str())];
    for (cmd, want) in &cases {
        if *want {
            assert!(fx.denied(cmd, &fx.repo, &safe), "{cmd}");
        }
    }
    assert!(fx.denied(&format!("{d} -rf {h}/.config"), &fx.repo, &safe));
    let why = fx
        .reason(&format!("{d} -rf {pb}/memory"), &fx.repo, &[])
        .expect("denied");
    assert!(why.contains("rule: protected playbook data"), "{why}");
    assert!(
        !why.contains("config set"),
        "no fix for protected data: {why}"
    );
}

/// Backups and staging files of the install are allowed by name. The binary
/// and its links stay blocked, because every hook runs the binary.
#[test]
fn playbook_install_files_are_allowed_by_name() {
    let fx = Fx::new();
    let d = rm();
    let bin = fx.home.join(".local/bin");
    fs::create_dir_all(&bin).expect("bin");
    fs::write(bin.join("playbook"), "").expect("binary");
    symlink("playbook", bin.join("pb")).expect("pb link");
    let b = s(&bin);
    let cases: Vec<(String, bool)> = vec![
        (format!("{d} -f {b}/playbook"), true),
        (format!("{d} -f {b}/pb"), true),
        (format!("{d} -f {b}/pbtop"), true),
        (format!("{d} -f {b}/playbook.0.36.0.bak"), false),
        (format!("{d} -f {b}/.playbook.update.1.2"), false),
        (format!("{d} -f {b}/playbook.*.bak"), false),
        (format!("{d} -f ~/.local/bin/playbook.0.36.0.bak"), false),
        (format!("{d} -f {b}/other"), true),
        (format!("{d} -f {b}/*"), true),
        (format!("{d} -f {b}/p*"), true),
        (format!("{d} -rf {b}"), true),
        (format!("{d} -f {b}/sub/playbook"), true),
    ];
    check(&fx, &fx.repo, &cases);
    // An install name that links out of every root is judged where it points.
    fs::write(fx.home.join("notes"), "").expect("notes");
    symlink(fx.home.join("notes"), bin.join("playbook.9.9.9.bak")).expect("bak link");
    assert!(fx.denied(&format!("{d} -f {b}/playbook.9.9.9.bak"), &fx.repo, &[]));
    // `PLAYBOOK_BIN_DIR` moves the allowance with the install.
    let other = fx.base.join("bin2");
    let env = [("PLAYBOOK_BIN_DIR", other.to_str().expect("utf8"))];
    let bak = "playbook.0.36.0.bak";
    assert!(!fx.denied(&format!("{d} -f {}/{bak}", s(&other)), &fx.repo, &env));
    assert!(fx.denied(&format!("{d} -f {b}/{bak}"), &fx.repo, &env));
}

#[cfg(target_os = "macos")]
#[test]
fn macos_caches_and_the_collector_agent_are_allowed() {
    let fx = Fx::new();
    let d = rm();
    let h = s(&fx.home);
    let agent = "io.github.pragmatic-engineer.playbook.collector.plist";
    let cases: Vec<(String, bool)> = vec![
        (format!("{d} -rf {h}/Library/Caches/some-app"), false),
        (format!("{d} -rf {h}/.cache/playbook"), false),
        (format!("{d} -f {h}/Library/LaunchAgents/{agent}"), false),
        (format!("{d} -rf {h}/Library/Caches"), true),
        (format!("{d} -rf {h}/Library/Caches/*"), true),
        (format!("{d} -rf {h}/Library/Preferences/x"), true),
        (format!("{d} -f {h}/Library/LaunchAgents/other.plist"), true),
    ];
    check(&fx, &fx.repo, &cases);
}

#[cfg(target_os = "linux")]
#[test]
fn linux_temp_folders_and_the_collector_unit_are_allowed() {
    let fx = Fx::new();
    let d = rm();
    let h = s(&fx.home);
    let cases: Vec<(String, bool)> = vec![
        (format!("{d} -rf /var/tmp/pb-x"), false),
        (format!("{d} -rf {h}/.cache/playbook"), false),
        (
            format!("{d} -f {h}/.config/systemd/user/playbook-collector.service"),
            false,
        ),
        (format!("{d} -rf /var/tmp"), true),
        (
            format!("{d} -f {h}/.config/systemd/user/other.service"),
            true,
        ),
    ];
    check(&fx, &fx.repo, &cases);
}

/// `$TMPDIR` as the hook sees it is a root, contents only, and a `..` out of
/// it is judged where it lands.
#[test]
fn the_tmpdir_of_the_hook_is_a_root_contents_only() {
    let fx = Fx::new();
    let d = rm();
    let t = fx.base.join("own-tmp");
    fs::create_dir_all(&t).expect("tmpdir");
    let ts = s(&t);
    let env = [("TMPDIR", ts.as_str())];
    let cases: Vec<(String, bool)> = vec![
        (format!("{d} -rf {ts}/x"), false),
        (format!("{d} -rf {ts}/x/"), false),
        (format!("{d} -rf {ts}"), true),
        (format!("{d} -rf {ts}/"), true),
        (format!("{d} -rf {ts}/../home/.ssh"), true),
        (format!("{d} -rf /tmp/../etc"), true),
    ];
    for (cmd, want) in cases {
        assert_eq!(fx.denied(&cmd, &fx.repo, &env), want, "{cmd}");
    }
}

/// A path outside every root gets the exact command that keeps the current
/// roots and adds its folder.
#[test]
fn the_denial_names_the_rule_and_gives_the_exact_fix_command() {
    let fx = Fx::new();
    let d = rm();
    assert!(fx.config_set(r#"["~/scratch/builds"]"#).status.success());
    let target = fx.home.join("code/acme/old");

    let why = fx
        .reason(&format!("{d} -rf {}", s(&target)), &fx.repo, &[])
        .expect("denied");

    assert!(why.contains("rule: outside the allowed roots"), "{why}");
    let want = r#"! playbook config set --global hooks.rmGuard.allowedRoots '[\"~/scratch/builds\",\"~/code/acme\"]'"#;
    assert!(why.contains(want), "{why}");
    // A dot folder in home is never offered as a root.
    let why = fx
        .reason(&format!("{d} -f ~/.ssh/id_rsa"), &fx.repo, &[])
        .expect("denied");
    assert!(!why.contains("config set"), "{why}");
}

/// On a case-insensitive disk a different spelling of protected data is the
/// same folder, so it stays blocked.
#[cfg(target_os = "macos")]
#[test]
fn a_different_letter_case_cannot_reach_protected_data() {
    let fx = Fx::new();
    let d = rm();
    let pb = s(&fx.home.join(".config/playbook"));
    fs::create_dir_all(fx.home.join(".config/playbook/memory")).expect("memory");
    fs::create_dir_all(fx.home.join(".config/playbook/runtime/handoff")).expect("handoff");
    assert!(fx.config_set(r#"["~/.config/playbook"]"#).status.success());
    for cmd in [
        format!("{d} -rf {pb}/MEMORY"),
        format!("{d} -rf {pb}/Memory/x"),
        format!("{d} -rf {pb}/runtime/HANDOFF"),
    ] {
        assert!(fx.denied(&cmd, &fx.repo, &[]), "{cmd}");
    }
}
