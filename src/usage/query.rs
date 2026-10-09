// SPDX-FileCopyrightText: 2026 Igor Santos
// SPDX-License-Identifier: Apache-2.0

//! The typed query layer both views read: the date range, the totals over it,
//! and the live summary. The web dashboard turns these into JSON (`api.rs`) and
//! the terminal view draws them, so a number is computed in one place only.

use super::aggregate::{civil_from_days, SECONDS_PER_DAY};
use super::db;
use super::{ToolInvocationEvent, UsageEvent};
use rusqlite::Connection;
use std::collections::BTreeMap;

/// The date window both views show. Days are UTC, like every grouping.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Range {
    LastDays(i64),
    CurrentMonth,
    All,
}

impl Range {
    /// The `range` query value. No value means all time, so older callers
    /// keep their behaviour.
    pub fn parse(value: Option<&str>) -> Option<Range> {
        match value {
            None | Some("all") => Some(Range::All),
            Some("30d") => Some(Range::LastDays(30)),
            Some("60d") => Some(Range::LastDays(60)),
            Some("90d") => Some(Range::LastDays(90)),
            Some("month") => Some(Range::CurrentMonth),
            Some(_) => None,
        }
    }

    pub fn key(self) -> &'static str {
        match self {
            Range::LastDays(30) => "30d",
            Range::LastDays(60) => "60d",
            Range::LastDays(_) => "90d",
            Range::CurrentMonth => "month",
            Range::All => "all",
        }
    }

    /// First and one-past-last UTC day, or `None` for all time. "Last N days"
    /// includes today; the current month runs from its 1st to its last day.
    pub fn days(self, now: i64) -> Option<(i64, i64)> {
        let today = now.div_euclid(SECONDS_PER_DAY);
        match self {
            Range::All => None,
            Range::LastDays(n) => Some((today - n + 1, today + 1)),
            Range::CurrentMonth => {
                let first = today - (civil_from_days(today).2 - 1);
                let probe = first + 31;
                Some((first, probe - (civil_from_days(probe).2 - 1)))
            }
        }
    }

    /// Whether an event at `timestamp` falls inside the range at `now`.
    pub fn contains(self, now: i64, timestamp: i64) -> bool {
        in_window(self.days(now), timestamp)
    }
}

pub fn day_of(timestamp: i64) -> i64 {
    timestamp.div_euclid(SECONDS_PER_DAY)
}

pub fn in_window(window: Option<(i64, i64)>, timestamp: i64) -> bool {
    window.is_none_or(|(first, past)| (first..past).contains(&day_of(timestamp)))
}

pub fn total_tokens(e: &UsageEvent) -> u64 {
    e.input_tokens + e.output_tokens + e.cache_creation_tokens + e.cache_read_tokens
}

/// The sums over a set of events, the headline numbers of both views.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Totals {
    pub messages: usize,
    pub input_tokens: u64,
    pub output_tokens: u64,
    pub cache_creation_tokens: u64,
    pub cache_read_tokens: u64,
    pub cost_usd: f64,
}

impl Totals {
    pub fn of(events: &[UsageEvent]) -> Totals {
        // `sum` rather than a fold from 0.0: an empty sum is -0.0 and the
        // dashboard's pinned JSON bytes include it.
        Totals {
            messages: events.len(),
            input_tokens: events.iter().map(|e| e.input_tokens).sum(),
            output_tokens: events.iter().map(|e| e.output_tokens).sum(),
            cache_creation_tokens: events.iter().map(|e| e.cache_creation_tokens).sum(),
            cache_read_tokens: events.iter().map(|e| e.cache_read_tokens).sum(),
            cost_usd: events.iter().map(|e| e.cost_usd).sum(),
        }
    }
}

/// The usage and tool events inside `range` at `now`.
pub fn in_range(
    usage: &[UsageEvent],
    tools: &[ToolInvocationEvent],
    now: i64,
    range: Range,
) -> (Vec<UsageEvent>, Vec<ToolInvocationEvent>) {
    let window = range.days(now);
    (
        usage
            .iter()
            .filter(|e| in_window(window, e.timestamp))
            .cloned()
            .collect(),
        tools
            .iter()
            .filter(|t| in_window(window, t.timestamp))
            .cloned()
            .collect(),
    )
}

/// The events a ranged view is built from. Everything before the first day of
/// the range is dropped downstream, so all time is the only range that needs
/// every row.
pub fn load_window(conn: &Connection, range: Range, now: i64) -> Result<Vec<UsageEvent>, String> {
    match range.days(now) {
        Some((first_day, _)) => {
            db::load_usage_events_since(conn, first_day.saturating_mul(SECONDS_PER_DAY))
        }
        None => db::load_usage_events(conn),
    }
}

/// A session counts as active when its last message is this recent.
pub const ACTIVE_SECONDS: i64 = 15 * 60;
pub const BURN_MINUTES: i64 = 60;
const ACTIVE_LIMIT: usize = 20;
/// Messages in the live feed.
pub const FEED_LIMIT: usize = 20;

/// Start of the first of the 60 minutes the burn chart and "last hour" cover:
/// the current minute and the 59 before it.
pub fn hour_start(now: i64) -> i64 {
    (now.div_euclid(60) - (BURN_MINUTES - 1)) * 60
}

/// The oldest timestamp the live summary needs: the start of today (UTC) or
/// of the last hour, whichever is earlier.
pub fn live_window_start(now: i64) -> i64 {
    hour_start(now).min(now.div_euclid(SECONDS_PER_DAY) * SECONDS_PER_DAY)
}

/// Ids of sessions with a message in the last 15 minutes, newest first.
pub fn active_session_ids(window: &[UsageEvent], now: i64) -> Vec<String> {
    let mut last: BTreeMap<&str, i64> = BTreeMap::new();
    for e in window.iter().filter(|e| e.timestamp > now - ACTIVE_SECONDS) {
        let at = last.entry(e.session_id.as_str()).or_insert(0);
        *at = (*at).max(e.timestamp);
    }
    let mut ids: Vec<(&str, i64)> = last.into_iter().collect();
    ids.sort_by(|a, b| b.1.cmp(&a.1).then_with(|| a.0.cmp(b.0)));
    ids.into_iter()
        .take(ACTIVE_LIMIT)
        .map(|(id, _)| id.to_string())
        .collect()
}

/// Messages, tokens and cost over a slice of events.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Spend {
    pub messages: usize,
    pub tokens: u64,
    pub cost_usd: f64,
    pub unpriced: usize,
}

impl Spend {
    pub fn of(events: &[&UsageEvent]) -> Spend {
        Spend {
            messages: events.len(),
            tokens: events.iter().map(|e| total_tokens(e)).sum(),
            cost_usd: events.iter().map(|e| e.cost_usd).sum(),
            unpriced: events.iter().filter(|e| e.unpriced).count(),
        }
    }
}

/// One active session: its newest message and the spend over the whole session.
#[derive(Debug, Clone, PartialEq)]
pub struct ActiveSession {
    pub id: String,
    pub repo: String,
    pub agent: String,
    pub model: String,
    pub last: i64,
    pub messages: usize,
    pub cost_usd: f64,
    pub unpriced: usize,
}

/// What the live view shows.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct LiveView {
    pub generated_at: i64,
    pub active: Vec<ActiveSession>,
    pub hour: Spend,
    pub today: Spend,
    /// Cost per minute for the last 60 minutes, oldest first, as (`HH:MM`, USD).
    pub burn: Vec<(String, f64)>,
    /// The newest messages, newest first.
    pub feed: Vec<UsageEvent>,
}

/// `HH:MM:SS` of a UTC timestamp.
pub fn clock(timestamp: i64) -> String {
    let s = timestamp.rem_euclid(SECONDS_PER_DAY);
    format!("{:02}:{:02}:{:02}", s / 3600, s % 3600 / 60, s % 60)
}

/// Builds the live view. `window` holds every event since `live_window_start`,
/// `sessions` every event of the active sessions (so "spend so far" is the
/// whole session), `latest` the newest messages, newest first.
pub fn live_view(
    window: &[UsageEvent],
    sessions: &[UsageEvent],
    latest: &[UsageEvent],
    now: i64,
) -> LiveView {
    let today_start = now.div_euclid(SECONDS_PER_DAY) * SECONDS_PER_DAY;
    let hour: Vec<&UsageEvent> = window
        .iter()
        .filter(|e| e.timestamp >= hour_start(now))
        .collect();
    let today: Vec<&UsageEvent> = window
        .iter()
        .filter(|e| e.timestamp >= today_start)
        .collect();

    let first_minute = now.div_euclid(60) - (BURN_MINUTES - 1);
    let mut per_minute = vec![0.0_f64; BURN_MINUTES as usize];
    for e in &hour {
        let slot = e.timestamp.div_euclid(60) - first_minute;
        if (0..BURN_MINUTES).contains(&slot) {
            per_minute[slot as usize] += e.cost_usd;
        }
    }
    let burn = per_minute
        .into_iter()
        .enumerate()
        .map(|(i, cost)| (clock((first_minute + i as i64) * 60)[..5].to_string(), cost))
        .collect();

    let active = active_session_ids(window, now)
        .iter()
        .filter_map(|id| {
            let events: Vec<&UsageEvent> =
                sessions.iter().filter(|e| &e.session_id == id).collect();
            let newest = events.iter().max_by_key(|e| e.timestamp).copied()?;
            Some(ActiveSession {
                id: id.clone(),
                repo: newest.repo.clone(),
                agent: newest.agent.clone(),
                model: newest.model.clone(),
                last: newest.timestamp,
                messages: events.len(),
                cost_usd: events.iter().map(|e| e.cost_usd).sum(),
                unpriced: events.iter().filter(|e| e.unpriced).count(),
            })
        })
        .collect();

    LiveView {
        generated_at: now,
        active,
        hour: Spend::of(&hour),
        today: Spend::of(&today),
        burn,
        feed: latest.iter().take(FEED_LIMIT).cloned().collect(),
    }
}

/// The three event sets `live_view` takes.
#[derive(Debug, Default)]
pub struct LiveInputs {
    pub window: Vec<UsageEvent>,
    pub sessions: Vec<UsageEvent>,
    pub latest: Vec<UsageEvent>,
}

/// The three event sets `live_view` takes, read from the store at `now`: the
/// last hour and today, every event of the active sessions, and the newest
/// messages. Not the whole dataset.
pub fn load_live_inputs(conn: &Connection, now: i64) -> Result<LiveInputs, String> {
    let window = db::load_usage_events_since(conn, live_window_start(now))?;
    let ids = active_session_ids(&window, now);
    let sessions = db::load_usage_events_of_sessions(conn, &ids)?;
    let latest = db::load_latest_usage_events(conn, FEED_LIMIT as i64)?;
    Ok(LiveInputs {
        window,
        sessions,
        latest,
    })
}

pub fn load_live(conn: &Connection, now: i64) -> Result<LiveView, String> {
    let i = load_live_inputs(conn, now)?;
    Ok(live_view(&i.window, &i.sessions, &i.latest, now))
}

#[cfg(test)]
mod tests {
    use super::*;

    const NOW: i64 = 1_788_400_000;

    fn event(ts: i64, session: &str, cost: f64) -> UsageEvent {
        UsageEvent {
            event_id: format!("{session}{ts}"),
            timestamp: ts,
            session_id: session.into(),
            model: "sonnet".into(),
            input_tokens: 10,
            output_tokens: 20,
            cache_creation_tokens: 1,
            cache_read_tokens: 2,
            cost_usd: cost,
            ..UsageEvent::default()
        }
    }

    #[test]
    fn totals_sum_every_counter() {
        let events = vec![event(NOW, "a", 0.25), event(NOW - 5, "b", 0.5)];
        let t = Totals::of(&events);
        assert_eq!((t.messages, t.input_tokens, t.output_tokens), (2, 20, 40));
        assert_eq!((t.cache_creation_tokens, t.cache_read_tokens), (2, 4));
        assert!((t.cost_usd - 0.75).abs() < 1e-9);
    }

    #[test]
    fn range_parse_and_key_round_trip() {
        for key in ["30d", "60d", "90d", "month", "all"] {
            assert_eq!(Range::parse(Some(key)).map(Range::key), Some(key));
        }
        assert_eq!(Range::parse(None), Some(Range::All));
        assert_eq!(Range::parse(Some("7d")), None);
    }

    #[test]
    fn range_contains_follows_utc_days() {
        let day = NOW.div_euclid(SECONDS_PER_DAY) * SECONDS_PER_DAY;
        let r = Range::LastDays(30);
        assert!(r.contains(NOW, day));
        assert!(!r.contains(NOW, day - 30 * SECONDS_PER_DAY));
        assert!(Range::All.contains(NOW, 0));
    }

    #[test]
    fn in_range_filters_both_event_kinds() {
        let old = NOW - 100 * SECONDS_PER_DAY;
        let usage = vec![event(NOW - 60, "a", 1.0), event(old, "a", 2.0)];
        let (inside, _) = in_range(&usage, &[], NOW, Range::LastDays(30));
        assert_eq!(inside.len(), 1);
        assert_eq!(Totals::of(&inside).cost_usd, 1.0);
    }

    #[test]
    fn live_view_reports_active_sessions_and_burn() {
        let recent = event(NOW - 60, "live", 0.5);
        let stale = event(NOW - 3 * 3600, "idle", 9.0);
        let window = vec![stale.clone(), recent.clone()];
        let sessions = vec![recent.clone()];
        let v = live_view(&window, &sessions, &[recent], NOW);
        assert_eq!(v.active.len(), 1);
        assert_eq!(v.active[0].id, "live");
        assert_eq!(v.hour.messages, 1);
        assert_eq!(v.burn.len(), BURN_MINUTES as usize);
        assert!((v.burn.iter().map(|b| b.1).sum::<f64>() - 0.5).abs() < 1e-9);
        assert_eq!(v.feed.len(), 1);
    }
}
