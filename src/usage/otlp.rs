// SPDX-FileCopyrightText: 2026 Igor Santos
// SPDX-License-Identifier: Apache-2.0

//! Reads an OTLP/HTTP JSON logs body into telemetry requests.
//!
//! Only the `api_request` event is used: it carries one API request with its
//! own request id, so a redelivered batch cannot double count. Metrics are
//! ignored on purpose (they have no request id).
//!
//! Privacy: this parser is the allowlist. Only the keys in `ALLOWED_KEYS` are
//! ever read, so prompts, tool content, paths, emails, user ids and resource
//! attributes are dropped here and never reach storage. String values that
//! are kept (model, effort, agent name, ids) are checked for a safe charset
//! and length, and a value that fails the check is dropped.

use super::telemetry_db::TelemetryRequest;
use serde_json::Value;

/// The attribute keys this parser reads. Everything else is ignored.
pub const ALLOWED_KEYS: &[&str] = &[
    "event.name",
    "session.id",
    "request_id",
    "model",
    "effort",
    "agent.name",
    "skill.name",
    "input_tokens",
    "output_tokens",
    "cache_read_tokens",
    "cache_creation_tokens",
    "cost_usd",
];

/// The most log records read from one body; the rest are dropped.
pub const MAX_RECORDS: usize = 2000;

const MAX_NAME_LEN: usize = 64;
const MAX_ID_LEN: usize = 128;
const EVENT_PREFIX: &str = "claude_code.";

#[derive(Default)]
struct Fields {
    event: Option<String>,
    session: Option<String>,
    request: Option<String>,
    model: Option<String>,
    effort: Option<String>,
    agent: Option<String>,
    skill: Option<String>,
    input: Option<u64>,
    output: Option<u64>,
    cache_read: Option<u64>,
    cache_creation: Option<u64>,
    cost: Option<f64>,
}

/// A name or id value: ASCII letters, digits and `. _ - : /` only, within `max`.
fn safe_text(value: &str, max: usize) -> Option<String> {
    let ok = !value.is_empty()
        && value.len() <= max
        && value
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b"._-:/".contains(&b));
    ok.then(|| value.to_string())
}

fn any_value(v: &Value) -> Option<&Value> {
    let map = v.as_object()?;
    ["stringValue", "intValue", "doubleValue"]
        .iter()
        .find_map(|k| map.get(*k))
}

fn as_u64(v: &Value) -> Option<u64> {
    match v {
        Value::Number(n) => n.as_u64().or_else(|| {
            n.as_f64()
                .filter(|f| *f >= 0.0 && f.fract() == 0.0)
                .map(|f| f as u64)
        }),
        Value::String(s) => s.trim().parse().ok(),
        _ => None,
    }
}

fn as_f64(v: &Value) -> Option<f64> {
    let f = match v {
        Value::Number(n) => n.as_f64(),
        Value::String(s) => s.trim().parse().ok(),
        _ => None,
    }?;
    (f.is_finite() && f >= 0.0).then_some(f)
}

fn read_attributes(attrs: &Value) -> Fields {
    let mut f = Fields::default();
    let Some(list) = attrs.as_array() else {
        return f;
    };
    for attr in list {
        let Some(key) = attr.get("key").and_then(Value::as_str) else {
            continue;
        };
        if !ALLOWED_KEYS.contains(&key) {
            continue;
        }
        let Some(value) = attr.get("value").and_then(any_value) else {
            continue;
        };
        let text = value.as_str();
        match key {
            "event.name" => f.event = text.map(str::to_string),
            "session.id" => f.session = text.and_then(|t| safe_text(t, MAX_ID_LEN)),
            "request_id" => f.request = text.and_then(|t| safe_text(t, MAX_ID_LEN)),
            "model" => f.model = text.and_then(|t| safe_text(t, MAX_NAME_LEN)),
            "effort" => f.effort = text.and_then(|t| safe_text(t, MAX_NAME_LEN)),
            "agent.name" => f.agent = text.and_then(|t| safe_text(t, MAX_NAME_LEN)),
            "skill.name" => f.skill = text.and_then(|t| safe_text(t, MAX_NAME_LEN)),
            "input_tokens" => f.input = as_u64(value),
            "output_tokens" => f.output = as_u64(value),
            "cache_read_tokens" => f.cache_read = as_u64(value),
            "cache_creation_tokens" => f.cache_creation = as_u64(value),
            "cost_usd" => f.cost = as_f64(value),
            _ => {}
        }
    }
    f
}

fn nanos_to_secs(record: &Value) -> Option<i64> {
    let raw = record.get("timeUnixNano")?;
    let nanos = as_u64(raw)?;
    i64::try_from(nanos / 1_000_000_000).ok().filter(|s| *s > 0)
}

fn event_name(record: &Value, f: &Fields) -> Option<String> {
    let name = f.event.clone().or_else(|| {
        record
            .get("body")
            .and_then(any_value)
            .and_then(Value::as_str)
            .map(str::to_string)
    })?;
    Some(name.strip_prefix(EVENT_PREFIX).unwrap_or(&name).to_string())
}

/// Parses an OTLP JSON logs body. Records that are not a complete
/// `api_request` (no session id, no request id, or another event) are skipped.
/// `now` stamps a record that has no usable time.
pub fn parse_logs(body: &[u8], now: i64) -> Result<Vec<TelemetryRequest>, String> {
    let doc: Value = serde_json::from_slice(body).map_err(|e| format!("invalid JSON: {e}"))?;
    let resources = doc
        .get("resourceLogs")
        .and_then(Value::as_array)
        .ok_or("missing resourceLogs")?;
    let mut out = Vec::new();
    let mut seen = 0usize;
    for scopes in resources
        .iter()
        .filter_map(|r| r.get("scopeLogs")?.as_array())
    {
        for records in scopes
            .iter()
            .filter_map(|s| s.get("logRecords")?.as_array())
        {
            for record in records {
                seen += 1;
                if seen > MAX_RECORDS {
                    return Ok(out);
                }
                let f = read_attributes(record.get("attributes").unwrap_or(&Value::Null));
                if event_name(record, &f).as_deref() != Some("api_request") {
                    continue;
                }
                let (Some(session_id), Some(request_id)) = (f.session, f.request) else {
                    continue;
                };
                out.push(TelemetryRequest {
                    session_id,
                    request_id,
                    timestamp: nanos_to_secs(record).unwrap_or(now),
                    model: f.model.unwrap_or_default(),
                    effort: f.effort.unwrap_or_default(),
                    agent_name: f.agent.unwrap_or_default(),
                    skill_name: f.skill.unwrap_or_default(),
                    input_tokens: f.input.unwrap_or(0),
                    output_tokens: f.output.unwrap_or(0),
                    cache_creation_tokens: f.cache_creation.unwrap_or(0),
                    cache_read_tokens: f.cache_read.unwrap_or(0),
                    cost_usd: f.cost.unwrap_or(0.0),
                });
            }
        }
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn attr(k: &str, v: Value) -> Value {
        json!({"key": k, "value": v})
    }

    fn s(k: &str, v: &str) -> Value {
        attr(k, json!({"stringValue": v}))
    }

    fn n(k: &str, v: i64) -> Value {
        attr(k, json!({"intValue": v.to_string()}))
    }

    fn body(records: Vec<Value>) -> Vec<u8> {
        json!({"resourceLogs": [{
            "resource": {"attributes": [s("user.email", "a@b.c")]},
            "scopeLogs": [{"logRecords": records}]
        }]})
        .to_string()
        .into_bytes()
    }

    fn api_request(extra: Vec<Value>) -> Value {
        let mut attrs = vec![
            s("event.name", "api_request"),
            s("session.id", "sess-1"),
            s("request_id", "req_1"),
            s("model", "claude-opus-5"),
            s("effort", "high"),
            s("agent.name", "reviewer"),
            n("input_tokens", 10),
            n("output_tokens", 20),
            n("cache_read_tokens", 30),
            n("cache_creation_tokens", 40),
            attr("cost_usd", json!({"doubleValue": 0.5})),
        ];
        attrs.extend(extra);
        json!({"timeUnixNano": "1760000000000000000", "attributes": attrs})
    }

    #[test]
    fn an_api_request_becomes_one_row() {
        let rows = parse_logs(&body(vec![api_request(vec![])]), 5).unwrap();
        assert_eq!(rows.len(), 1);
        let r = &rows[0];
        assert_eq!(r.session_id, "sess-1");
        assert_eq!(r.request_id, "req_1");
        assert_eq!(r.timestamp, 1_760_000_000);
        assert_eq!(r.agent_name, "reviewer");
        assert_eq!(
            (
                r.input_tokens,
                r.output_tokens,
                r.cache_read_tokens,
                r.cache_creation_tokens
            ),
            (10, 20, 30, 40)
        );
        assert_eq!(r.cost_usd, 0.5);
    }

    #[test]
    fn the_skill_name_is_kept_and_an_unsafe_one_is_dropped() {
        let rows = parse_logs(
            &body(vec![api_request(vec![s("skill.name", "tinyskill")])]),
            5,
        )
        .unwrap();
        assert_eq!(rows[0].skill_name, "tinyskill");
        let rows = parse_logs(
            &body(vec![api_request(vec![s("skill.name", "a b/../c d")])]),
            5,
        )
        .unwrap();
        assert_eq!(rows[0].skill_name, "");
        let rows = parse_logs(&body(vec![api_request(vec![])]), 5).unwrap();
        assert_eq!(rows[0].skill_name, "");
    }

    #[test]
    fn the_event_name_may_come_from_the_body_with_the_prefix() {
        let mut rec = api_request(vec![]);
        let attrs = rec["attributes"].as_array_mut().unwrap();
        attrs.retain(|a| a["key"] != "event.name");
        rec["body"] = json!({"stringValue": "claude_code.api_request"});
        assert_eq!(parse_logs(&body(vec![rec]), 5).unwrap().len(), 1);
    }

    #[test]
    fn a_prompt_bearing_attribute_is_dropped_and_never_read() {
        let rec = api_request(vec![
            s("prompt", "write me a secret"),
            s("prompt_text", "hunter2"),
            s("file_path", "/Users/someone/secret.txt"),
            s("user.email", "someone@example.com"),
            s("tool_input", "rm -rf"),
        ]);
        let rows = parse_logs(&body(vec![rec]), 5).unwrap();
        let dump = format!("{rows:?}");
        for leaked in ["secret", "hunter2", "someone", "rm -rf"] {
            assert!(!dump.contains(leaked), "{leaked} leaked into {dump}");
        }
        assert!(!ALLOWED_KEYS
            .iter()
            .any(|k| k.contains("prompt") || k.contains("path")));
    }

    #[test]
    fn an_unsafe_name_is_dropped_not_stored() {
        let mut rec = api_request(vec![]);
        rec["attributes"]
            .as_array_mut()
            .unwrap()
            .retain(|a| a["key"] != "agent.name");
        rec["attributes"]
            .as_array_mut()
            .unwrap()
            .push(s("agent.name", "/Users/me/my project"));
        assert_eq!(parse_logs(&body(vec![rec]), 5).unwrap()[0].agent_name, "");
    }

    #[test]
    fn other_events_and_incomplete_records_are_skipped() {
        let tool = json!({"attributes": [s("event.name", "tool_result"), s("session.id", "s")]});
        let mut no_req = api_request(vec![]);
        no_req["attributes"]
            .as_array_mut()
            .unwrap()
            .retain(|a| a["key"] != "request_id");
        let rows = parse_logs(&body(vec![tool, no_req, api_request(vec![])]), 5).unwrap();
        assert_eq!(rows.len(), 1);
    }

    #[test]
    fn a_missing_time_uses_now_and_bad_numbers_are_zero() {
        let mut rec = api_request(vec![]);
        rec.as_object_mut().unwrap().remove("timeUnixNano");
        let attrs = rec["attributes"].as_array_mut().unwrap();
        attrs.retain(|a| a["key"] != "cost_usd" && a["key"] != "input_tokens");
        attrs.push(attr("cost_usd", json!({"doubleValue": -3.0})));
        attrs.push(s("input_tokens", "lots"));
        let r = &parse_logs(&body(vec![rec]), 77).unwrap()[0];
        assert_eq!((r.timestamp, r.cost_usd, r.input_tokens), (77, 0.0, 0));
    }

    #[test]
    fn more_than_the_record_cap_is_truncated() {
        let records = (0..MAX_RECORDS + 5)
            .map(|i| {
                let mut r = api_request(vec![]);
                r["attributes"]
                    .as_array_mut()
                    .unwrap()
                    .retain(|a| a["key"] != "request_id");
                r["attributes"]
                    .as_array_mut()
                    .unwrap()
                    .push(s("request_id", &format!("r{i}")));
                r
            })
            .collect();
        assert_eq!(parse_logs(&body(records), 5).unwrap().len(), MAX_RECORDS);
    }

    #[test]
    fn invalid_bodies_are_errors() {
        assert!(parse_logs(b"not json", 1).is_err());
        assert!(parse_logs(b"{}", 1).is_err());
        assert!(parse_logs(b"[]", 1).is_err());
    }

    #[test]
    fn a_deeply_nested_body_is_rejected_without_a_crash() {
        let deep = format!("{}{}", "[".repeat(5000), "]".repeat(5000));
        assert!(parse_logs(deep.as_bytes(), 1).is_err());
    }
}
