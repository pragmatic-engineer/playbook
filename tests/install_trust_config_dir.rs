// SPDX-FileCopyrightText: 2026 Igor Santos
// SPDX-License-Identifier: Apache-2.0

//! After `playbook init`, `install.sh` marks `~/.config/playbook` as trusted in
//! `~/.claude.json` through `playbook trust`. It is a best-effort step: an
//! older binary or a failure warns and never fails the install. The `playbook`
//! on PATH is a stub that records its trust calls, or runs the real binary.

#[path = "support/shell_fixture.rs"]
mod fx;

use fx::{repo_root, stderr, stdout, write_exec, Work};
use serde_json::Value;
use std::fs;
use std::path::PathBuf;
use std::process::Command;

#[derive(Clone, Copy)]
enum Trust {
    Ok,
    Old,
    Fail,
}

struct Case {
    work: Work,
    home: PathBuf,
    record: PathBuf,
    log: String,
    rc: i32,
}

fn stub(trust: Trust, record: &std::path::Path) -> String {
    let real = env!("CARGO_BIN_EXE_playbook");
    let trust_arm = match trust {
        Trust::Old => "echo \"error: unrecognized subcommand 'trust'\" >&2\n        exit 2".to_string(),
        Trust::Fail => format!(
            "[ \"${{2:-}}\" = \"--help\" ] && exit 0\n        printf 'trust %s\\n' \"${{*:2}}\" >> \"{}\"\n        echo \"playbook trust: simulated write failure\" >&2\n        exit 0",
            record.display()
        ),
        Trust::Ok => format!(
            "[ \"${{2:-}}\" = \"--help\" ] && exit 0\n        printf 'trust %s\\n' \"${{*:2}}\" >> \"{}\"\n        exec \"{real}\" trust \"${{@:2}}\"",
            record.display()
        ),
    };
    format!(
        "case \"${{1:-}}\" in\n  --version) printf 'playbook 0.16.0\\n' ;;\n  init)\n    mkdir -p \"$CLAUDE_HOME\"\n    printf '{{}}' > \"$CLAUDE_HOME/settings.json\"\n    if [[ \"$*\" == *--help* ]]; then\n      printf -- '--aliases --system-prompt\\n'\n    fi\n    ;;\n  trust)\n        {trust_arm}\n    ;;\nesac\nexit 0\n"
    )
}

/// Run the installer with a PATH of only the stub dir and symlinks to the few
/// system tools it needs, so nothing on the host leaks in.
fn run(tag: &str, trust: Trust, claude_json: Option<&str>) -> Case {
    run_seeded(tag, trust, |_| claude_json.map(str::to_string))
}

/// Like `run`, with `~/.claude.json` content built from the home path.
fn run_seeded(tag: &str, trust: Trust, seed: impl Fn(&std::path::Path) -> Option<String>) -> Case {
    let work = Work::new(tag);
    let src = work.dir("src");
    let home = work.dir("home");
    let bindir = work.dir("bin");
    let tools = work.dir("tools");
    let record = work.path("trust.record");
    fs::write(&record, "").unwrap();
    write_exec(&bindir.join("playbook"), &stub(trust, &record));
    for tool in [
        "bash",
        "curl",
        "tar",
        "shasum",
        "sha256sum",
        "mktemp",
        "mv",
        "chmod",
        "rm",
        "cat",
        "grep",
        "sed",
        "awk",
        "basename",
        "dirname",
        "mkdir",
        "cp",
        "find",
        "date",
        "uname",
        "sort",
        "tail",
    ] {
        if let Ok(real) = which(tool) {
            #[cfg(unix)]
            let _ = std::os::unix::fs::symlink(real, tools.join(tool));
        }
    }
    if let Some(body) = seed(&home) {
        fs::write(home.join(".claude.json"), body).unwrap();
    }
    let out = Command::new("bash")
        .arg(repo_root().join("install.sh"))
        .args(["--yes", "--skip-plugin"])
        .env_remove("SHELL")
        .env("PLAYBOOK_SRC", &src)
        .env("PLAYBOOK_BIN_DIR", &bindir)
        .env("CLAUDE_HOME", home.join(".claude"))
        .env("HOME", &home)
        .env("PATH", format!("{}:{}", bindir.display(), tools.display()))
        .output()
        .expect("bash spawns");
    let log = format!("{}{}", stdout(&out), stderr(&out));
    Case {
        work,
        home,
        record,
        log,
        rc: out.status.code().unwrap_or(-1),
    }
}

fn which(tool: &str) -> Result<PathBuf, ()> {
    let path = std::env::var_os("PATH").ok_or(())?;
    std::env::split_paths(&path)
        .map(|d| d.join(tool))
        .find(|p| p.is_file())
        .ok_or(())
}

fn claude_json(c: &Case) -> String {
    fs::read_to_string(c.home.join(".claude.json")).unwrap_or_default()
}

fn project_field(c: &Case, project: &str, field: &str) -> Value {
    let v: Value = serde_json::from_str(&claude_json(c)).unwrap();
    v["projects"][project][field].clone()
}

#[test]
fn no_claude_json_is_a_clean_noop() {
    let c = run("trust-none", Trust::Ok, None);
    assert_eq!(c.rc, 0, "{}", c.log);
    assert!(!c.home.join(".claude.json").exists(), "file was created");
    assert!(fs::read_to_string(&c.record).unwrap().is_empty());
    let _ = &c.work;
}

#[test]
fn a_trust_capable_binary_is_called_once_with_the_config_dir() {
    let c = run("trust-call", Trust::Ok, Some("{}"));
    assert_eq!(c.rc, 0, "{}", c.log);
    let want = format!("trust {}/.config/playbook", c.home.display());
    assert_eq!(fs::read_to_string(&c.record).unwrap().trim_end(), want);
    let dir = format!("{}/.config/playbook", c.home.display());
    assert_eq!(
        project_field(&c, &dir, "hasTrustDialogAccepted"),
        Value::Bool(true)
    );
}

#[test]
fn an_existing_entry_is_updated_without_clobbering_siblings_or_other_projects() {
    let c = run_seeded("trust-existing", Trust::Ok, |home| {
        Some(format!(
            "{{\"projects\":{{\"{0}/.config/playbook\":{{\"hasTrustDialogAccepted\":false,\"lastCost\":1.23}},\"{0}/some/other/project\":{{\"hasTrustDialogAccepted\":false}}}}}}",
            home.display()
        ))
    });
    assert_eq!(c.rc, 0, "{}", c.log);
    let cfg = format!("{}/.config/playbook", c.home.display());
    let other = format!("{}/some/other/project", c.home.display());
    assert_eq!(
        project_field(&c, &cfg, "hasTrustDialogAccepted"),
        Value::Bool(true)
    );
    assert_eq!(project_field(&c, &cfg, "lastCost"), serde_json::json!(1.23));
    assert_eq!(
        project_field(&c, &other, "hasTrustDialogAccepted"),
        Value::Bool(false)
    );
}

#[test]
fn an_older_binary_warns_but_does_not_fail() {
    let c = run("trust-old", Trust::Old, Some("{}"));
    assert_eq!(
        c.rc, 0,
        "a best-effort step must not fail the installer: {}",
        c.log
    );
    assert!(fs::read_to_string(&c.record).unwrap().is_empty());
    assert_eq!(
        c.log.matches("has no 'trust' command").count(),
        1,
        "{}",
        c.log
    );
    assert!(c.log.contains("playbook trust"), "{}", c.log);
    assert_eq!(claude_json(&c), "{}");
}

#[test]
fn a_trust_failure_is_surfaced_as_a_warning() {
    let c = run("trust-fail", Trust::Fail, Some("{}"));
    assert_eq!(c.rc, 0, "{}", c.log);
    assert!(c.log.contains("simulated write failure"), "{}", c.log);
    assert_eq!(claude_json(&c), "{}");
}
