// SPDX-FileCopyrightText: 2026 Igor Santos
// SPDX-License-Identifier: MIT

//! `UsageSource` over Claude Code's own session transcripts
//! (`~/.claude/projects/**/*.jsonl`). The root is injected so tests never
//! read the real home directory.
//!
//! One assistant message is split across several transcript lines (one per
//! content block), each repeating the same `usage`. Events are therefore
//! deduped by `message.id` (usage) and `tool_use.id` (tools), not by line.

use super::repo::RepoResolver;
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
        // A file last modified before the watermark cannot hold a newer
        // event, so unchanged transcripts are never re-read. This keeps a
        // repeated ingest (the dashboard polls every few seconds) cheap.
        files.retain(|f| !modified_before(f, watermark));

        let mut usage: Vec<UsageEvent> = Vec::new();
        let mut usage_index: HashMap<String, usize> = HashMap::new();
        let mut tools: Vec<ToolInvocationEvent> = Vec::new();
        let mut tool_seen: HashMap<String, ()> = HashMap::new();
        let mut repos = RepoResolver::default();

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
                if let Some(event) = usage_event(&value, timestamp, &mut repos) {
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

fn modified_before(path: &Path, watermark: i64) -> bool {
    let Ok(modified) = fs::metadata(path).and_then(|m| m.modified()) else {
        return false;
    };
    let Ok(since_epoch) = modified.duration_since(std::time::UNIX_EPOCH) else {
        return false;
    };
    i64::try_from(since_epoch.as_secs()).is_ok_and(|secs| secs < watermark)
}

fn str_field<'a>(value: &'a Value, key: &str) -> &'a str {
    value.get(key).and_then(Value::as_str).unwrap_or("")
}

fn token(usage: &Value, key: &str) -> u64 {
    usage.get(key).and_then(Value::as_u64).unwrap_or(0)
}

/// Tokens written to the one hour cache, from `usage.cache_creation`. Absent
/// means the whole write counts as the five minute cache.
fn one_hour_cache_tokens(usage: &Value) -> u64 {
    usage
        .get("cache_creation")
        .map(|tiers| token(tiers, "ephemeral_1h_input_tokens"))
        .unwrap_or(0)
}

fn usage_event(line: &Value, timestamp: i64, repos: &mut RepoResolver) -> Option<UsageEvent> {
    let message = line.get("message")?;
    let event_id = message.get("id")?.as_str()?.to_string();
    let usage = message.get("usage")?;
    let cache_write = token(usage, "cache_creation_input_tokens");
    let mut event = UsageEvent {
        event_id,
        timestamp,
        session_id: str_field(line, "sessionId").to_string(),
        model: str_field(message, "model").to_string(),
        effort: str_field(line, "effort").to_string(),
        repo: repos.resolve(str_field(line, "cwd")),
        branch: str_field(line, "gitBranch").to_string(),
        input_tokens: token(usage, "input_tokens"),
        output_tokens: token(usage, "output_tokens"),
        cache_creation_tokens: cache_write,
        cache_creation_1h_tokens: one_hour_cache_tokens(usage),
        cache_read_tokens: token(usage, "cache_read_input_tokens"),
        ..UsageEvent::default()
    };
    event.apply_pricing();
    Some(event)
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
        // Sonnet 5 is 2/10, all five minute cache: 2*2 + 770*10 + 29653*2.5 + 22178*0.2.
        assert!((a.cost_usd - 0.0862721).abs() < 1e-9);

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
    fn cache_tiers_repo_markers_and_unpriced_models_come_through() {
        let events = ClaudeCodeSource::new(root("tiers"))
            .events_since(0)
            .unwrap();
        let by_id = |id: &str| events.usage.iter().find(|e| e.event_id == id).unwrap();

        // Opus 5.5 is 4/20: 1000*4 + 500*20 + 2000*4*1.25 + 4000*4*2 + 10000*4*0.05.
        let t1 = by_id("msg_T1");
        assert_eq!(
            (t1.cache_creation_tokens, t1.cache_creation_1h_tokens),
            (6000, 4000)
        );
        assert!((t1.cost_usd - 0.058).abs() < 1e-9 && !t1.unpriced);
        assert_eq!(
            t1.repo, "playbook",
            "a .claude/worktrees path names its repo"
        );

        // Opus 4.9 is not in the price table, so it is flagged, not guessed.
        let t2 = by_id("msg_T2");
        assert_eq!((t2.cost_usd, t2.unpriced), (0.0, true));
        assert_eq!(
            t2.repo, "ward",
            "a .git/review-worktrees path names its repo"
        );

        // No tokens means nothing to price, even for an unknown model.
        let t3 = by_id("msg_T3");
        assert_eq!((t3.cost_usd, t3.unpriced), (0.0, false));

        // No tier breakdown: the whole write is the five minute cache.
        let t4 = by_id("msg_T4");
        assert_eq!(t4.cache_creation_1h_tokens, 0);
        assert!((t4.cost_usd - 0.0025).abs() < 1e-9);
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
    fn a_transcript_older_than_the_watermark_is_not_read_at_all() {
        let dir = crate::common::test_support::scratch_dir("old-transcript");
        let project = dir.join("p");
        fs::create_dir_all(&project).unwrap();
        let file = project.join("s.jsonl");
        fs::copy(root("usage").join("proj-one/s1.jsonl"), &file).unwrap();
        let old = std::time::UNIX_EPOCH + std::time::Duration::from_secs(1000);
        fs::File::options()
            .write(true)
            .open(&file)
            .unwrap()
            .set_modified(old)
            .unwrap();
        let source = ClaudeCodeSource::new(dir.clone());

        let skipped = source.events_since(2000).unwrap();
        let read = source.events_since(0).unwrap();

        // The file's events are newer than 2000, but the file itself was
        // last modified at 1000, so it is skipped without being parsed.
        assert!(skipped.usage.is_empty());
        assert_eq!(read.usage.len(), 3);
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn a_root_that_does_not_exist_yields_no_events() {
        let source = ClaudeCodeSource::new(root("no-such-dir"));

        let events = source.events_since(0).unwrap();

        assert!(events.usage.is_empty() && events.tools.is_empty());
    }
}
