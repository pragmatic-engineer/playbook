// SPDX-FileCopyrightText: 2026 Igor Santos
// SPDX-License-Identifier: Apache-2.0

//! `playbook eval bench --strategy ...`: a tool-enabled implementer bench. It
//! compares how `/playbook:implement` can dispatch a Work Unit: one
//! `implementer` call per TDD step (`per-step`: RED, then GREEN, then
//! REFACTOR) against one call for the whole unit (`per-wu`). Each run gets a
//! scratch git repo and a real `claude` session with the plugin loaded, so
//! tool use, skills, verify runs and commits all happen.
//!
//! A run passes only when everything holds: the final code passes the original
//! visible test and the hidden tests, nothing outside the allowed files
//! changed, the RED commit holds only the test file and its test fails at that
//! commit, and RED comes before GREEN in the log.

use super::bench::{Case, Score};
use crate::common::par;
use serde_json::{json, Value};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::Mutex;
use std::time::{Duration, Instant};

/// The strategies this bench knows.
pub const STRATEGIES: [&str; 2] = ["per-step", "per-wu"];

/// Per-run conservative cost guess used to refuse an over-cap run up front.
fn estimate_per_run(strategy: &str) -> f64 {
    if strategy == "per-step" {
        0.25
    } else {
        0.12
    }
}

const STEPS: [&str; 3] = ["red", "green", "refactor"];

/// What one run produced.
#[derive(Debug, Clone)]
pub struct Run {
    pub id: String,
    pub strategy: String,
    pub model: String,
    pub effort: String,
    pub run: u32,
    pub outcome: Result<Graded, String>,
}

#[derive(Debug, Clone)]
pub struct Graded {
    pub ok: bool,
    pub detail: String,
    pub cost: f64,
    pub wall_s: f64,
    pub dispatches: u32,
}

/// The scratch tasks inside the fixture's `tdd_repo` score.
struct Task<'a> {
    files: &'a BTreeMap<String, String>,
    test_file: &'a str,
    scenario: &'a str,
    visible: &'a str,
    hidden: &'a BTreeMap<String, String>,
    allowed: &'a [String],
    verify: &'a str,
}

fn task_of(case: &Case) -> Option<Task<'_>> {
    match &case.score {
        Score::TddRepo {
            files,
            test_file,
            scenario,
            visible,
            hidden,
            allowed,
            verify,
        } => Some(Task {
            files,
            test_file,
            scenario,
            visible,
            hidden,
            allowed,
            verify,
        }),
        _ => None,
    }
}

fn git(dir: &Path, args: &[&str]) -> Result<String, String> {
    let out = Command::new("git")
        .args(args)
        .current_dir(dir)
        .env("GIT_CONFIG_GLOBAL", "/dev/null")
        .env("GIT_CONFIG_SYSTEM", "/dev/null")
        .output()
        .map_err(|e| format!("git: {e}"))?;
    if out.status.success() {
        Ok(String::from_utf8_lossy(&out.stdout).trim().to_string())
    } else {
        Err(format!(
            "git {}: {}",
            args.join(" "),
            String::from_utf8_lossy(&out.stderr).trim()
        ))
    }
}

fn safe_name(name: &str) -> Result<(), String> {
    let p = Path::new(name);
    if p.is_absolute() || p.components().any(|c| c.as_os_str() == "..") {
        return Err(format!("unsafe fixture path {name}"));
    }
    Ok(())
}

fn write_files(dir: &Path, files: &BTreeMap<String, String>) -> Result<(), String> {
    for (name, body) in files {
        safe_name(name)?;
        let p = dir.join(name);
        if let Some(parent) = p.parent() {
            std::fs::create_dir_all(parent).map_err(|e| e.to_string())?;
        }
        std::fs::write(&p, body).map_err(|e| e.to_string())?;
    }
    Ok(())
}

/// An ssh signing key, so the agent's signed commits work in the scratch repo.
fn make_key(dir: &Path) -> Result<PathBuf, String> {
    let key = dir.join("key");
    let st = Command::new("ssh-keygen")
        .args(["-q", "-t", "ed25519", "-N", ""])
        .arg("-f")
        .arg(&key)
        .status()
        .map_err(|e| format!("ssh-keygen: {e}"))?;
    if st.success() {
        Ok(key)
    } else {
        Err("ssh-keygen failed".into())
    }
}

fn brief_text(case: &Case, t: &Task, repo: &Path) -> String {
    let prod: Vec<&str> = t.allowed.iter().map(String::as_str).collect();
    format!(
        "# Work Unit wu-1: {}\n\nWorktree: {}\nFiles: {} (production), {} (tests, create it)\n\
         Changes: {}\nTest scenario s1:\n{}\n\
         Done when: `{}` exits 0 and the examples above hold.\nScoped verify: `{}`\n\
         Touch only the files named above. Never edit any other test file.\n",
        case.id,
        repo.display(),
        prod.join(", "),
        t.test_file,
        case.task,
        t.scenario,
        t.verify,
        t.verify
    )
}

fn dispatch_prompts(strategy: &str, brief: &Path, repo: &Path, dir: &Path) -> Vec<String> {
    let report = |step: &str| dir.join(format!("wu-1.s1-{step}.report.md"));
    let head = format!(
        "Brief file: {}. Worktree: {}.",
        brief.display(),
        repo.display()
    );
    if strategy == "per-wu" {
        return vec![format!(
            "{head} This dispatch owns the whole Work Unit: for each scenario run RED, then GREEN, then REFACTOR in that order, each as its own checkpoint commit with subject wip(wu-1): <step> - <scenario-id> (red, green, refactor). Write one report file per step at {}/wu-1.<scenario-id>-<step>.report.md. Return only a status, the last commit SHA and a one-line verify result.",
            dir.display()
        )];
    }
    STEPS
        .iter()
        .map(|step| {
            format!(
                "{head} Your step: {} for scenario s1. Report file: {}. Commit subject: wip(wu-1): {step} - s1.",
                step.to_uppercase(),
                report(step).display()
            )
        })
        .collect()
}

struct Call {
    cost: f64,
    wall: f64,
}

fn call_claude(
    prompt: &str,
    repo: &Path,
    home: &Path,
    plugin: &Path,
    model: &str,
    effort: &str,
) -> Result<Call, String> {
    let started = Instant::now();
    let mut child = Command::new("claude")
        .args(["-p", "--agent", "playbook:implementer", "--plugin-dir"])
        .arg(plugin)
        .args([
            "--model",
            model,
            "--effort",
            effort,
            "--dangerously-skip-permissions",
            "--setting-sources",
            "",
            "--max-turns",
            "40",
            "--max-budget-usd",
            "1",
            "--output-format",
            "json",
            "--no-session-persistence",
        ])
        .current_dir(repo)
        .env("HOME", home)
        .env("PLAYBOOK_HEADLESS", "1")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|e| format!("claude: {e}"))?;
    if let Some(mut stdin) = child.stdin.take() {
        use std::io::Write;
        let _ = stdin.write_all(prompt.as_bytes());
    }
    let out = child.wait_with_output().map_err(|e| e.to_string())?;
    let v: Value = serde_json::from_slice(&out.stdout).map_err(|_| {
        format!(
            "no json: {}",
            String::from_utf8_lossy(&out.stderr)
                .trim()
                .chars()
                .take(120)
                .collect::<String>()
        )
    })?;
    let cost = v["total_cost_usd"].as_f64().unwrap_or(0.0);
    if v["is_error"].as_bool() == Some(true) {
        return Err(format!(
            "claude error: {}",
            v["result"]
                .as_str()
                .unwrap_or("")
                .chars()
                .take(120)
                .collect::<String>()
        ));
    }
    Ok(Call {
        cost,
        wall: started.elapsed().as_secs_f64(),
    })
}

fn python(dir: &Path, file: &str) -> bool {
    let Ok(mut child) = Command::new("python3")
        .arg(file)
        .current_dir(dir)
        .env("PYTHONDONTWRITEBYTECODE", "1")
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
    else {
        return false;
    };
    let started = Instant::now();
    loop {
        match child.try_wait() {
            Ok(Some(s)) => return s.success(),
            Ok(None) if started.elapsed() > Duration::from_secs(20) => {
                let _ = child.kill();
                return false;
            }
            Ok(None) => std::thread::sleep(Duration::from_millis(20)),
            Err(_) => return false,
        }
    }
}

fn noise(path: &str) -> bool {
    path.contains("__pycache__") || path.ends_with(".pyc")
}

/// Grade a finished run, returning the failures (empty means a pass).
fn grade(t: &Task, repo: &Path, base: &str, scratch: &Path) -> Result<Vec<String>, String> {
    let mut bad = Vec::new();
    let log = git(
        repo,
        &[
            "log",
            "--reverse",
            "--format=%H\t%s",
            &format!("{base}..HEAD"),
        ],
    )?;
    let commits: Vec<(String, String)> = log
        .lines()
        .filter_map(|l| l.split_once('\t'))
        .map(|(h, s)| (h.to_string(), s.to_lowercase()))
        .collect();
    let find = |word: &str| commits.iter().position(|(_, s)| s.contains(word));
    match (find("red"), find("green")) {
        (Some(r), Some(g)) if r < g => {
            let sha = &commits[r].0;
            let files = git(repo, &["show", "--name-only", "--format=", sha])?;
            let touched: Vec<&str> = files.lines().filter(|f| !noise(f)).collect();
            if touched != [t.test_file] {
                bad.push(format!("RED touched {touched:?}"));
            }
            let red_dir = scratch.join("red");
            std::fs::create_dir_all(&red_dir).map_err(|e| e.to_string())?;
            let tar = Command::new("sh")
                .arg("-c")
                .arg(format!(
                    "git -C '{}' archive {sha} | tar -x -C '{}'",
                    repo.display(),
                    red_dir.display()
                ))
                .env("GIT_CONFIG_GLOBAL", "/dev/null")
                .status()
                .map_err(|e| e.to_string())?;
            if !tar.success() {
                return Err("could not export the RED commit".into());
            }
            if python(&red_dir, t.test_file) {
                bad.push("RED test passed at the RED commit".into());
            }
        }
        (Some(_), Some(_)) => bad.push("GREEN before RED".into()),
        (None, _) => bad.push("no RED commit".into()),
        (_, None) => bad.push("no GREEN commit".into()),
    }
    let changed = git(repo, &["diff", "--name-only", base, "HEAD"])?;
    for f in changed.lines().filter(|f| !noise(f)) {
        if f != t.test_file && !t.allowed.iter().any(|a| a == f) {
            bad.push(format!("edited {f}"));
        }
    }
    if !python(repo, t.test_file) {
        bad.push("own test fails on final code".into());
    }
    let mut check = BTreeMap::new();
    check.insert(t.test_file.to_string(), t.visible.to_string());
    for (k, v) in t.hidden {
        check.insert(k.clone(), v.clone());
    }
    write_files(repo, &check)?;
    for name in check.keys() {
        if !python(repo, name) {
            bad.push(format!("{name} fails"));
        }
    }
    Ok(bad)
}

fn run_one(
    case: &Case,
    t: &Task,
    strategy: &str,
    model: &str,
    effort: &str,
    plugin: &Path,
    key: &Path,
) -> Result<Graded, String> {
    // A counter, not the clock: parallel runs can read the same nanosecond.
    static NEXT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
    let scratch = std::env::temp_dir().join(format!(
        "playbook-wubench-{}-{}",
        std::process::id(),
        NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
    ));
    let (repo, home, brief_dir) = (
        scratch.join("repo"),
        scratch.join("home"),
        scratch.join("brief"),
    );
    for d in [&repo, &home, &brief_dir] {
        std::fs::create_dir_all(d).map_err(|e| e.to_string())?;
    }
    let result = (|| {
        git(&repo, &["init", "-q", "-b", "main"])?;
        let key_s = key.to_string_lossy().to_string();
        for kv in [
            ("user.name", "Bench"),
            ("user.email", "bench@example.com"),
            ("gpg.format", "ssh"),
            ("user.signingkey", key_s.as_str()),
            ("commit.gpgsign", "false"),
        ] {
            git(&repo, &["config", kv.0, kv.1])?;
        }
        write_files(&repo, t.files)?;
        git(&repo, &["add", "-A"])?;
        git(&repo, &["commit", "-q", "-m", "base"])?;
        let base = git(&repo, &["rev-parse", "HEAD"])?;
        let brief = brief_dir.join("wu-1.brief.md");
        std::fs::write(&brief, brief_text(case, t, &repo)).map_err(|e| e.to_string())?;
        let prompts = dispatch_prompts(strategy, &brief, &repo, &brief_dir);
        let (mut cost, mut wall) = (0.0, 0.0);
        for p in &prompts {
            let c = call_claude(p, &repo, &home, plugin, model, effort)?;
            cost += c.cost;
            wall += c.wall;
        }
        let bad = grade(t, &repo, &base, &scratch)?;
        Ok(Graded {
            ok: bad.is_empty(),
            detail: if bad.is_empty() {
                "ok".into()
            } else {
                bad.join("; ")
            },
            cost,
            wall_s: wall,
            dispatches: prompts.len() as u32,
        })
    })();
    let _ = std::fs::remove_dir_all(&scratch);
    result
}

/// What to run.
pub struct Plan<'a> {
    pub cases: Vec<&'a Case>,
    pub strategies: Vec<String>,
    pub models: Vec<String>,
    pub efforts: Vec<String>,
    pub runs: u32,
    pub max_cost_usd: f64,
    pub jobs: usize,
    pub json: bool,
    pub plugin_root: PathBuf,
}

/// Run every (case, strategy, model, effort, run) and print the comparison.
/// Returns the process exit code.
pub fn run(plan: &Plan) -> i32 {
    for s in &plan.strategies {
        if !STRATEGIES.contains(&s.as_str()) {
            eprintln!("eval bench: unknown --strategy '{s}' (use per-step or per-wu)");
            return 1;
        }
    }
    for tool in ["claude", "python3", "git", "ssh-keygen"] {
        let found = std::env::var_os("PATH")
            .map(|p| std::env::split_paths(&p).any(|d| d.join(tool).is_file()))
            .unwrap_or(false);
        if !found {
            eprintln!("eval bench: {tool} is required for a tool-enabled run");
            return 1;
        }
    }
    let mut jobs = Vec::new();
    for c in &plan.cases {
        for s in &plan.strategies {
            for m in &plan.models {
                for e in &plan.efforts {
                    for r in 1..=plan.runs {
                        jobs.push((*c, s.clone(), m.clone(), e.clone(), r));
                    }
                }
            }
        }
    }
    let estimate: f64 = jobs.iter().map(|j| estimate_per_run(&j.1)).sum();
    eprintln!(
        "bench: {} run(s), estimated ${:.2}, cap ${:.2}",
        jobs.len(),
        estimate,
        plan.max_cost_usd
    );
    if estimate > plan.max_cost_usd {
        eprintln!(
            "eval bench: estimated cost ${estimate:.2} is over --max-cost-usd ${:.2}; narrow --id, lower --runs, or raise the cap",
            plan.max_cost_usd
        );
        return 1;
    }
    let keydir = std::env::temp_dir().join(format!("playbook-wubench-key-{}", std::process::id()));
    let key = match std::fs::create_dir_all(&keydir)
        .map_err(|e| e.to_string())
        .and_then(|()| make_key(&keydir))
    {
        Ok(k) => k,
        Err(e) => {
            eprintln!("eval bench: {e}");
            return 1;
        }
    };
    let spent = Mutex::new(0.0f64);
    let runs: Vec<Run> = par::map(&jobs, plan.jobs.max(1), |(c, s, m, e, r)| {
        let mk = |outcome| Run {
            id: c.id.clone(),
            strategy: s.clone(),
            model: m.clone(),
            effort: e.clone(),
            run: *r,
            outcome,
        };
        if *spent.lock().unwrap_or_else(|x| x.into_inner()) >= plan.max_cost_usd {
            return mk(Err("skipped: spend cap reached".into()));
        }
        let Some(t) = task_of(c) else {
            return mk(Err("not a tdd_repo case".into()));
        };
        let out = run_one(c, &t, s, m, e, &plan.plugin_root, &key);
        if let Ok(g) = &out {
            *spent.lock().unwrap_or_else(|x| x.into_inner()) += g.cost;
        }
        mk(out)
    });
    let _ = std::fs::remove_dir_all(&keydir);
    let total = *spent.lock().unwrap_or_else(|x| x.into_inner());
    if plan.json {
        println!("{}", render_json(&runs, total, plan.max_cost_usd));
    } else {
        print!("{}", render_table(&runs, total));
    }
    i32::from(!runs.iter().any(|r| r.outcome.is_ok()))
}

type Key = (String, String, String);

#[derive(Default)]
struct Agg {
    n: usize,
    pass: usize,
    errored: usize,
    cost: f64,
    wall: f64,
    dispatches: u32,
}

fn aggregate(runs: &[Run]) -> BTreeMap<Key, Agg> {
    let mut map: BTreeMap<Key, Agg> = BTreeMap::new();
    for r in runs {
        let a = map
            .entry((r.strategy.clone(), r.model.clone(), r.effort.clone()))
            .or_default();
        match &r.outcome {
            Ok(g) => {
                a.n += 1;
                a.pass += usize::from(g.ok);
                a.cost += g.cost;
                a.wall += g.wall_s;
                a.dispatches += g.dispatches;
            }
            Err(_) => a.errored += 1,
        }
    }
    map
}

fn render_table(runs: &[Run], spent: f64) -> String {
    let mut out = String::from(
        "strategy\tmodel\teffort\tpass\tcost/run\twall(s)/run\tdispatches/run\terrored\n",
    );
    for ((s, m, e), a) in aggregate(runs) {
        let n = a.n.max(1) as f64;
        out.push_str(&format!(
            "{s}\t{m}\t{e}\t{}/{} ({:.0}%)\t${:.4}\t{:.1}\t{:.1}\t{}\n",
            a.pass,
            a.n,
            100.0 * a.pass as f64 / n,
            a.cost / n,
            a.wall / n,
            f64::from(a.dispatches) / n,
            a.errored
        ));
    }
    for r in runs {
        match &r.outcome {
            Ok(g) if !g.ok => out.push_str(&format!(
                "fail\t{}\t{}\trun {}\t{}\n",
                r.id, r.strategy, r.run, g.detail
            )),
            Err(e) => out.push_str(&format!(
                "error\t{}\t{}\trun {}\t{e}\n",
                r.id, r.strategy, r.run
            )),
            _ => {}
        }
    }
    out.push_str(&format!("total spend ${spent:.4}\n"));
    out
}

fn render_json(runs: &[Run], spent: f64, cap: f64) -> String {
    let rows: Vec<Value> = runs
        .iter()
        .map(|r| match &r.outcome {
            Ok(g) => json!({"id": r.id, "strategy": r.strategy, "model": r.model,
                "effort": r.effort, "run": r.run, "ok": g.ok, "detail": g.detail,
                "cost": g.cost, "wall_s": g.wall_s, "dispatches": g.dispatches}),
            Err(e) => json!({"id": r.id, "strategy": r.strategy, "model": r.model,
                "effort": r.effort, "run": r.run, "error": e}),
        })
        .collect();
    let summary: Vec<Value> = aggregate(runs)
        .into_iter()
        .map(|((s, m, e), a)| {
            let n = a.n.max(1) as f64;
            json!({"strategy": s, "model": m, "effort": e, "n": a.n, "pass": a.pass,
                "pass_rate": a.pass as f64 / n, "cost_per_run": a.cost / n,
                "wall_s_per_run": a.wall / n, "errored": a.errored})
        })
        .collect();
    json!({"spent_usd": spent, "cap_usd": cap, "summary": summary, "runs": rows}).to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn task_files() -> BTreeMap<String, String> {
        BTreeMap::from([(
            "m.py".to_string(),
            "def f():\n    raise NotImplementedError\n".to_string(),
        )])
    }

    #[test]
    fn per_step_dispatches_three_times_and_per_wu_once() {
        let (b, r, d) = (Path::new("/b.md"), Path::new("/r"), Path::new("/d"));
        let steps = dispatch_prompts("per-step", b, r, d);
        assert_eq!(steps.len(), 3);
        assert!(
            steps[0].contains("RED") && steps[1].contains("GREEN") && steps[2].contains("REFACTOR")
        );
        let wu = dispatch_prompts("per-wu", b, r, d);
        assert_eq!(wu.len(), 1);
        assert!(wu[0].contains("whole Work Unit"));
    }

    #[test]
    fn fixture_paths_that_escape_are_refused() {
        let d = std::env::temp_dir();
        let bad = BTreeMap::from([("../x.py".to_string(), String::new())]);
        assert!(write_files(&d, &bad).is_err());
        let abs = BTreeMap::from([("/x.py".to_string(), String::new())]);
        assert!(write_files(&d, &abs).is_err());
    }

    #[test]
    fn the_table_compares_strategies() {
        let g = |ok, cost| Run {
            id: "a".into(),
            strategy: "per-wu".into(),
            model: "sonnet".into(),
            effort: "medium".into(),
            run: 1,
            outcome: Ok(Graded {
                ok,
                detail: "d".into(),
                cost,
                wall_s: 10.0,
                dispatches: 1,
            }),
        };
        let t = render_table(&[g(true, 0.1), g(false, 0.3)], 0.4);
        assert!(
            t.contains("per-wu\tsonnet\tmedium\t1/2 (50%)\t$0.2000\t10.0\t1.0\t0"),
            "{t}"
        );
        assert!(t.contains("fail\ta\tper-wu\trun 1\td"));
    }

    #[test]
    fn grading_needs_red_before_green_with_only_the_test_in_red() {
        let need = ["git", "python3"];
        let on_path = |t: &str| {
            std::env::var_os("PATH")
                .map(|p| std::env::split_paths(&p).any(|d| d.join(t).is_file()))
                .unwrap_or(false)
        };
        if !need.iter().all(|t| on_path(t)) {
            return;
        }
        let dir = std::env::temp_dir().join(format!("pb-wubench-grade-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let repo = dir.join("repo");
        std::fs::create_dir_all(&repo).unwrap();
        let setup: [&[&str]; 3] = [
            &["init", "-q", "-b", "main"],
            &["config", "user.name", "t"],
            &["config", "user.email", "t@e.x"],
        ];
        for args in setup {
            git(&repo, args).unwrap();
        }
        write_files(&repo, &task_files()).unwrap();
        git(&repo, &["add", "-A"]).unwrap();
        git(&repo, &["commit", "-q", "-m", "base"]).unwrap();
        let base = git(&repo, &["rev-parse", "HEAD"]).unwrap();
        let visible = "from m import f\nassert f() == 1\n".to_string();
        let hidden = BTreeMap::new();
        let allowed = vec!["m.py".to_string()];
        let files = task_files();
        let t = Task {
            files: &files,
            test_file: "test_m.py",
            scenario: "s",
            visible: &visible,
            hidden: &hidden,
            allowed: &allowed,
            verify: "python3 test_m.py",
        };
        // RED: only the test, failing.
        write_files(
            &repo,
            &BTreeMap::from([("test_m.py".to_string(), visible.clone())]),
        )
        .unwrap();
        git(&repo, &["add", "-A"]).unwrap();
        git(&repo, &["commit", "-q", "-m", "wip(wu-1): red - s1"]).unwrap();
        // GREEN
        write_files(
            &repo,
            &BTreeMap::from([("m.py".to_string(), "def f():\n    return 1\n".to_string())]),
        )
        .unwrap();
        git(&repo, &["add", "-A"]).unwrap();
        git(&repo, &["commit", "-q", "-m", "wip(wu-1): green - s1"]).unwrap();
        assert_eq!(grade(&t, &repo, &base, &dir).unwrap(), Vec::<String>::new());

        // A hardcoded answer that the hidden test catches fails.
        let hidden2 = BTreeMap::from([(
            "hidden_m.py".to_string(),
            "from m import f\nassert f() == 2\n".to_string(),
        )]);
        let t2 = Task {
            hidden: &hidden2,
            ..t
        };
        let bad = grade(&t2, &repo, &base, &dir).unwrap();
        assert_eq!(bad, vec!["hidden_m.py fails".to_string()]);
        let _ = std::fs::remove_dir_all(&dir);
    }
}
