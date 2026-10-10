// SPDX-FileCopyrightText: 2026 Igor Santos
// SPDX-License-Identifier: Apache-2.0

//! The hook fire matrix. `install.sh --yes --skip-plugin` wires settings.json
//! through the real `playbook init`. Every distinct hook name found there must
//! have a case below that fires the installed binary on a real payload and
//! checks the observable effect, so a hook that is wired but dead, or wired
//! with no case, fails here.

#[path = "support/shell_fixture.rs"]
mod fx;

use fx::{bin_dir_with_playbook, both, repo_root, Work};
use serde_json::Value;
use std::collections::BTreeSet;
use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

struct Fire {
    work: Work,
    playbook: PathBuf,
    home: PathBuf,
}

/// stdout of `playbook hook <name>` fed `payload`, run from `cwd` with `home`
/// as HOME and the given extra env.
fn run_hook(
    playbook: &Path,
    name: &str,
    payload: &str,
    home: &Path,
    cwd: &Path,
    envs: &[(&str, &str)],
    strip: &[&str],
) -> (bool, String) {
    let mut cmd = Command::new(playbook);
    cmd.args(["hook", name])
        .current_dir(cwd)
        .env("HOME", home)
        .env("PLAYBOOK_HEADLESS", "0")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null());
    for k in strip {
        cmd.env_remove(k);
    }
    for (k, v) in envs {
        cmd.env(k, v);
    }
    let mut child = cmd.spawn().expect("hook spawns");
    child
        .stdin
        .take()
        .unwrap()
        .write_all(payload.as_bytes())
        .unwrap();
    let out = child.wait_with_output().unwrap();
    (
        out.status.success(),
        String::from_utf8_lossy(&out.stdout).into_owned(),
    )
}

fn installed() -> Fire {
    let work = Work::new("fire");
    let bin = bin_dir_with_playbook(&work);
    let home = work.dir("home");
    let out = Command::new("bash")
        .arg(repo_root().join("install.sh"))
        .args(["--yes", "--skip-plugin"])
        .env("PLAYBOOK_HEADLESS", "0")
        .env("HOME", &home)
        .env("CLAUDE_HOME", home.join(".claude"))
        .env("PLAYBOOK_SRC", repo_root())
        .env("PLAYBOOK_BIN_DIR", &bin)
        .output()
        .unwrap();
    assert!(out.status.success(), "install failed: {}", both(&out));
    Fire {
        playbook: bin.join("playbook"),
        home,
        work,
    }
}

/// Every `playbook hook <name>` command in a settings value.
fn hook_names(settings: &Value) -> BTreeSet<String> {
    let mut names = BTreeSet::new();
    let mut stack = vec![settings.get("hooks").cloned().unwrap_or(Value::Null)];
    while let Some(v) = stack.pop() {
        match v {
            Value::Object(map) => {
                if let Some(cmd) = map.get("command").and_then(Value::as_str) {
                    if let Some(name) = cmd.strip_prefix("playbook hook ") {
                        names.insert(name.split_whitespace().next().unwrap_or("").to_string());
                    }
                }
                stack.extend(map.into_values());
            }
            Value::Array(items) => stack.extend(items),
            _ => {}
        }
    }
    names
}

fn json_str(s: &str) -> String {
    serde_json::to_string(s).unwrap()
}

fn scratch(f: &Fire, tag: &str) -> PathBuf {
    let dir = f.work.dir(tag);
    fs::canonicalize(&dir).unwrap_or(dir)
}

fn runtime(home: &Path, sid: &str) -> PathBuf {
    home.join(".config/playbook/runtime").join(sid)
}

fn fires(f: &Fire, name: &str) -> bool {
    match name {
        "session-init" => {
            let home = scratch(f, "h-session-init");
            run_hook(
                &f.playbook,
                name,
                r#"{"session_id":"fire-session-init"}"#,
                &home,
                &home,
                &[],
                &[],
            );
            fs::read_to_string(runtime(&home, "fire-session-init").join("start-ts"))
                .is_ok_and(|t| !t.trim().is_empty() && t.trim().chars().all(|c| c.is_ascii_digit()))
        }
        "preread-edit-check" => {
            let home = scratch(f, "h-preread-edit");
            let file = home.join("target.txt");
            fs::write(&file, "hello\n").unwrap();
            let dir = runtime(&home, "fire-preread-edit");
            fs::create_dir_all(&dir).unwrap();
            let ts = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_secs()
                - 10;
            fs::write(
                dir.join("edits.jsonl"),
                format!(
                    "{{\"path\":{},\"ts\":{ts}}}\n",
                    json_str(&file.display().to_string())
                ),
            )
            .unwrap();
            let payload = format!(
                "{{\"session_id\":\"fire-preread-edit\",\"tool_input\":{{\"file_path\":{}}}}}",
                json_str(&file.display().to_string())
            );
            run_hook(&f.playbook, name, &payload, &home, &home, &[], &[])
                .1
                .contains("ago")
        }
        "preread-size-check" => {
            let home = scratch(f, "h-preread-size");
            let file = home.join("big.log");
            fs::write(
                &file,
                (1..=1500).map(|n| format!("{n}\n")).collect::<String>(),
            )
            .unwrap();
            let payload = format!(
                "{{\"tool_input\":{{\"file_path\":{}}}}}",
                json_str(&file.display().to_string())
            );
            let out = run_hook(&f.playbook, name, &payload, &home, &home, &[], &[]).1;
            out.contains("\"permissionDecision\":\"deny\"") && out.contains("1500 lines")
        }
        "search-counter" => {
            let home = scratch(f, "h-search");
            for _ in 0..4 {
                run_hook(
                    &f.playbook,
                    name,
                    r#"{"session_id":"fire-search-counter","tool_name":"Grep"}"#,
                    &home,
                    &home,
                    &[],
                    &[],
                );
            }
            fs::read_to_string(runtime(&home, "fire-search-counter").join("search-count"))
                .is_ok_and(|t| t.trim() == "4")
        }
        "memory-anchors" => {
            let home = scratch(f, "h-anchors");
            let mem = home.join(".config/playbook/memory");
            fs::create_dir_all(&mem).unwrap();
            fs::write(
                mem.join("memory.graph.json"),
                r#"{
  "nodes": [
    {"id": "global/fire-anchor-fact", "file": "fire-anchor-fact.md", "scope": "global", "type": "project", "name": "fire-anchor-fact", "description": "fixture fact"},
    {"id": "code:Cargo.toml", "file": "Cargo.toml", "scope": "code", "type": "code"}
  ],
  "edges": [
    {"from": "global/fire-anchor-fact", "to": "code:Cargo.toml", "relation": "anchors"}
  ]
}"#,
            )
            .unwrap();
            let target = repo_root().join("Cargo.toml");
            let payload = format!(
                "{{\"session_id\":\"fire-memory-anchors\",\"tool_name\":\"Edit\",\"tool_input\":{{\"file_path\":{}}}}}",
                json_str(&target.display().to_string())
            );
            run_hook(&f.playbook, name, &payload, &home, &repo_root(), &[], &[])
                .1
                .contains("fire-anchor-fact")
        }
        "post-edit-track" => {
            let home = scratch(f, "h-post-edit");
            run_hook(
                &f.playbook,
                name,
                r#"{"session_id":"fire-post-edit-track","tool_name":"Edit","tool_input":{"file_path":"/tmp/fire-x.txt"}}"#,
                &home,
                &home,
                &[],
                &[],
            );
            fs::read_to_string(runtime(&home, "fire-post-edit-track").join("edit-count"))
                .is_ok_and(|t| t.trim() == "1")
        }
        "rebuild-memory-graph" => {
            let home = scratch(f, "h-rebuild");
            let mem = home.join(".config/playbook/memory");
            fs::create_dir_all(&mem).unwrap();
            let fact = mem.join("fire-fact.md");
            fs::write(&fact, "---\nname: fire-fact\ntype: reference\nlinks:\n  relates_to: fire-other\n---\n\nBody text.\n").unwrap();
            let payload = format!(
                "{{\"tool_input\":{{\"file_path\":{}}}}}",
                json_str(&fact.display().to_string())
            );
            run_hook(&f.playbook, name, &payload, &home, &home, &[], &[]);
            fs::read_to_string(mem.join("memory.graph.json"))
                .ok()
                .and_then(|t| serde_json::from_str::<Value>(&t).ok())
                .is_some_and(|g| g["edges"].as_array().is_some_and(|e| e.len() == 1))
        }
        "auto-model-detect" => {
            let home = scratch(f, "h-model");
            let out = run_hook(
                &f.playbook,
                name,
                r#"{"prompt":"Should we design a new schema and evaluate the tradeoffs between the two approaches?"}"#,
                &home,
                &home,
                &[],
                &[],
            )
            .1;
            !out.is_empty() && out.contains("UserPromptSubmit")
        }
        "precompact-warn" => {
            let home = scratch(f, "h-precompact");
            run_hook(
                &f.playbook,
                name,
                r#"{"trigger":"auto","session_id":"fire-precompact"}"#,
                &home,
                &home,
                &[],
                &[],
            )
            .1
            .contains("(auto)")
        }
        "session-clean-exit" => {
            let home = scratch(f, "h-exit");
            run_hook(
                &f.playbook,
                name,
                r#"{"session_id":"fire-clean-exit","reason":"logout"}"#,
                &home,
                &home,
                &[],
                &[],
            );
            fs::read_to_string(runtime(&home, "fire-clean-exit").join("clean-exit"))
                .is_ok_and(|t| t.trim() == "logout")
        }
        "memory-capture" => {
            let home = scratch(f, "h-capture");
            let dir = runtime(&home, "fire-memory-capture");
            fs::create_dir_all(&dir).unwrap();
            fs::write(dir.join("capture-due"), "").unwrap();
            let out = run_hook(
                &f.playbook,
                name,
                r#"{"session_id":"fire-memory-capture"}"#,
                &home,
                &home,
                &[],
                &[],
            )
            .1;
            serde_json::from_str::<Value>(&out).is_ok_and(|v| v["decision"] == "block")
        }
        "rm-workspace-guard" => {
            let home = f.home.clone();
            let out = run_hook(
                &f.playbook,
                name,
                r#"{"tool_input":{"command":"rm -rf /etc/hosts"}}"#,
                &home,
                &home,
                &[],
                &[],
            )
            .1;
            serde_json::from_str::<Value>(&out)
                .is_ok_and(|v| v["hookSpecificOutput"]["permissionDecision"] == "deny")
        }
        "bg-await-guard" => {
            let home = f.home.clone();
            !run_hook(
                &f.playbook,
                name,
                r#"{"tool_input":{"command":"npm install","run_in_background":true}}"#,
                &home,
                &home,
                &[],
                &[],
            )
            .1
            .is_empty()
        }
        "no-slop-guard" => {
            let home = f.home.clone();
            let cmd = "git commit -m \"x \u{2014} y\"";
            let payload = format!("{{\"tool_input\":{{\"command\":{}}}}}", json_str(cmd));
            run_hook(&f.playbook, name, &payload, &home, &home, &[], &[])
                .1
                .contains("\"permissionDecision\":\"deny\"")
        }
        "policy-guard" => {
            let home = f.home.clone();
            run_hook(
                &f.playbook,
                name,
                r#"{"tool_input":{"command":"git commit --no-verify -m x"}}"#,
                &home,
                &home,
                &[],
                &[],
            )
            .1
            .contains("\"permissionDecision\":\"deny\"")
        }
        "commit-message-sanitizer" => {
            let home = f.home.clone();
            let bad = "git commit -m 'feat: x' -m 'Claude-Session: https://claude.ai/code/session_01AbCdEfGhIjKlMnOpQr'";
            let out = run_hook(
                &f.playbook,
                name,
                &format!("{{\"tool_input\":{{\"command\":{}}}}}", json_str(bad)),
                &home,
                &home,
                &[],
                &[],
            )
            .1;
            if !(out.contains("\"updatedInput\"")
                && !out.contains("permissionDecision")
                && !out.contains("Claude-Session: https"))
            {
                return false;
            }
            let ok = "git commit -s -m 'feat: x' -m 'Refs: PLAT-1'";
            run_hook(
                &f.playbook,
                name,
                &format!("{{\"tool_input\":{{\"command\":{}}}}}", json_str(ok)),
                &home,
                &home,
                &[],
                &[],
            )
            .1
            .is_empty()
        }
        "precommit-check" => {
            let repo = scratch(f, "r-precommit");
            let git = |args: &[&str]| {
                Command::new("git")
                    .arg("-C")
                    .arg(&repo)
                    .args(args)
                    .output()
                    .unwrap()
            };
            git(&["init", "-q"]);
            git(&["config", "user.email", "t@example.com"]);
            git(&["config", "user.name", "Test"]);
            git(&["config", "commit.gpgsign", "false"]);
            fs::write(repo.join(".env"), "TOKEN=abc\n").unwrap();
            git(&["add", "-f", ".env"]);
            let payload = format!(
                "{{\"tool_input\":{{\"command\":{}}}}}",
                json_str("git commit -m 'chore: env'")
            );
            let out = run_hook(&f.playbook, name, &payload, &f.home, &repo, &[], &[]).1;
            !out.is_empty() && !out.contains("\"permissionDecision\":\"deny\"")
        }
        "auto-guard" => {
            let ask = r#"{"session_id":"fire-auto-guard","hook_event_name":"PreToolUse","tool_name":"AskUserQuestion","tool_input":{"questions":[]}}"#;
            let prompt = r#"{"session_id":"fire-auto-guard","hook_event_name":"UserPromptSubmit","permission_mode":"default","prompt":"hi"}"#;
            let auto = |payload: &str, mode: &[(&str, &str)]| {
                let home = scratch(f, "h-auto");
                run_hook(
                    &f.playbook,
                    "auto-guard",
                    payload,
                    &home,
                    &home,
                    mode,
                    &["PLAYBOOK_MODE", "XDG_CONFIG_HOME"],
                )
            };
            let (ok1, out1) = auto(ask, &[]);
            let (ok2, out2) = auto(ask, &[("PLAYBOOK_MODE", "auto")]);
            let (ok3, out3) = auto(prompt, &[("PLAYBOOK_MODE", "auto")]);
            ok1 && out1.is_empty()
                && ok2
                && out2.contains("\"permissionDecision\":\"deny\"")
                && ok3
                && out3.contains("trusted permission mode")
        }
        "auto-cost" => {
            let payload = r#"{"session_id":"fire-auto-cost","hook_event_name":"PreToolUse","tool_name":"Agent","tool_input":{"prompt":"x"}}"#;
            let auto = |mode: &[(&str, &str)]| {
                let home = scratch(f, "h-cost");
                run_hook(
                    &f.playbook,
                    "auto-cost",
                    payload,
                    &home,
                    &home,
                    mode,
                    &["PLAYBOOK_MODE", "XDG_CONFIG_HOME"],
                )
            };
            let (ok1, out1) = auto(&[]);
            let (ok2, out2) = auto(&[("PLAYBOOK_MODE", "auto")]);
            ok1 && out1.is_empty()
                && ok2
                && out2.contains("\"permissionDecision\":\"deny\"")
                && out2.contains("cost unreadable")
        }
        other => panic!("no fire-matrix case registered for hook: {other}"),
    }
}

#[test]
fn every_wired_hook_fires_and_produces_its_observable_effect() {
    let f = installed();
    let settings: Value = serde_json::from_str(
        &fs::read_to_string(f.home.join(".claude/settings.json")).expect("settings.json"),
    )
    .unwrap();
    let names = hook_names(&settings);
    assert!(
        names.len() >= 18,
        "expected 18 distinct hook names, got {}: {names:?}",
        names.len()
    );
    let failed: Vec<&String> = names.iter().filter(|n| !fires(&f, n)).collect();
    assert!(failed.is_empty(), "hooks that did not fire: {failed:?}");
}
