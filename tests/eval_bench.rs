// SPDX-FileCopyrightText: 2026 Igor Santos
// SPDX-License-Identifier: Apache-2.0

//! `playbook eval bench`. `claude` is a stub executable on PATH, so no test
//! makes an API call.

use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::atomic::{AtomicU64, Ordering};

static COUNTER: AtomicU64 = AtomicU64::new(0);

struct Run {
    code: i32,
    out: String,
    err: String,
    dir: PathBuf,
}

fn scratch() -> PathBuf {
    let n = COUNTER.fetch_add(1, Ordering::Relaxed);
    let dir = std::env::temp_dir().join(format!(
        "playbook-bench-{}-{n}",
        playbook::testing::run_id()
    ));
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir).unwrap();
    dir
}

fn stub(dir: &Path, name: &str, body: &str) {
    let path = dir.join(name);
    fs::write(&path, format!("#!/bin/sh\n{body}\n")).unwrap();
    fs::set_permissions(&path, fs::Permissions::from_mode(0o755)).unwrap();
}

/// Cases: one reviewer case with a planted bug on line 12, one clean case.
const CASES: &str = r#"[
 {"id":"bug","role":"reviewer","system":"You review code.","task":"Find bugs.","input":"12: x","score":{"kind":"findings","bug_lines":[12],"keywords":"off.by.one"}},
 {"id":"clean","role":"reviewer","system":"You review code.","task":"Find bugs.","input":"1: y","score":{"kind":"findings"}},
 {"id":"claims","role":"fact-checker","system":"Check.","task":"Check.","input":"1. a","score":{"kind":"claims","truth":{"1":true}}}
]"#;

fn envelope(result: &str, cost: f64) -> String {
    let result = serde_json::to_string(result).unwrap();
    format!(
        r#"{{"type":"result","result":{result},"total_cost_usd":{cost},"modelUsage":{{"claude-haiku-5-5":{{"costUSD":{cost}}}}}}}"#
    )
}

fn bench(args: &[&str], reply: &str, cost: f64, with_claude: bool) -> Run {
    let dir = scratch();
    if with_claude {
        fs::write(dir.join("reply.json"), envelope(reply, cost)).unwrap();
        let d = dir.display();
        stub(
            &dir,
            "claude",
            &format!("echo \"$@\" >> '{d}/argv'\ncat > /dev/null\ncat '{d}/reply.json'"),
        );
    }
    fs::write(dir.join("cases.json"), CASES).unwrap();
    let out = Command::new(env!("CARGO_BIN_EXE_playbook"))
        .args(["eval", "bench", "--cases"])
        .arg(dir.join("cases.json"))
        .args(args)
        .env("PATH", format!("{}:/usr/bin:/bin", dir.display()))
        .output()
        .unwrap();
    Run {
        code: out.status.code().unwrap_or(-1),
        out: String::from_utf8_lossy(&out.stdout).into_owned(),
        err: String::from_utf8_lossy(&out.stderr).into_owned(),
        dir,
    }
}

#[test]
fn list_prints_every_case_without_claude() {
    let r = bench(&["--list"], "", 0.0, false);
    assert_eq!(r.code, 0, "{}", r.err);
    assert!(r.out.contains("reviewer\tbug"));
    assert!(r.out.contains("fact-checker\tclaims"));
}

#[test]
fn a_missing_claude_cli_is_a_clear_error() {
    let r = bench(&[], "", 0.0, false);
    assert_eq!(r.code, 1);
    assert!(r.err.contains("claude CLI is required"), "{}", r.err);
}

#[test]
fn json_output_scores_cases_and_reports_cost() {
    let reply =
        r#"{"findings":[{"file":"a","line":12,"severity":"blocking","issue":"off by one"}]}"#;
    let r = bench(
        &["--role", "reviewer", "--json", "--runs", "2"],
        reply,
        0.01,
        true,
    );
    assert_eq!(r.code, 0, "{}", r.err);
    let v: serde_json::Value = serde_json::from_str(r.out.trim()).unwrap();
    let cells = v["cells"].as_array().unwrap();
    assert_eq!(cells.len(), 4, "2 cases x 2 runs");
    let bug: Vec<_> = cells.iter().filter(|c| c["id"] == "bug").collect();
    assert!(bug.iter().all(|c| c["ok"] == true));
    let clean: Vec<_> = cells.iter().filter(|c| c["id"] == "clean").collect();
    assert!(
        clean.iter().all(|c| c["ok"] == false),
        "a blocking finding on clean code is a false positive"
    );
    assert!((v["spent_usd"].as_f64().unwrap() - 0.04).abs() < 1e-9);
    let argv = fs::read_to_string(r.dir.join("argv")).unwrap();
    assert!(argv.contains("--model haiku") && argv.contains("--effort medium"));
}

#[test]
fn an_estimate_above_the_cap_refuses_before_any_call() {
    let r = bench(
        &[
            "--model",
            "opus",
            "--effort",
            "max",
            "--max-cost-usd",
            "0.0001",
        ],
        "{}",
        0.0,
        true,
    );
    assert_eq!(r.code, 1);
    assert!(r.err.contains("over --max-cost-usd"), "{}", r.err);
    assert!(!r.dir.join("argv").exists(), "no call may be made");
}

#[test]
fn claims_are_scored_per_claim() {
    let r = bench(
        &["--role", "fact-checker", "--json"],
        r#"{"claims":{"1":true}}"#,
        0.001,
        true,
    );
    assert_eq!(r.code, 0, "{}", r.err);
    let v: serde_json::Value = serde_json::from_str(r.out.trim()).unwrap();
    assert_eq!(v["cells"][0]["ok"], true);
}

#[test]
fn the_shipped_fixtures_load_and_cover_every_role() {
    let out = Command::new(env!("CARGO_BIN_EXE_playbook"))
        .args(["eval", "bench", "--list"])
        .output()
        .unwrap();
    let text = String::from_utf8_lossy(&out.stdout);
    for role in [
        "reviewer",
        "test-reviewer",
        "fact-checker",
        "critic",
        "analyst",
        "implementer",
    ] {
        assert!(
            text.contains(&format!("{role}\t")),
            "no {role} cases:\n{text}"
        );
    }
}

#[test]
fn the_lens_split_fixture_pairs_one_single_reviewer_with_five_lens_reviewers_per_diff() {
    let out = Command::new(env!("CARGO_BIN_EXE_playbook"))
        .args(["eval", "bench", "--list", "--cases"])
        .arg(Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/bench-extra"))
        .output()
        .unwrap();
    let text = String::from_utf8_lossy(&out.stdout);
    assert_eq!(text.matches("reviewer-single\t").count(), 10, "{text}");
    assert_eq!(text.matches("reviewer-swarm\t").count(), 50, "{text}");
    assert!(text.contains("reviewer-swarm\trd-par-cap@scope"));
}

/// One tool-enabled case: implement `f` so it returns 1. The stub `claude`
/// plays the implementer by reading its dispatch prompt.
const WU_CASE: &str = r#"[{"id":"wu-f","role":"implementer-wu","system":"x","task":"Implement f.",
 "score":{"kind":"tdd_repo","files":{"m.py":"def f():\n    raise NotImplementedError\n"},
 "test_file":"test_m.py","scenario":"f() returns 1","visible":"from m import f\nassert f() == 1\n",
 "hidden":{"hidden_m.py":"from m import f\nassert f() != 2\n"},"allowed":["m.py"],"verify":"python3 test_m.py"}}]"#;

const WU_STUB: &str = r#"p=$(cat)
echo "$p" >> "$STUB_LOG"
red() { printf 'from m import f\nassert f() == 1\n' > test_m.py; git add -A; git commit -q -m "wip(wu-1): red - s1"; }
green() { printf 'def f():\n    return 1\n' > m.py; git add -A; git commit -q -m "wip(wu-1): green - s1"; }
case "$p" in
  *"Your step: RED"*) red ;;
  *"Your step: GREEN"*) green ;;
  *"Your step: REFACTOR"*) : ;;
  *"whole Work Unit"*) red; green ;;
esac
echo '{"result":"DONE","total_cost_usd":0.05,"is_error":false}'"#;

fn wu_bench(args: &[&str]) -> Run {
    let dir = scratch();
    let d = dir.display();
    stub(
        &dir,
        "claude",
        &format!("export STUB_LOG='{d}/prompts'\n{WU_STUB}"),
    );
    fs::write(dir.join("cases.json"), WU_CASE).unwrap();
    let out = Command::new(env!("CARGO_BIN_EXE_playbook"))
        .args(["eval", "bench", "--cases"])
        .arg(dir.join("cases.json"))
        .args(args)
        .env(
            "PATH",
            format!("{}:/usr/bin:/bin:/usr/local/bin", dir.display()),
        )
        .output()
        .unwrap();
    Run {
        code: out.status.code().unwrap_or(-1),
        out: String::from_utf8_lossy(&out.stdout).into_owned(),
        err: String::from_utf8_lossy(&out.stderr).into_owned(),
        dir,
    }
}

fn have(tool: &str) -> bool {
    ["/usr/bin", "/bin", "/usr/local/bin"]
        .iter()
        .any(|d| Path::new(d).join(tool).is_file())
}

#[test]
fn per_step_makes_three_dispatches_and_per_wu_makes_one() {
    if !["git", "python3", "ssh-keygen"].iter().all(|t| have(t)) {
        return;
    }
    let r = wu_bench(&[
        "--strategy",
        "per-step,per-wu",
        "--json",
        "--max-cost-usd",
        "5",
    ]);
    assert_eq!(r.code, 0, "{}", r.err);
    let v: serde_json::Value = serde_json::from_str(r.out.trim()).unwrap();
    let runs = v["runs"].as_array().unwrap();
    assert_eq!(runs.len(), 2);
    for run in runs {
        assert_eq!(run["ok"], true, "{run}");
    }
    let by = |s: &str| runs.iter().find(|r| r["strategy"] == s).unwrap();
    assert_eq!(by("per-step")["dispatches"], 3);
    assert_eq!(by("per-wu")["dispatches"], 1);
    assert!((by("per-step")["cost"].as_f64().unwrap() - 0.15).abs() < 1e-9);
    let prompts = fs::read_to_string(r.dir.join("prompts")).unwrap();
    assert_eq!(prompts.matches("Your step:").count(), 3);
    assert_eq!(prompts.matches("whole Work Unit").count(), 1);
}

#[test]
fn a_run_that_skips_red_is_a_failure() {
    if !["git", "python3", "ssh-keygen"].iter().all(|t| have(t)) {
        return;
    }
    let dir = scratch();
    let d = dir.display();
    // The stub only ever commits GREEN.
    stub(
        &dir,
        "claude",
        &format!(
            "cat > /dev/null\nprintf 'def f():\\n    return 1\\n' > m.py\ngit add -A\ngit commit -q -m 'wip(wu-1): green - s1'\necho '{{\"result\":\"DONE\",\"total_cost_usd\":0.01}}'\n# {d}"
        ),
    );
    fs::write(dir.join("cases.json"), WU_CASE).unwrap();
    let out = Command::new(env!("CARGO_BIN_EXE_playbook"))
        .args(["eval", "bench", "--cases"])
        .arg(dir.join("cases.json"))
        .args(["--strategy", "per-wu", "--json"])
        .env(
            "PATH",
            format!("{}:/usr/bin:/bin:/usr/local/bin", dir.display()),
        )
        .output()
        .unwrap();
    let v: serde_json::Value = serde_json::from_slice(&out.stdout).unwrap();
    let run = &v["runs"][0];
    assert_eq!(run["ok"], false);
    assert!(
        run["detail"].as_str().unwrap().contains("no RED commit"),
        "{run}"
    );
}

#[test]
fn an_unknown_strategy_is_refused_and_the_single_turn_bench_skips_tdd_cases() {
    let r = wu_bench(&["--strategy", "per-everything"]);
    assert_eq!(r.code, 1);
    assert!(r.err.contains("unknown --strategy"), "{}", r.err);
    let plain = wu_bench(&["--model", "haiku"]);
    assert_eq!(plain.code, 1, "no single-turn case matches");
    assert!(plain.err.contains("no case matches"), "{}", plain.err);
}
