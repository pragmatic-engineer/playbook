// SPDX-FileCopyrightText: 2026 Igor Santos
// SPDX-License-Identifier: Apache-2.0

//! `UsageSource` over Codex CLI rollout logs
//! (`~/.codex/sessions/YYYY/MM/DD/rollout-*.jsonl`), read-only. Format from
//! openai/codex `codex-rs/protocol/src/protocol.rs` (`TokenCountEvent`,
//! `TokenUsageInfo`, `SessionMeta`, `TurnContextItem`).
//!
//! Each line is `{timestamp, type, payload}`. `session_meta` carries the
//! session id, cwd and git branch, `turn_context` the model, effort and cwd,
//! and `event_msg` with `payload.type == "token_count"` the usage. Codex
//! repeats a token_count when nothing changed, so an event is one change of
//! the cumulative `total_token_usage.total_tokens`, priced from
//! `last_token_usage`. Compressed `.jsonl.zst` rollouts are not read.

use super::claude_code::{collect_jsonl, modified_before, parse_iso8601_utc, read_lines, token};
use super::repo::RepoResolver;
use super::{Events, UsageEvent, UsageSource};
use serde_json::Value;
use std::path::PathBuf;

pub struct CodexSource {
    root: PathBuf,
}

impl CodexSource {
    pub fn new(root: PathBuf) -> Self {
        Self { root }
    }
}

impl UsageSource for CodexSource {
    fn name(&self) -> &str {
        "codex"
    }

    fn events_since(&self, watermark: i64) -> Result<Events, String> {
        let mut files = Vec::new();
        collect_jsonl(&self.root, &mut files);
        files.retain(|f| is_rollout(f) && !modified_before(f, watermark));
        files.sort();

        let mut repos = RepoResolver::default();
        let mut usage = Vec::new();
        for file in files {
            let mut state = Session {
                id: file_session_id(&file),
                ..Session::default()
            };
            read_lines(&file, &mut |line| {
                if let Ok(value) = serde_json::from_str::<Value>(line) {
                    if let Some(event) = state.apply(&value, watermark, &mut repos) {
                        usage.push(event);
                    }
                }
            });
        }
        Ok(Events {
            usage,
            tools: Vec::new(),
        })
    }
}

fn is_rollout(path: &std::path::Path) -> bool {
    path.file_name()
        .and_then(|n| n.to_str())
        .is_some_and(|n| n.starts_with("rollout-"))
}

/// The trailing uuid of `rollout-<time>-<uuid>.jsonl`, used until a
/// `session_meta` line names the session.
fn file_session_id(path: &std::path::Path) -> String {
    let stem = path.file_stem().and_then(|s| s.to_str()).unwrap_or("");
    let parts: Vec<&str> = stem.split('-').collect();
    if parts.len() > 5 {
        parts[parts.len() - 5..].join("-")
    } else {
        stem.to_string()
    }
}

#[derive(Default)]
struct Session {
    id: String,
    cwd: String,
    branch: String,
    model: String,
    effort: String,
    last_total: Option<u64>,
}

impl Session {
    fn apply(
        &mut self,
        line: &Value,
        watermark: i64,
        repos: &mut RepoResolver,
    ) -> Option<UsageEvent> {
        let payload = line.get("payload")?;
        match line.get("type")?.as_str()? {
            "session_meta" => self.read_meta(payload),
            "turn_context" => self.read_context(payload),
            "event_msg" if payload.get("type").and_then(Value::as_str) == Some("token_count") => {
                return self.usage_event(line, payload, watermark, repos);
            }
            _ => {}
        }
        None
    }

    fn read_meta(&mut self, payload: &Value) {
        if let Some(id) = text(payload, "id") {
            self.id = id;
        }
        if let Some(cwd) = text(payload, "cwd") {
            self.cwd = cwd;
        }
        if let Some(branch) = payload
            .get("git")
            .and_then(|g| g.get("branch"))
            .and_then(Value::as_str)
        {
            self.branch = branch.to_string();
        }
    }

    fn read_context(&mut self, payload: &Value) {
        if let Some(cwd) = text(payload, "cwd") {
            self.cwd = cwd;
        }
        if let Some(model) = text(payload, "model") {
            self.model = model;
        }
        if let Some(effort) = text(payload, "effort") {
            self.effort = effort;
        }
    }

    fn usage_event(
        &mut self,
        line: &Value,
        payload: &Value,
        watermark: i64,
        repos: &mut RepoResolver,
    ) -> Option<UsageEvent> {
        let info = payload.get("info").filter(|i| i.is_object())?;
        let total = token(info.get("total_token_usage")?, "total_tokens");
        if self.last_total == Some(total) {
            return None;
        }
        self.last_total = Some(total);
        let timestamp = line
            .get("timestamp")
            .and_then(Value::as_str)
            .and_then(parse_iso8601_utc)?;
        if timestamp < watermark {
            return None;
        }
        let last = info.get("last_token_usage")?;
        let input = token(last, "input_tokens");
        let cached = token(last, "cached_input_tokens").min(input);
        let mut event = UsageEvent {
            event_id: format!("codex:{}:{timestamp}:{total}", self.id),
            timestamp,
            session_id: self.id.clone(),
            agent: "codex".to_string(),
            model: self.model.clone(),
            effort: self.effort.clone(),
            repo: repos.resolve(&self.cwd),
            branch: self.branch.clone(),
            input_tokens: input - cached,
            // Codex counts reasoning tokens inside output_tokens.
            output_tokens: token(last, "output_tokens"),
            cache_read_tokens: cached,
            ..UsageEvent::default()
        };
        if event.input_tokens + event.output_tokens + event.cache_read_tokens == 0 {
            return None;
        }
        event.apply_pricing();
        Some(event)
    }
}

fn text(value: &Value, key: &str) -> Option<String> {
    value
        .get(key)
        .and_then(Value::as_str)
        .filter(|s| !s.is_empty())
        .map(str::to_string)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::usage::{db, ingest::ingest};

    fn root() -> PathBuf {
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/usage/codex")
    }

    #[test]
    fn token_counts_become_events_with_session_model_repo_and_branch() {
        let events = CodexSource::new(root()).events_since(0).unwrap();
        assert!(events.tools.is_empty());
        assert_eq!(
            events.usage.len(),
            4,
            "null info, repeat, zero usage, junk and a non-rollout file are skipped"
        );

        let first = &events.usage[0];
        assert_eq!(
            first.event_id,
            "codex:0199aaaa-0000-7000-8000-000000000001:1788249610:1050"
        );
        assert_eq!(first.timestamp, 1788249610);
        assert_eq!(first.agent, "codex");
        assert_eq!(first.model, "gpt-5-codex");
        assert_eq!(first.effort, "high");
        assert_eq!(first.repo, "proj-codex");
        assert_eq!(first.branch, "feat/x");
        assert_eq!(
            (
                first.input_tokens,
                first.cache_read_tokens,
                first.output_tokens
            ),
            (800, 200, 50)
        );
        assert!(first.unpriced, "OpenAI models are not in the price table");
        assert_eq!(first.cost_usd, 0.0);

        let second = &events.usage[1];
        assert_eq!(
            second.event_id,
            "codex:0199aaaa-0000-7000-8000-000000000001:1788249660:3150"
        );
        assert_eq!(
            (
                second.input_tokens,
                second.cache_read_tokens,
                second.output_tokens
            ),
            (700, 1300, 100)
        );
    }

    #[test]
    fn a_rollout_without_session_meta_uses_the_file_name_id_and_context_cwd() {
        let events = CodexSource::new(root()).events_since(0).unwrap();
        let third = &events.usage[2];
        assert_eq!(third.session_id, "0199aaaa-0000-7000-8000-000000000002");
        assert_eq!(third.repo, "proj-other");
        assert_eq!(third.model, "gpt-5");
        assert_eq!(third.branch, "");
    }

    #[test]
    fn cached_tokens_above_input_are_clamped_not_wrapped() {
        let events = CodexSource::new(root()).events_since(0).unwrap();
        let clamped = &events.usage[3];
        assert_eq!(
            (
                clamped.input_tokens,
                clamped.cache_read_tokens,
                clamped.output_tokens
            ),
            (0, 50, 0)
        );
    }

    #[test]
    fn the_watermark_drops_older_events() {
        let events = CodexSource::new(root()).events_since(1788249660).unwrap();
        let ids: Vec<_> = events.usage.iter().map(|e| e.timestamp).collect();
        assert_eq!(ids, vec![1788249660, 1788339605, 1788340200]);
    }

    #[test]
    fn a_missing_directory_yields_nothing() {
        let events = CodexSource::new(root().join("nope"))
            .events_since(0)
            .unwrap();
        assert_eq!(events, Events::default());
    }

    #[test]
    fn rerunning_ingest_does_not_duplicate_events() {
        let dir = crate::common::test_support::scratch_dir("codex-ingest");
        let conn = db::open_db(&dir.join("usage.db")).unwrap();
        let source = CodexSource::new(root());
        let first = ingest(&source, "unknown", &conn).unwrap();
        conn.execute(
            "UPDATE usage_watermarks SET watermark = 0 WHERE source = 'codex'",
            [],
        )
        .unwrap();
        let again = ingest(&source, "unknown", &conn).unwrap();
        assert_eq!(first.usage_inserted, 4);
        assert_eq!(again.usage_inserted, 0);
        assert_eq!(again.duplicates_skipped, 4);
        assert_eq!(again.usage_updated, 0);
        assert_eq!(db::count_usage_events(&conn).unwrap(), 4);
        let stored = db::load_usage_events(&conn).unwrap();
        assert!(stored.iter().all(|e| e.agent == "codex"));
    }
}
