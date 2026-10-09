// SPDX-FileCopyrightText: 2026 Igor Santos
// SPDX-License-Identifier: Apache-2.0

//! `playbook eval bench`: a live benchmark of agent roles across models and
//! effort levels. Each case is a fixed prompt with an objective check (a
//! planted bug found, a claim judged right, a test that passes), so there is
//! no judge model. Costs real API calls, so it estimates first and refuses to
//! start when the estimate is over `--max-cost-usd`, and it stops mid run
//! when the spend reaches the cap.

use crate::common::par;
use crate::models;
use crate::usage::pricing::{self, Tokens};
use regex::Regex;
use serde::Deserialize;
use serde_json::{json, Value};
use std::collections::BTreeMap;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::Mutex;
use std::time::{Duration, Instant};

pub const DEFAULT_CASES: &str = "tests/fixtures/bench";

/// Appended to a prompt so a model without tools answers instead of asking to read files.
const NO_TOOLS_NOTE: &str = "\n\nIMPORTANT: everything you need is included above. You have no tools in this run (do not call Read, Grep, Glob or Skill). Answer directly: your final message must be only what was asked for.";

/// How a reply is checked.
#[derive(Debug, Clone, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Score {
    /// A JSON list of findings. With planted bugs, one finding must sit within
    /// `tolerance` lines of a bug line or match `keywords`. With none planted, no
    /// finding may be `blocking`.
    Findings {
        #[serde(default)]
        bug_lines: Vec<i64>,
        #[serde(default)]
        keywords: Option<String>,
        #[serde(default = "one")]
        tolerance: i64,
    },
    /// A JSON object with a `verdict` equal to `want`.
    Verdict { want: String },
    /// A JSON object `{"claims": {"1": true, ...}}` that matches `truth` exactly.
    Claims { truth: BTreeMap<String, bool> },
    /// A JSON object `{"facts": [...]}` of memory facts: well formed, with every
    /// anchor drawn from `allowed_anchors` and every `required_anchors` covered.
    Facts {
        allowed_anchors: Vec<String>,
        required_anchors: Vec<String>,
    },
    /// Code that must pass a test: `files` are written to a temp dir, the
    /// reply's code block becomes `solution`, and `cmd` must exit 0.
    RunTest {
        files: BTreeMap<String, String>,
        solution: String,
        cmd: Vec<String>,
    },
}

fn one() -> i64 {
    1
}

fn yes() -> bool {
    true
}

/// One benchmark case.
#[derive(Debug, Clone, Deserialize)]
pub struct Case {
    pub id: String,
    pub role: String,
    /// Agent file (relative to the repo root) whose body is the system prompt.
    #[serde(default)]
    pub agent: Option<String>,
    /// Inline system prompt, used when there is no `agent`.
    #[serde(default)]
    pub system: Option<String>,
    pub task: String,
    #[serde(default)]
    pub input: Option<String>,
    /// Input read from a file next to the case file.
    #[serde(default)]
    pub input_file: Option<String>,
    /// Append the no-tools note (default true).
    #[serde(default = "yes")]
    pub note: bool,
    pub score: Score,
}

/// What to run.
#[derive(Debug, Clone)]
pub struct Options {
    pub cases: PathBuf,
    pub roles: Vec<String>,
    pub ids: Vec<String>,
    pub models: Vec<String>,
    pub efforts: Vec<String>,
    pub runs: u32,
    pub max_cost_usd: f64,
    pub jobs: usize,
    pub json: bool,
    pub list: bool,
    pub repo_root: PathBuf,
}

/// A case with its prompts resolved.
#[derive(Debug, Clone)]
pub struct Loaded {
    pub case: Case,
    pub system: String,
    pub user: String,
}

/// One scored call.
#[derive(Debug, Clone)]
pub struct Cell {
    pub id: String,
    pub role: String,
    pub model: String,
    pub effort: String,
    pub run: u32,
    pub outcome: Outcome,
}

#[derive(Debug, Clone)]
pub enum Outcome {
    Scored {
        ok: bool,
        detail: String,
        cost: f64,
        wall_s: f64,
        resolved: Vec<String>,
    },
    Errored(String),
    Skipped,
}

// ---------- loading ----------

/// The prompt body: everything after the second `---` line of an agent file.
pub fn agent_body(text: &str) -> String {
    let mut seen = 0;
    let mut out = Vec::new();
    for line in text.lines() {
        if line.trim_end() == "---" {
            seen += 1;
            continue;
        }
        if seen >= 2 {
            out.push(line);
        }
    }
    out.join("\n").trim().to_string()
}

/// Every case under `path`: one `.json` file or a directory of them. A file
/// holds a list of cases or `{"cases": [...]}`.
pub fn load_cases(path: &Path, repo_root: &Path) -> Result<Vec<Loaded>, String> {
    let mut files = Vec::new();
    if path.is_dir() {
        let entries = std::fs::read_dir(path).map_err(|e| format!("{}: {e}", path.display()))?;
        for entry in entries.flatten() {
            let p = entry.path();
            if p.extension().is_some_and(|e| e == "json") {
                files.push(p);
            }
        }
        files.sort();
    } else {
        files.push(path.to_path_buf());
    }
    if files.is_empty() {
        return Err(format!("no case files in {}", path.display()));
    }
    let mut out = Vec::new();
    for file in files {
        let text =
            std::fs::read_to_string(&file).map_err(|e| format!("{}: {e}", file.display()))?;
        let value: Value =
            serde_json::from_str(&text).map_err(|e| format!("{}: {e}", file.display()))?;
        let list = match value {
            Value::Array(a) => a,
            Value::Object(mut o) => match o.remove("cases") {
                Some(Value::Array(a)) => a,
                _ => return Err(format!("{}: expected a list of cases", file.display())),
            },
            _ => return Err(format!("{}: expected a list of cases", file.display())),
        };
        let dir = file.parent().unwrap_or(Path::new("."));
        for item in list {
            let case: Case = serde_json::from_value(item)
                .map_err(|e| format!("{}: bad case: {e}", file.display()))?;
            out.push(resolve(case, dir, repo_root)?);
        }
    }
    let mut seen = std::collections::BTreeSet::new();
    for l in &out {
        if !seen.insert(l.case.id.clone()) {
            return Err(format!("duplicate case id {}", l.case.id));
        }
    }
    Ok(out)
}

fn resolve(case: Case, dir: &Path, repo_root: &Path) -> Result<Loaded, String> {
    let system = match (&case.agent, &case.system) {
        (Some(a), _) => {
            let p = repo_root.join(a);
            let text = std::fs::read_to_string(&p)
                .map_err(|e| format!("case {}: agent {}: {e}", case.id, p.display()))?;
            agent_body(&text)
        }
        (None, Some(s)) => s.clone(),
        (None, None) => {
            return Err(format!(
                "case {}: needs an agent or a system prompt",
                case.id
            ))
        }
    };
    let input = match (&case.input, &case.input_file) {
        (Some(i), _) => i.clone(),
        (None, Some(f)) => std::fs::read_to_string(dir.join(f))
            .map_err(|e| format!("case {}: input_file {f}: {e}", case.id))?,
        (None, None) => String::new(),
    };
    let mut user = if input.is_empty() {
        case.task.clone()
    } else {
        format!("{}\n\n{}", case.task, input)
    };
    if case.note {
        user.push_str(NO_TOOLS_NOTE);
    }
    Ok(Loaded { case, system, user })
}

// ---------- scoring ----------

/// Parses a reply as JSON, retrying on the outermost `[...]` or `{...}` span
/// because models wrap the answer in fences or prose.
pub fn parse_json(text: &str) -> Option<Value> {
    let t = text.trim();
    if let Ok(v) = serde_json::from_str::<Value>(t) {
        return Some(v);
    }
    let brace = t.find('{');
    let bracket = t.find('[');
    let spans: [(char, char, Option<usize>); 2] = [('[', ']', bracket), ('{', '}', brace)];
    let mut order: Vec<(char, char, usize)> = spans
        .iter()
        .filter_map(|(o, c, p)| p.map(|p| (*o, *c, p)))
        .collect();
    order.sort_by_key(|(_, _, p)| *p);
    for (_, close, start) in order {
        if let Some(end) = t.rfind(close) {
            if end > start {
                if let Ok(v) = serde_json::from_str::<Value>(&t[start..=end]) {
                    return Some(v);
                }
            }
        }
    }
    None
}

/// The first fenced code block of a reply, else the reply itself.
pub fn code_block(text: &str) -> String {
    if let Some(start) = text.find("```") {
        let rest = &text[start + 3..];
        let after_lang = rest.find('\n').map(|i| &rest[i + 1..]).unwrap_or(rest);
        if let Some(end) = after_lang.find("```") {
            return after_lang[..end].to_string();
        }
        return after_lang.to_string();
    }
    text.to_string()
}

fn findings_of(v: &Value) -> Option<Vec<Value>> {
    match v {
        Value::Array(a) => Some(a.clone()),
        Value::Object(o) => o.get("findings").and_then(Value::as_array).cloned(),
        _ => None,
    }
}

/// Checks `reply` against `score`. `Ok((passed, detail))`, or `Err` when the
/// check itself could not run (for example no `python3`).
pub fn judge(score: &Score, reply: &str) -> Result<(bool, String), String> {
    match score {
        Score::Findings {
            bug_lines,
            keywords,
            tolerance,
        } => {
            let Some(list) = parse_json(reply).as_ref().and_then(findings_of) else {
                return Ok((false, "unparseable".into()));
            };
            if bug_lines.is_empty() && keywords.is_none() {
                let blocking = list
                    .iter()
                    .filter(|f| {
                        f.get("severity")
                            .and_then(Value::as_str)
                            .is_some_and(|s| s.eq_ignore_ascii_case("blocking"))
                    })
                    .count();
                return Ok(if blocking == 0 {
                    (true, "clean".into())
                } else {
                    (false, format!("FALSE-POSITIVE blocking={blocking}"))
                });
            }
            let re = keywords
                .as_deref()
                .map(|k| Regex::new(&format!("(?i){k}")))
                .transpose()
                .map_err(|e| format!("bad keywords regex: {e}"))?;
            for f in &list {
                let line = f
                    .get("line")
                    .or_else(|| f.get("step"))
                    .and_then(|l| {
                        l.as_i64()
                            .or_else(|| l.as_str().and_then(|s| s.parse().ok()))
                    })
                    .unwrap_or(i64::MIN);
                let issue = f.get("issue").map(|i| i.to_string()).unwrap_or_default();
                let near = bug_lines
                    .iter()
                    .any(|b| line != i64::MIN && (line - b).abs() <= *tolerance);
                let said = re.as_ref().is_some_and(|r| r.is_match(&issue));
                if near || said {
                    return Ok((true, format!("hit line={line}")));
                }
            }
            Ok((false, format!("MISSED n={}", list.len())))
        }
        Score::Verdict { want } => {
            let Some(v) = parse_json(reply) else {
                return Ok((false, "unparseable".into()));
            };
            let got = v
                .get("verdict")
                .and_then(Value::as_str)
                .unwrap_or("")
                .to_uppercase();
            Ok((got == want.to_uppercase(), format!("got {got} want {want}")))
        }
        Score::Claims { truth } => {
            let Some(v) = parse_json(reply) else {
                return Ok((false, "unparseable".into()));
            };
            let Some(claims) = v.get("claims").and_then(Value::as_object) else {
                return Ok((false, "unparseable".into()));
            };
            let wrong: Vec<&str> = truth
                .iter()
                .filter(|(k, want)| claims.get(*k).and_then(Value::as_bool) != Some(**want))
                .map(|(k, _)| k.as_str())
                .collect();
            Ok((wrong.is_empty(), format!("wrong={}", wrong.join(","))))
        }
        Score::Facts {
            allowed_anchors,
            required_anchors,
        } => {
            let Some(v) = parse_json(reply) else {
                return Ok((false, "unparseable".into()));
            };
            let Some(facts) = v.get("facts").and_then(Value::as_array) else {
                return Ok((false, "unparseable".into()));
            };
            if facts.is_empty() {
                return Ok((false, "no facts".into()));
            }
            let mut seen: Vec<String> = Vec::new();
            for f in facts {
                let title = f.get("title").and_then(Value::as_str).unwrap_or("");
                let kind = f.get("type").and_then(Value::as_str).unwrap_or("");
                let scope = f.get("scope").and_then(Value::as_str).unwrap_or("");
                if title.trim().is_empty() || !["project", "reference"].contains(&kind) {
                    return Ok((false, "bad fact shape".into()));
                }
                if !["repo", "global"].contains(&scope) {
                    return Ok((false, "bad scope".into()));
                }
                for a in f
                    .get("anchors")
                    .and_then(Value::as_array)
                    .into_iter()
                    .flatten()
                    .filter_map(Value::as_str)
                {
                    if !allowed_anchors.iter().any(|x| x == a) {
                        return Ok((false, format!("INVENTED anchor {a}")));
                    }
                    seen.push(a.to_string());
                }
            }
            let missing: Vec<&String> = required_anchors
                .iter()
                .filter(|r| !seen.contains(r))
                .collect();
            Ok(if missing.is_empty() {
                (true, "ok".into())
            } else {
                (
                    false,
                    format!(
                        "MISSING anchors {}",
                        missing
                            .iter()
                            .map(|s| s.as_str())
                            .collect::<Vec<_>>()
                            .join(",")
                    ),
                )
            })
        }
        Score::RunTest {
            files,
            solution,
            cmd,
        } => run_test(files, solution, cmd, &code_block(reply)),
    }
}

fn run_test(
    files: &BTreeMap<String, String>,
    solution: &str,
    cmd: &[String],
    code: &str,
) -> Result<(bool, String), String> {
    let Some((prog, args)) = cmd.split_first() else {
        return Err("run_test needs a cmd".into());
    };
    if !tool_on_path(prog) {
        return Err(format!("{prog} is needed to score this case"));
    }
    let dir = std::env::temp_dir().join(format!(
        "playbook-bench-{}-{}",
        std::process::id(),
        crate::common::time::now_nanos()
    ));
    std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    let result = (|| {
        for (name, body) in files {
            let rel = Path::new(name);
            if rel.is_absolute() || rel.components().any(|c| c.as_os_str() == "..") {
                return Err(format!("unsafe fixture path {name}"));
            }
            std::fs::write(dir.join(rel), body).map_err(|e| e.to_string())?;
        }
        std::fs::write(dir.join(solution), code).map_err(|e| e.to_string())?;
        let mut child = Command::new(prog)
            .args(args)
            .current_dir(&dir)
            .stdout(Stdio::null())
            .stderr(Stdio::piped())
            .spawn()
            .map_err(|e| e.to_string())?;
        let started = Instant::now();
        loop {
            match child.try_wait() {
                Ok(Some(status)) => {
                    let mut err = String::new();
                    if let Some(mut s) = child.stderr.take() {
                        use std::io::Read;
                        let _ = s.read_to_string(&mut err);
                    }
                    let tail: String = err.lines().last().unwrap_or("").chars().take(120).collect();
                    return Ok(if status.success() {
                        (true, "tests pass".to_string())
                    } else {
                        (false, format!("tests fail: {tail}"))
                    });
                }
                Ok(None) if started.elapsed() > Duration::from_secs(20) => {
                    let _ = child.kill();
                    return Ok((false, "test timed out".to_string()));
                }
                Ok(None) => std::thread::sleep(Duration::from_millis(20)),
                Err(e) => return Err(e.to_string()),
            }
        }
    })();
    let _ = std::fs::remove_dir_all(&dir);
    result
}

fn tool_on_path(tool: &str) -> bool {
    std::env::var_os("PATH")
        .map(|p| std::env::split_paths(&p).any(|d| d.join(tool).is_file()))
        .unwrap_or(false)
}

// ---------- cost ----------

fn full_model_id(alias: &str) -> String {
    models::TIERS
        .iter()
        .find(|t| t.alias == alias)
        .map(|t| t.preferred.to_string())
        .unwrap_or_else(|| alias.to_string())
}

/// Output tokens a call spends, by effort, from measured runs: thinking grows
/// fast with the level.
fn expected_output_tokens(effort: &str) -> u64 {
    match effort {
        "low" => 800,
        "medium" => 1500,
        "high" => 2500,
        "xhigh" => 4000,
        _ => 8000,
    }
}

/// A conservative cost estimate for one call. Input tokens are about four
/// characters each, plus a fixed system overhead. `None` for a model with no
/// price in the table.
pub fn estimate_call(model: &str, effort: &str, prompt_chars: usize) -> Option<f64> {
    let tokens = Tokens {
        input: (prompt_chars as u64) / 4 + 3000,
        output: expected_output_tokens(effort),
        ..Tokens::default()
    };
    pricing::cost_usd(&full_model_id(model), &tokens)
}

// ---------- running ----------

fn claude_args(model: &str, effort: &str, system: &str) -> Vec<String> {
    [
        "-p",
        "--model",
        model,
        "--effort",
        effort,
        "--output-format",
        "json",
        "--tools",
        "",
        "--system-prompt",
        system,
        "--setting-sources",
        "",
        "--max-turns",
        "1",
        "--no-session-persistence",
    ]
    .iter()
    .map(|s| s.to_string())
    .collect()
}

fn call_claude(
    case: &Loaded,
    model: &str,
    effort: &str,
) -> Result<(String, f64, Vec<String>, f64), String> {
    let started = Instant::now();
    let mut last = String::new();
    for _ in 0..2 {
        let mut child = Command::new("claude")
            .args(claude_args(model, effort, &case.system))
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .map_err(|e| format!("claude: {e}"))?;
        if let Some(mut stdin) = child.stdin.take() {
            let _ = stdin.write_all(case.user.as_bytes());
        }
        let out = child.wait_with_output().map_err(|e| e.to_string())?;
        let text = String::from_utf8_lossy(&out.stdout).to_string();
        match serde_json::from_str::<Value>(&text) {
            Ok(v) => {
                let result = v
                    .get("result")
                    .and_then(Value::as_str)
                    .unwrap_or("")
                    .to_string();
                let cost = v
                    .get("total_cost_usd")
                    .and_then(Value::as_f64)
                    .unwrap_or(0.0);
                let resolved = v
                    .get("modelUsage")
                    .and_then(Value::as_object)
                    .map(|m| m.keys().cloned().collect())
                    .unwrap_or_default();
                if v.get("is_error").and_then(Value::as_bool) == Some(true) {
                    last = format!(
                        "claude error: {}",
                        result.chars().take(120).collect::<String>()
                    );
                    continue;
                }
                return Ok((result, cost, resolved, started.elapsed().as_secs_f64()));
            }
            Err(_) => {
                let err = String::from_utf8_lossy(&out.stderr);
                last = format!(
                    "no json: {}",
                    err.trim().chars().take(120).collect::<String>()
                );
            }
        }
    }
    Err(last)
}

/// Every (case, model, effort, run) to execute, in a stable order.
pub fn plan<'a>(cases: &'a [Loaded], opts: &Options) -> Vec<(&'a Loaded, String, String, u32)> {
    let mut jobs = Vec::new();
    for c in cases {
        if !opts.roles.is_empty() && !opts.roles.contains(&c.case.role) {
            continue;
        }
        if !opts.ids.is_empty() && !opts.ids.iter().any(|i| c.case.id.starts_with(i.as_str())) {
            continue;
        }
        for m in &opts.models {
            for e in &opts.efforts {
                for r in 1..=opts.runs {
                    jobs.push((c, m.clone(), e.clone(), r));
                }
            }
        }
    }
    jobs
}

/// The estimated cost of `jobs`, or the first model with no price.
pub fn estimate_total(jobs: &[(&Loaded, String, String, u32)]) -> Result<f64, String> {
    let mut total = 0.0;
    for (c, m, e, _) in jobs {
        let chars = c.system.len() + c.user.len();
        total += estimate_call(m, e, chars)
            .ok_or_else(|| format!("no price for model '{m}', cannot estimate the cost"))?;
    }
    Ok(total)
}

/// Runs the benchmark and returns the process exit code.
pub fn run(opts: &Options) -> i32 {
    let cases = match load_cases(&opts.cases, &opts.repo_root) {
        Ok(c) => c,
        Err(e) => return fail(&e),
    };
    if opts.list {
        for c in &cases {
            println!("{}\t{}", c.case.role, c.case.id);
        }
        return 0;
    }
    if !tool_on_path("claude") {
        return fail("claude CLI is required (needs a live login)");
    }
    if opts.models.is_empty() || opts.efforts.is_empty() || opts.runs == 0 {
        return fail("give at least one --model, one --effort and --runs 1 or more");
    }
    let jobs = plan(&cases, opts);
    if jobs.is_empty() {
        return fail("no case matches --role and --id");
    }
    let estimate = match estimate_total(&jobs) {
        Ok(e) => e,
        Err(e) => return fail(&e),
    };
    eprintln!(
        "bench: {} call(s), estimated ${:.2}, cap ${:.2}",
        jobs.len(),
        estimate,
        opts.max_cost_usd
    );
    if estimate > opts.max_cost_usd {
        return fail(&format!(
            "estimated cost ${estimate:.2} is over --max-cost-usd ${:.2}; narrow --role, --id, --model or --effort, lower --runs, or raise the cap",
            opts.max_cost_usd
        ));
    }
    let spent = Mutex::new(0.0f64);
    let cells: Vec<Cell> = par::map(&jobs, opts.jobs.max(1), |(c, m, e, r)| {
        let skipped = |outcome| Cell {
            id: c.case.id.clone(),
            role: c.case.role.clone(),
            model: m.clone(),
            effort: e.clone(),
            run: *r,
            outcome,
        };
        if *spent.lock().unwrap_or_else(|x| x.into_inner()) >= opts.max_cost_usd {
            return skipped(Outcome::Skipped);
        }
        match call_claude(c, m, e) {
            Err(err) => skipped(Outcome::Errored(err)),
            Ok((reply, cost, resolved, wall)) => {
                *spent.lock().unwrap_or_else(|x| x.into_inner()) += cost;
                match judge(&c.case.score, &reply) {
                    Ok((ok, detail)) => skipped(Outcome::Scored {
                        ok,
                        detail,
                        cost,
                        wall_s: wall,
                        resolved,
                    }),
                    Err(e) => skipped(Outcome::Errored(e)),
                }
            }
        }
    });
    let total_spent = *spent.lock().unwrap_or_else(|x| x.into_inner());
    if opts.json {
        println!("{}", render_json(&cells, total_spent, opts.max_cost_usd));
    } else {
        print!("{}", render_table(&cells, total_spent));
    }
    let scored = cells
        .iter()
        .filter(|c| matches!(c.outcome, Outcome::Scored { .. }))
        .count();
    i32::from(scored == 0)
}

fn fail(msg: &str) -> i32 {
    eprintln!("eval bench: {msg}");
    1
}

// ---------- output ----------

type GroupKey = (String, String, String);

struct Group {
    n: usize,
    pass: usize,
    errored: usize,
    skipped: usize,
    cost: f64,
    wall: f64,
}

fn groups(cells: &[Cell]) -> BTreeMap<GroupKey, Group> {
    let mut map: BTreeMap<GroupKey, Group> = BTreeMap::new();
    for c in cells {
        let g = map
            .entry((c.role.clone(), c.model.clone(), c.effort.clone()))
            .or_insert(Group {
                n: 0,
                pass: 0,
                errored: 0,
                skipped: 0,
                cost: 0.0,
                wall: 0.0,
            });
        match &c.outcome {
            Outcome::Scored {
                ok, cost, wall_s, ..
            } => {
                g.n += 1;
                g.pass += usize::from(*ok);
                g.cost += cost;
                g.wall += wall_s;
            }
            Outcome::Errored(_) => g.errored += 1,
            Outcome::Skipped => g.skipped += 1,
        }
    }
    map
}

pub fn render_table(cells: &[Cell], spent: f64) -> String {
    let mut out = String::new();
    out.push_str("role\tmodel\teffort\tpass\tcost/call\twall(s)\terrored\tskipped\n");
    for ((role, model, effort), g) in groups(cells) {
        let n = g.n.max(1) as f64;
        out.push_str(&format!(
            "{role}\t{model}\t{effort}\t{}/{} ({:.0}%)\t${:.5}\t{:.1}\t{}\t{}\n",
            g.pass,
            g.n,
            100.0 * g.pass as f64 / n,
            g.cost / n,
            g.wall / n,
            g.errored,
            g.skipped
        ));
    }
    for c in cells {
        match &c.outcome {
            Outcome::Scored {
                ok: false, detail, ..
            } => {
                out.push_str(&format!(
                    "fail\t{}\t{}\t{}\trun {}\t{}\n",
                    c.id, c.model, c.effort, c.run, detail
                ));
            }
            Outcome::Errored(e) => {
                out.push_str(&format!(
                    "error\t{}\t{}\t{}\trun {}\t{}\n",
                    c.id, c.model, c.effort, c.run, e
                ));
            }
            _ => {}
        }
    }
    out.push_str(&format!("total spend ${spent:.4}\n"));
    out
}

pub fn render_json(cells: &[Cell], spent: f64, cap: f64) -> String {
    let rows: Vec<Value> = cells
        .iter()
        .map(|c| match &c.outcome {
            Outcome::Scored {
                ok,
                detail,
                cost,
                wall_s,
                resolved,
            } => json!({"id": c.id, "role": c.role, "model": c.model, "effort": c.effort,
                "run": c.run, "ok": ok, "detail": detail, "cost": cost, "wall_s": wall_s,
                "resolved": resolved}),
            Outcome::Errored(e) => json!({"id": c.id, "role": c.role, "model": c.model,
                "effort": c.effort, "run": c.run, "error": e}),
            Outcome::Skipped => json!({"id": c.id, "role": c.role, "model": c.model,
                "effort": c.effort, "run": c.run, "skipped": true}),
        })
        .collect();
    let summary: Vec<Value> = groups(cells)
        .into_iter()
        .map(|((role, model, effort), g)| {
            let n = g.n.max(1) as f64;
            json!({"role": role, "model": model, "effort": effort, "n": g.n, "pass": g.pass,
                "pass_rate": g.pass as f64 / n, "cost_per_call": g.cost / n,
                "wall_s_per_call": g.wall / n, "errored": g.errored, "skipped": g.skipped})
        })
        .collect();
    json!({"spent_usd": spent, "cap_usd": cap, "summary": summary, "cells": rows}).to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn findings(bug: &[i64], kw: Option<&str>) -> Score {
        Score::Findings {
            bug_lines: bug.to_vec(),
            keywords: kw.map(str::to_string),
            tolerance: 1,
        }
    }

    #[test]
    fn agent_body_drops_the_frontmatter() {
        assert_eq!(agent_body("---\nname: x\n---\nBody\nTwo\n"), "Body\nTwo");
    }

    #[test]
    fn parse_json_accepts_fences_prose_and_arrays() {
        assert!(parse_json("```json\n{\"a\":1}\n```").is_some());
        assert!(parse_json("Here:\n[{\"line\":1}]\nok").is_some());
        assert!(parse_json("nothing {[ here").is_none());
    }

    #[test]
    fn a_finding_near_a_planted_line_or_matching_the_keywords_passes() {
        let s = findings(&[10], Some("overflow"));
        assert!(
            judge(&s, r#"{"findings":[{"line":11,"issue":"x"}]}"#)
                .unwrap()
                .0
        );
        assert!(
            judge(&s, r#"[{"line":99,"issue":"integer OVERFLOW here"}]"#)
                .unwrap()
                .0
        );
        assert!(
            !judge(&s, r#"{"findings":[{"line":40,"issue":"style"}]}"#)
                .unwrap()
                .0
        );
        assert!(!judge(&s, "not json").unwrap().0);
    }

    #[test]
    fn a_clean_case_fails_only_on_a_blocking_finding() {
        let s = findings(&[], None);
        assert!(judge(&s, r#"{"findings":[]}"#).unwrap().0);
        assert!(
            judge(&s, r#"{"findings":[{"severity":"nit","line":1}]}"#)
                .unwrap()
                .0
        );
        assert!(
            !judge(&s, r#"{"findings":[{"severity":"blocking","line":1}]}"#)
                .unwrap()
                .0
        );
    }

    #[test]
    fn claims_must_match_exactly() {
        let truth = BTreeMap::from([("1".to_string(), true), ("2".to_string(), false)]);
        let s = Score::Claims { truth };
        assert!(judge(&s, r#"{"claims":{"1":true,"2":false}}"#).unwrap().0);
        let bad = judge(&s, r#"{"claims":{"1":true,"2":true}}"#).unwrap();
        assert!(!bad.0 && bad.1 == "wrong=2");
        assert!(!judge(&s, r#"{"claims":{"1":true}}"#).unwrap().0);
    }

    #[test]
    fn verdicts_compare_case_insensitively() {
        let s = Score::Verdict {
            want: "FAIL".into(),
        };
        assert!(judge(&s, r#"{"verdict":"fail"}"#).unwrap().0);
        assert!(!judge(&s, r#"{"verdict":"PASS"}"#).unwrap().0);
    }

    #[test]
    fn facts_reject_invented_anchors_and_need_the_required_ones() {
        let s = Score::Facts {
            allowed_anchors: vec!["src/a.rs".into(), "src/b.rs".into()],
            required_anchors: vec!["src/a.rs".into()],
        };
        let ok =
            r#"{"facts":[{"title":"A","type":"project","scope":"repo","anchors":["src/a.rs"]}]}"#;
        assert!(judge(&s, ok).unwrap().0);
        let invented =
            r#"{"facts":[{"title":"A","type":"project","scope":"repo","anchors":["src/zzz.rs"]}]}"#;
        assert!(judge(&s, invented).unwrap().1.starts_with("INVENTED"));
        let missing =
            r#"{"facts":[{"title":"A","type":"project","scope":"repo","anchors":["src/b.rs"]}]}"#;
        assert!(judge(&s, missing).unwrap().1.starts_with("MISSING"));
    }

    #[test]
    fn code_blocks_are_extracted() {
        assert_eq!(code_block("x\n```python\nprint(1)\n```\ny"), "print(1)\n");
        assert_eq!(code_block("plain"), "plain");
    }

    #[test]
    fn run_test_scores_a_solution_by_running_the_tests() {
        if !tool_on_path("python3") {
            return;
        }
        let s = Score::RunTest {
            files: BTreeMap::from([(
                "t.py".to_string(),
                "from sol import f\nassert f(2) == 4\n".to_string(),
            )]),
            solution: "sol.py".into(),
            cmd: vec!["python3".into(), "t.py".into()],
        };
        assert!(
            judge(&s, "```python\ndef f(x):\n    return x * 2\n```")
                .unwrap()
                .0
        );
        assert!(
            !judge(&s, "```python\ndef f(x):\n    return x + 1\n```")
                .unwrap()
                .0
        );
    }

    #[test]
    fn run_test_refuses_fixture_paths_that_escape() {
        if !tool_on_path("python3") {
            return;
        }
        let s = Score::RunTest {
            files: BTreeMap::from([("../x.py".to_string(), "".to_string())]),
            solution: "sol.py".into(),
            cmd: vec!["python3".into(), "-c".into(), "pass".into()],
        };
        assert!(judge(&s, "x").is_err());
    }

    #[test]
    fn the_estimate_grows_with_effort_and_refuses_unknown_models() {
        let low = estimate_call("haiku", "low", 4000).unwrap();
        let max = estimate_call("haiku", "max", 4000).unwrap();
        assert!(max > low);
        assert!(estimate_call("sonnet", "low", 4000).unwrap() > low);
        assert!(estimate_call("mystery-model", "low", 4000).is_none());
    }

    #[test]
    fn the_table_groups_by_role_model_and_effort() {
        let cell = |ok: bool, run| Cell {
            id: "a".into(),
            role: "reviewer".into(),
            model: "haiku".into(),
            effort: "low".into(),
            run,
            outcome: Outcome::Scored {
                ok,
                detail: "d".into(),
                cost: 0.001,
                wall_s: 2.0,
                resolved: vec![],
            },
        };
        let t = render_table(&[cell(true, 1), cell(false, 2)], 0.002);
        assert!(t.contains("reviewer\thaiku\tlow\t1/2 (50%)"), "{t}");
        assert!(t.contains("total spend $0.0020"));
    }
}
