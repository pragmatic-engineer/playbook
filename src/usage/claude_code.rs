// SPDX-FileCopyrightText: 2026 Igor Santos
// SPDX-License-Identifier: MIT

//! `UsageSource` over Claude Code's own session transcripts
//! (`~/.claude/projects/**/*.jsonl`). The root is injected so tests never
//! read the real home directory.
//!
//! One assistant message is split across several transcript lines (one per
//! content block), each repeating the same `usage`. Events are therefore
//! deduped by `message.id` (usage) and `tool_use.id` (tools), not by line.

use super::pricing::cost_usd;
use super::{Events, ToolInvocationEvent, ToolKind, UsageEvent, UsageSource};
use serde_json::Value;
use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};

pub struct ClaudeCodeSource {
    root: PathBuf,
}

impl ClaudeCodeSource {
    pub fn new(root: PathBuf) -> Self {
        Self { root }
    }
}

impl UsageSource for ClaudeCodeSource {
    fn name(&self) -> &str {
        "claude-code"
    }

    fn events_since(&self, watermark: i64) -> Result<Events, String> {
        let mut files = Vec::new();
        collect_jsonl(&self.root, &mut files);
        files.sort();

        let mut usage: Vec<UsageEvent> = Vec::new();
        let mut usage_index: HashMap<String, usize> = HashMap::new();
        let mut tools: Vec<ToolInvocationEvent> = Vec::new();
        let mut tool_seen: HashMap<String, ()> = HashMap::new();

        for file in files {
            let Ok(text) = fs::read_to_string(&file) else {
                continue;
            };
            for line in text.lines() {
                let Ok(value) = serde_json::from_str::<Value>(line) else {
                    continue;
                };
                if value.get("type").and_then(Value::as_str) != Some("assistant") {
                    continue;
                }
                let Some(timestamp) = value
                    .get("timestamp")
                    .and_then(Value::as_str)
                    .and_then(parse_iso8601_utc)
                else {
                    continue;
                };
                if timestamp < watermark {
                    continue;
                }
                if let Some(event) = usage_event(&value, timestamp) {
                    match usage_index.get(&event.event_id) {
                        Some(&i) if event.output_tokens > usage[i].output_tokens => {
                            usage[i] = event
                        }
                        Some(_) => {}
                        None => {
                            usage_index.insert(event.event_id.clone(), usage.len());
                            usage.push(event);
                        }
                    }
                }
                for tool in tool_events(&value, timestamp) {
                    if tool_seen.insert(tool.event_id.clone(), ()).is_none() {
                        tools.push(tool);
                    }
                }
            }
        }
        Ok(Events { usage, tools })
    }
}

fn collect_jsonl(dir: &Path, out: &mut Vec<PathBuf>) {
    let Ok(entries) = fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            collect_jsonl(&path, out);
        } else if path.extension().and_then(|e| e.to_str()) == Some("jsonl") {
            out.push(path);
        }
    }
}

fn str_field<'a>(value: &'a Value, key: &str) -> &'a str {
    value.get(key).and_then(Value::as_str).unwrap_or("")
}

fn token(usage: &Value, key: &str) -> u64 {
    usage.get(key).and_then(Value::as_u64).unwrap_or(0)
}

fn usage_event(line: &Value, timestamp: i64) -> Option<UsageEvent> {
    let message = line.get("message")?;
    let event_id = message.get("id")?.as_str()?.to_string();
    let usage = message.get("usage")?;
    let model = str_field(message, "model").to_string();
    let input = token(usage, "input_tokens");
    let output = token(usage, "output_tokens");
    let cache_write = token(usage, "cache_creation_input_tokens");
    let cache_read = token(usage, "cache_read_input_tokens");
    let cwd = str_field(line, "cwd");
    Some(UsageEvent {
        event_id,
        timestamp,
        session_id: str_field(line, "sessionId").to_string(),
        account: String::new(),
        cost_usd: cost_usd(&model, input, output, cache_write, cache_read),
        model,
        effort: str_field(line, "effort").to_string(),
        repo: Path::new(cwd)
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or("")
            .to_string(),
        branch: str_field(line, "gitBranch").to_string(),
        input_tokens: input,
        output_tokens: output,
        cache_creation_tokens: cache_write,
        cache_read_tokens: cache_read,
    })
}

fn tool_events(line: &Value, timestamp: i64) -> Vec<ToolInvocationEvent> {
    let Some(blocks) = line
        .get("message")
        .and_then(|m| m.get("content"))
        .and_then(Value::as_array)
    else {
        return Vec::new();
    };
    let session_id = str_field(line, "sessionId");
    blocks
        .iter()
        .filter(|b| b.get("type").and_then(Value::as_str) == Some("tool_use"))
        .filter_map(|b| {
            let input = b.get("input")?;
            let (kind, name) = match b.get("name")?.as_str()? {
                "Skill" => (ToolKind::Skill, str_field(input, "skill")),
                "Agent" => {
                    let subagent = str_field(input, "subagent_type");
                    let name = if subagent.is_empty() {
                        str_field(input, "description")
                    } else {
                        subagent
                    };
                    (ToolKind::Agent, name)
                }
                _ => return None,
            };
            Some(ToolInvocationEvent {
                event_id: b.get("id")?.as_str()?.to_string(),
                timestamp,
                session_id: session_id.to_string(),
                account: String::new(),
                kind,
                name: name.to_string(),
            })
        })
        .collect()
}

/// `2026-09-01T08:51:22.982Z` to epoch seconds (UTC). Fractional seconds are
/// dropped. No date crate exists in this binary, so this uses the standard
/// days-from-civil conversion.
fn parse_iso8601_utc(s: &str) -> Option<i64> {
    let (date, time) = s.split_once('T')?;
    let mut d = date.split('-');
    let year: i64 = d.next()?.parse().ok()?;
    let month: i64 = d.next()?.parse().ok()?;
    let day: i64 = d.next()?.parse().ok()?;
    let time = time.trim_end_matches('Z');
    let time = time.split('.').next()?;
    let mut t = time.split(':');
    let hour: i64 = t.next()?.parse().ok()?;
    let minute: i64 = t.next()?.parse().ok()?;
    let second: i64 = t.next()?.parse().ok()?;
    let y = if month <= 2 { year - 1 } else { year };
    let era = y.div_euclid(400);
    let yoe = y - era * 400;
    let mp = (month + 9) % 12;
    let doy = (153 * mp + 2) / 5 + day - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    let days = era * 146097 + doe - 719468;
    Some(days * 86400 + hour * 3600 + minute * 60 + second)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn root(name: &str) -> PathBuf {
        PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("tests/fixtures/usage")
            .join(name)
    }

    #[test]
    fn parses_timestamps_to_independently_computed_epochs() {
        assert_eq!(
            parse_iso8601_utc("2026-09-01T08:51:22.982Z"),
            Some(1788252682)
        );
        assert_eq!(
            parse_iso8601_utc("2026-09-02T00:00:01.000Z"),
            Some(1788307201)
        );
        assert_eq!(parse_iso8601_utc("not a time"), None);
    }

    #[test]
    fn usage_events_dedup_by_message_id_and_skip_malformed_lines() {
        let events = ClaudeCodeSource::new(root("usage"))
            .events_since(0)
            .unwrap();

        assert_eq!(
            events.usage.len(),
            3,
            "msg_A spans two lines but is one message"
        );
        let a = &events.usage[0];
        assert_eq!(a.event_id, "msg_A");
        assert_eq!(a.timestamp, 1788252682);
        assert_eq!(a.model, "claude-sonnet-5");
        assert_eq!(a.effort, "high");
        assert_eq!(a.repo, "proj-one");
        assert_eq!(a.branch, "main");
        assert_eq!(a.session_id, "s1");
        assert_eq!(
            (
                a.input_tokens,
                a.output_tokens,
                a.cache_creation_tokens,
                a.cache_read_tokens
            ),
            (2, 770, 29653, 22178)
        );
        assert!((a.cost_usd - 0.12940815).abs() < 1e-9);

        let b = &events.usage[1];
        assert_eq!(
            (b.event_id.as_str(), b.model.as_str(), b.effort.as_str()),
            ("msg_B", "claude-opus-5", "xhigh")
        );
        assert_eq!((b.repo.as_str(), b.branch.as_str()), ("proj-two", "feat/x"));
        assert!((b.cost_usd - 0.00755).abs() < 1e-9);

        let c = &events.usage[2];
        assert_eq!((c.event_id.as_str(), c.effort.as_str()), ("msg_C", ""));
        assert!((c.cost_usd - 0.0035).abs() < 1e-9);
    }

    #[test]
    fn watermark_is_inclusive_and_filters_older_events() {
        let source = ClaudeCodeSource::new(root("usage"));

        let at_boundary = source.events_since(1788253200).unwrap();
        let ids: Vec<&str> = at_boundary
            .usage
            .iter()
            .map(|e| e.event_id.as_str())
            .collect();
        assert_eq!(ids, vec!["msg_B", "msg_C"]);
    }

    #[test]
    fn tool_events_extract_skills_and_agents_with_and_without_subagent_type() {
        let events = ClaudeCodeSource::new(root("tools"))
            .events_since(0)
            .unwrap();

        let got: Vec<(&str, &str, &str)> = events
            .tools
            .iter()
            .map(|t| (t.event_id.as_str(), t.kind.as_str(), t.name.as_str()))
            .collect();
        assert_eq!(
            got,
            vec![
                ("tu_1", "skill", "playbook:plan"),
                ("tu_2", "skill", "playbook:implement"),
                ("tu_3", "agent", "Explore"),
                ("tu_4", "agent", "Draft briefs"),
            ],
            "tu_4 appears on two lines but is one invocation"
        );
    }

    #[test]
    fn a_root_that_does_not_exist_yields_no_events() {
        let source = ClaudeCodeSource::new(root("no-such-dir"));

        let events = source.events_since(0).unwrap();

        assert!(events.usage.is_empty() && events.tools.is_empty());
    }
}
