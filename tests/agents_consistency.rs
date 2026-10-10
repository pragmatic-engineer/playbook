// SPDX-FileCopyrightText: 2026 Igor Santos
// SPDX-License-Identifier: Apache-2.0

//! Keeps the agent files, the model and effort docs, the routing table and the
//! variant names that commands and docs mention from drifting apart.

use playbook::agents::variants::{session_agents, Mode, TIERS};
use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::{Path, PathBuf};

const ROUTING_DOC: &str = "docs/internals/02-model-routing-and-memory.md";
const CONCEPTS_DOC: &str = "docs/concepts/03-why-the-pieces-are-shaped-this-way.md";
const MAX_DESCRIPTION_BYTES: usize = 300;

fn root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
}

fn read(rel: &str) -> String {
    fs::read_to_string(root().join(rel)).unwrap_or_else(|e| panic!("{rel}: {e}"))
}

/// A frontmatter value, with surrounding double quotes removed.
fn key(text: &str, name: &str) -> String {
    let block = &text[4..text.find("\n---").expect("closing ---")];
    block
        .lines()
        .find_map(|l| l.strip_prefix(&format!("{name}:")))
        .unwrap_or_else(|| panic!("missing {name}"))
        .trim()
        .trim_matches('"')
        .to_string()
}

/// Agent name -> (model, effort, description) from `agents/*.md`.
fn agents() -> BTreeMap<String, (String, String, String)> {
    let mut out = BTreeMap::new();
    for entry in fs::read_dir(root().join("agents")).unwrap() {
        let path = entry.unwrap().path();
        if path.extension().is_some_and(|e| e == "md") {
            let name = path.file_stem().unwrap().to_string_lossy().to_string();
            let text = fs::read_to_string(&path).unwrap();
            out.insert(
                name,
                (
                    key(&text, "model"),
                    key(&text, "effort"),
                    key(&text, "description"),
                ),
            );
        }
    }
    out
}

/// The backticked agent names on one line, with an optional `agents/` prefix.
fn agent_names(line: &str, known: &BTreeSet<&str>) -> BTreeSet<String> {
    line.split('`')
        .skip(1)
        .step_by(2)
        .map(|s| s.trim_start_matches("agents/"))
        .filter(|s| known.contains(s))
        .map(str::to_string)
        .collect()
}

#[test]
fn descriptions_stay_short_because_every_session_lists_them() {
    for (name, (_, _, description)) in agents() {
        assert!(
            description.len() <= MAX_DESCRIPTION_BYTES,
            "agents/{name}.md description is {} bytes, the limit is {MAX_DESCRIPTION_BYTES}",
            description.len()
        );
    }
}

#[test]
fn the_tier_bullets_name_every_agent_under_its_own_model() {
    let all = agents();
    let known: BTreeSet<&str> = all.keys().map(String::as_str).collect();
    for doc in [ROUTING_DOC, CONCEPTS_DOC] {
        let text = read(doc);
        for (model, label) in [
            ("sonnet", "- **Sonnet**"),
            ("haiku", "- **Haiku**"),
            ("opus", "- **Opus**"),
        ] {
            let line = text
                .lines()
                .find(|l| l.starts_with(label))
                .unwrap_or_else(|| panic!("{doc}: no '{label}' bullet"));
            let named = agent_names(line, &known);
            let want: BTreeSet<String> = all
                .iter()
                .filter(|(_, (m, _, _))| m == model)
                .map(|(n, _)| n.clone())
                .collect();
            assert_eq!(
                named, want,
                "{doc}: the {label} bullet must name exactly the agents whose file says model: {model}"
            );
        }
    }
}

#[test]
fn the_routing_table_matches_the_routing_code_and_the_agent_files() {
    let all = agents();
    let text = read(ROUTING_DOC);
    let mut seen = 0;
    for line in text.lines().filter(|l| l.starts_with("| `")) {
        let cols: Vec<&str> = line.trim_matches('|').split('|').map(str::trim).collect();
        let kind = cols[0].trim_matches('`');
        if cols.len() < 3
            || !["mechanical", "classify", "check", "review", "design"].contains(&kind)
        {
            continue;
        }
        seen += 1;
        let route = playbook::routing::table(kind, "medium").expect("known kind");
        assert_eq!(
            cols[1],
            format!("{}, {}", route.model, route.effort),
            "doc route table row for `{kind}` differs from src/routing.rs"
        );
        let (model, effort, _) = &all[route.agent];
        // `design` names the critic as its verifier, the design itself runs
        // in a command on Opus.
        if kind != "design" {
            assert_eq!(
                &route.model, model,
                "route `{kind}` model vs agents/{}.md",
                route.agent
            );
        }
        // These routes run the agent as shipped. `classify` and `check` ask for
        // one tier above their Haiku files and reach it through a variant.
        if ["mechanical", "review"].contains(&kind) {
            assert_eq!(
                &route.effort, effort,
                "route `{kind}` effort vs agents/{}.md",
                route.agent
            );
        }
    }
    assert_eq!(seen, 5, "the doc route table lost a row");
    let implement = playbook::routing::table("implement", "medium").unwrap();
    let (model, effort, _) = &all["implementer"];
    assert_eq!(
        (implement.model, implement.effort),
        (model.as_str(), effort.as_str())
    );
}

/// The `session_agents` variant names for the shipped agents, in `all` mode.
fn every_variant() -> BTreeSet<String> {
    let dir = root().join("agents");
    session_agents(&dir, Mode::All, &|_| None)
        .keys()
        .cloned()
        .collect()
}

fn text_files(dir: &Path, out: &mut Vec<PathBuf>) {
    for entry in fs::read_dir(dir).unwrap() {
        let path = entry.unwrap().path();
        let skip = path
            .file_name()
            .is_some_and(|n| n == "adr" || n == "target");
        if path.is_dir() && !skip {
            text_files(&path, out);
        } else if path.extension().is_some_and(|e| e == "md") {
            out.push(path);
        }
    }
}

#[test]
fn a_variant_named_in_commands_skills_and_docs_exists() {
    let all = agents();
    let variants = every_variant();
    let mut files = vec![
        root().join("README.md"),
        root().join("prompts/SYSTEM_PROMPT.md"),
    ];
    for dir in ["commands", "skills", "docs"] {
        text_files(&root().join(dir), &mut files);
    }
    let mut problems = Vec::new();
    for file in files {
        let text = fs::read_to_string(&file).unwrap();
        for name in all.keys() {
            for tier in TIERS {
                let variant = format!("{name}-{tier}");
                let mut from = 0;
                while let Some(i) = text[from..].find(&variant) {
                    let at = from + i;
                    from = at + variant.len();
                    let before = text[..at].chars().last();
                    let after = text[from..].chars().next();
                    let word = |c: Option<char>| c.is_some_and(|c| c.is_alphanumeric() || c == '-');
                    if !word(before) && !word(after) && !variants.contains(&variant) {
                        problems.push(format!(
                            "{} names `{variant}`, which no session renders (a tier equal to the base effort renders nothing)",
                            file.strip_prefix(root()).unwrap().display()
                        ));
                    }
                }
            }
        }
    }
    problems.dedup();
    assert!(problems.is_empty(), "{}", problems.join("\n"));
}

#[test]
fn docs_list_exactly_the_agents_that_get_an_xhigh_variant() {
    let all = agents();
    let known: BTreeSet<&str> = all.keys().map(String::as_str).collect();
    let dir = root().join("agents");
    let rendered: BTreeSet<String> = session_agents(&dir, Mode::Auto, &|_| None)
        .keys()
        .cloned()
        .filter_map(|n| n.strip_suffix("-xhigh").map(str::to_string))
        .filter(|n| known.contains(n.as_str()))
        .collect();
    // Haiku stops at medium under the model cap, apart from opted-in roles.
    for name in &rendered {
        let model = &all[name].0;
        assert!(
            model != "haiku" || name == "git",
            "{name} runs on Haiku and still gets an xhigh variant"
        );
    }
    let escalating: BTreeSet<String> = rendered.iter().filter(|n| *n != "git").cloned().collect();
    let mut checked = 0;
    for doc in [
        "docs/authoring/02-authoring-agents.md",
        "docs/guides/04-config-keys.md",
    ] {
        for line in read(doc).lines().filter(|l| l.contains("plus `xhigh` ")) {
            let tail = &line[line.find("plus `xhigh` ").unwrap()..];
            let sentence = tail.split(". ").next().unwrap();
            assert_eq!(
                agent_names(sentence, &known),
                escalating,
                "{doc}: the agents that get an xhigh variant changed, update this sentence"
            );
            checked += 1;
        }
    }
    assert_eq!(checked, 2, "both docs must state the xhigh set");
}

#[test]
fn the_effort_docs_agree_with_the_reviewer_file() {
    let all = agents();
    assert_eq!(all["reviewer"].0, "opus");
    assert_eq!(
        all["reviewer"].1, "medium",
        "see ADR-0021 before raising it"
    );
    let doc = read(ROUTING_DOC);
    let row = doc
        .lines()
        .find(|l| l.starts_with("| agent | `reviewer` |"))
        .expect("reviewer policy row");
    assert!(row.contains("`opus` | `medium`"), "{row}");
    let quick = read("commands/quick-review.md");
    assert!(
        quick.contains("--cap medium"),
        "quick-review lost its medium cap"
    );
}
