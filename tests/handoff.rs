// SPDX-FileCopyrightText: 2026 Igor Santos
// SPDX-License-Identifier: MIT

//! Binary-level tests for `playbook handoff` and the SessionStart hook's
//! handoff load and log. Every test uses a scratch HOME, never the real one.

use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicU64, Ordering};

static COUNTER: AtomicU64 = AtomicU64::new(0);

struct Env {
    home: PathBuf,
    cwd: PathBuf,
}

impl Env {
    fn new(tag: &str) -> Env {
        let n = COUNTER.fetch_add(1, Ordering::Relaxed);
        let root =
            std::env::temp_dir().join(format!("pbk-handoff-{}-{n}-{tag}", std::process::id()));
        let home = root.join("home");
        let cwd = root.join("work").join("repo");
        fs::create_dir_all(&home).unwrap();
        fs::create_dir_all(&cwd).unwrap();
        Env {
            home: home.canonicalize().unwrap(),
            cwd: cwd.canonicalize().unwrap(),
        }
    }

    fn run_in(&self, cwd: &Path, args: &[&str], stdin: &str) -> (i32, String, String) {
        let mut child = Command::new(env!("CARGO_BIN_EXE_playbook"))
            .env_remove("CI")
            .env_remove("PLAYBOOK_HEADLESS")
            .args(args)
            .current_dir(cwd)
            .env("HOME", &self.home)
            .env("PWD", cwd)
            .env("CLAUDE_PLUGIN_ROOT", env!("CARGO_MANIFEST_DIR"))
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .unwrap();
        child
            .stdin
            .take()
            .unwrap()
            .write_all(stdin.as_bytes())
            .unwrap();
        let out = child.wait_with_output().unwrap();
        (
            out.status.code().unwrap_or(-1),
            String::from_utf8_lossy(&out.stdout).into_owned(),
            String::from_utf8_lossy(&out.stderr).into_owned(),
        )
    }

    fn run(&self, args: &[&str], stdin: &str) -> (i32, String, String) {
        self.run_in(&self.cwd, args, stdin)
    }

    fn session_start(&self, cwd: &Path, source: &str) -> String {
        let payload = format!(
            r#"{{"hook_event_name":"SessionStart","source":"{source}","session_id":"sid-{source}"}}"#
        );
        let (code, stdout, _) = self.run_in(cwd, &["hook", "session-init"], &payload);
        assert_eq!(code, 0);
        stdout
    }

    fn handoff_dir(&self) -> PathBuf {
        self.home.join(".config/playbook/runtime/handoff")
    }

    fn log(&self) -> String {
        fs::read_to_string(self.home.join(".config/playbook/runtime/session-start.log"))
            .unwrap_or_default()
    }
}

#[test]
fn save_then_show_round_trips_without_consuming() {
    let env = Env::new("roundtrip");

    let (code, out, _) = env.run(&["handoff", "save"], "# Topic\nNEXT-STEP-ONE\n");
    let (_, shown_once, _) = env.run(&["handoff", "show"], "");
    let (_, shown_twice, _) = env.run(&["handoff", "show"], "");

    assert_eq!(code, 0);
    assert!(out.starts_with("handoff saved: "), "{out}");
    assert!(shown_once.contains("NEXT-STEP-ONE"), "{shown_once}");
    assert!(
        shown_twice.contains("NEXT-STEP-ONE"),
        "show must not consume: {shown_twice}"
    );
}

#[test]
fn save_refuses_empty_input_and_writes_nothing() {
    let env = Env::new("empty");

    let (code, _, err) = env.run(&["handoff", "save"], "   \n");

    assert_eq!(code, 1);
    assert!(err.contains("empty handoff"), "{err}");
    assert!(!env.handoff_dir().exists() || fs::read_dir(env.handoff_dir()).unwrap().count() == 0);
}

#[cfg(unix)]
#[test]
fn saved_files_and_their_directory_are_owner_only() {
    use std::os::unix::fs::PermissionsExt;
    let env = Env::new("mode");

    env.run(&["handoff", "save"], "# x\n");

    let mode = |p: &Path| fs::metadata(p).unwrap().permissions().mode() & 0o777;
    assert_eq!(mode(&env.handoff_dir()), 0o700);
    let file = fs::read_dir(env.handoff_dir())
        .unwrap()
        .next()
        .unwrap()
        .unwrap()
        .path();
    assert_eq!(mode(&file), 0o600);
}

#[test]
fn dir_works_from_an_unrelated_working_directory() {
    let env = Env::new("dir");
    let elsewhere = env.home.join("elsewhere");
    fs::create_dir_all(&elsewhere).unwrap();
    let target = env.cwd.to_str().unwrap();

    let (code, _, _) = env.run_in(
        &elsewhere,
        &["handoff", "save", "--dir", target],
        "# Topic\nVIA-DIR\n",
    );
    let (_, shown, _) = env.run_in(&elsewhere, &["handoff", "show", "--dir", target], "");
    let (_, here, _) = env.run(&["handoff", "show"], "");

    assert_eq!(code, 0);
    assert!(shown.contains("VIA-DIR"), "{shown}");
    assert!(
        here.contains("VIA-DIR"),
        "the directory's own session sees it: {here}"
    );
}

#[test]
fn a_bad_dir_fails_with_a_clear_message() {
    let env = Env::new("baddir");

    let (code, _, err) = env.run(
        &["handoff", "save", "--dir", "/no/such/dir/anywhere"],
        "# x\n",
    );

    assert_eq!(code, 1);
    assert!(err.contains("is not a directory"), "{err}");
}

#[test]
fn a_clear_start_injects_the_handoff_archives_it_and_logs_the_start() {
    let env = Env::new("clear");
    env.run(&["handoff", "save"], "# Topic\nINJECT-ME\n");

    let stdout = env.session_start(&env.cwd, "clear");

    assert!(
        stdout.contains("INJECT-ME"),
        "the hook must inject it: {stdout}"
    );
    assert_eq!(
        fs::read_dir(env.handoff_dir().join("used"))
            .unwrap()
            .count(),
        1
    );
    let (_, shown, _) = env.run(&["handoff", "show"], "");
    assert!(
        shown.contains("already loaded by the last session start"),
        "{shown}"
    );
    assert!(shown.contains("INJECT-ME"), "{shown}");
    let log = env.log();
    let fields: Vec<&str> = log.lines().last().unwrap().split('\t').collect();
    assert_eq!(fields.len(), 5, "{log}");
    assert_eq!(
        (fields[1], fields[2], fields[4]),
        ("clear", "sid-clear", "1")
    );
}

#[test]
fn a_start_with_no_handoff_still_logs_with_zero_injected() {
    let env = Env::new("none");

    env.session_start(&env.cwd, "startup");

    let log = env.log();
    let fields: Vec<&str> = log.lines().last().unwrap().split('\t').collect();
    assert_eq!((fields[1], fields[4]), ("startup", "0"), "{log}");
}

#[test]
fn another_directorys_handoff_is_listed_by_show_but_never_injected() {
    let env = Env::new("other");
    let other = env.home.join("work").join("other-repo");
    fs::create_dir_all(&other).unwrap();
    env.run_in(&other, &["handoff", "save"], "# Other topic\nOTHER-ONLY\n");

    let stdout = env.session_start(&env.cwd, "clear");
    let (_, shown, _) = env.run(&["handoff", "show"], "");

    assert!(!stdout.contains("OTHER-ONLY"), "{stdout}");
    assert!(
        shown.starts_with("No handoff saved for this directory."),
        "{shown}"
    );
    assert!(shown.contains("Other topic"), "{shown}");
}

#[test]
fn a_child_directory_handoff_is_not_loaded_by_its_parent() {
    let env = Env::new("child");
    let child = env.cwd.join("sub");
    fs::create_dir_all(&child).unwrap();
    env.run_in(&child, &["handoff", "save"], "# Child\nCHILD-ONLY\n");

    let stdout = env.session_start(&env.cwd, "clear");

    assert!(!stdout.contains("CHILD-ONLY"), "{stdout}");
}

#[test]
fn status_shows_the_clear_event_and_the_counts() {
    let env = Env::new("status");
    env.run(&["handoff", "save"], "# Topic\nSTATUS\n");
    env.session_start(&env.cwd, "clear");
    env.run(&["handoff", "save"], "# Second\nSTATUS-2\n");

    let (code, out, _) = env.run(&["handoff", "status"], "");

    assert_eq!(code, 0);
    assert!(out.contains("clear"), "{out}");
    assert!(out.contains("1 unread, 1 already loaded"), "{out}");
    assert!(out.contains("/playbook:session-start"), "{out}");
}

#[cfg(unix)]
#[test]
fn a_read_only_runtime_dir_does_not_fail_the_hook() {
    use std::os::unix::fs::PermissionsExt;
    let env = Env::new("ro");
    let runtime = env.home.join(".config/playbook/runtime");
    fs::create_dir_all(&runtime).unwrap();
    fs::set_permissions(&runtime, fs::Permissions::from_mode(0o500)).unwrap();

    let (code, _, _) = env.run(
        &["hook", "session-init"],
        r#"{"source":"clear","session_id":"ro"}"#,
    );

    fs::set_permissions(&runtime, fs::Permissions::from_mode(0o700)).unwrap();
    assert_eq!(
        code, 0,
        "logging is best-effort and must never fail the hook"
    );
}
