// SPDX-FileCopyrightText: 2026 Igor Santos
// SPDX-License-Identifier: Apache-2.0

//! Where limits come from. Measured limits come from the `rate_limits` object
//! Claude Code passes to the statusline command on stdin (documented at
//! code.claude.com/docs/en/statusline). The statusline keeps the latest copy
//! in a small private file next to the usage database, and the usage views read
//! it back. Only percentages, reset times and, behind a Claude apps gateway,
//! the dollar spend limit are kept: no account, no token, no session id. The
//! rate limit fields exist only for Pro and Max subscribers (or a gateway), so
//! on an API key there is nothing to read and the file is never written.
//!
//! Manual limits come from the `usage.budget.*` config keys.

use super::limits::{Burn, Limit, LimitPeriod, Origin, State, Unit};
use super::period::Period;
use super::query::WeekStart;
use super::UsageEvent;
use crate::common::atomic::write_atomic_private;
use serde_json::{json, Value};
use std::path::{Path, PathBuf};

/// A snapshot older than this is shown as stale rather than hidden.
pub const STALE_SECONDS: i64 = 6 * 3600;

/// One usage window, as Claude Code reports it.
#[derive(Debug, Clone, PartialEq)]
pub struct Window {
    pub used_percentage: f64,
    pub resets_at: i64,
}

/// The gateway spend limit. The dollar fields are optional in Claude Code.
#[derive(Debug, Clone, PartialEq)]
pub struct Spend {
    pub used_percentage: f64,
    pub resets_at: i64,
    pub used_usd: Option<f64>,
    pub limit_usd: Option<f64>,
    pub period: Option<LimitPeriod>,
}

/// The latest `rate_limits` seen by the statusline.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct Snapshot {
    pub captured_at: i64,
    pub five_hour: Option<Window>,
    pub seven_day: Option<Window>,
    pub spend_limit: Option<Spend>,
}

fn window(v: Option<&Value>) -> Option<Window> {
    let v = v?;
    let pct = v.get("used_percentage")?.as_f64()?;
    let at = v.get("resets_at")?.as_i64()?;
    (pct.is_finite() && pct >= 0.0).then_some(Window {
        used_percentage: pct,
        resets_at: at,
    })
}

fn spend(v: Option<&Value>) -> Option<Spend> {
    let w = window(v)?;
    let v = v?;
    let usd = |k: &str| {
        v.get(k)
            .and_then(Value::as_f64)
            .filter(|n| n.is_finite() && *n >= 0.0)
    };
    Some(Spend {
        used_percentage: w.used_percentage,
        resets_at: w.resets_at,
        used_usd: usd("used_usd"),
        limit_usd: usd("limit_usd"),
        period: v
            .get("period")
            .and_then(Value::as_str)
            .and_then(LimitPeriod::parse),
    })
}

impl Snapshot {
    /// Reads `rate_limits` from the statusline input. `None` when the input is
    /// not JSON or has no usable window (API key accounts, or before the first
    /// response of a session).
    pub fn from_statusline(input: &str, now: i64) -> Option<Snapshot> {
        let root: Value = serde_json::from_str(input).ok()?;
        let r = root.get("rate_limits")?;
        let snap = Snapshot {
            captured_at: now,
            five_hour: window(r.get("five_hour")),
            seven_day: window(r.get("seven_day")),
            spend_limit: spend(r.get("spend_limit")),
        };
        (snap.five_hour.is_some() || snap.seven_day.is_some() || snap.spend_limit.is_some())
            .then_some(snap)
    }

    fn to_value(&self) -> Value {
        let w = |w: &Option<Window>| {
            w.as_ref()
                .map(|w| json!({"used_percentage": w.used_percentage, "resets_at": w.resets_at}))
        };
        json!({
            "captured_at": self.captured_at,
            "five_hour": w(&self.five_hour),
            "seven_day": w(&self.seven_day),
            "spend_limit": self.spend_limit.as_ref().map(|s| json!({
                "used_percentage": s.used_percentage,
                "resets_at": s.resets_at,
                "used_usd": s.used_usd,
                "limit_usd": s.limit_usd,
                "period": s.period.map(LimitPeriod::key),
            })),
        })
    }

    fn from_value(v: &Value) -> Snapshot {
        Snapshot {
            captured_at: v.get("captured_at").and_then(Value::as_i64).unwrap_or(0),
            five_hour: window(v.get("five_hour")),
            seven_day: window(v.get("seven_day")),
            spend_limit: spend(v.get("spend_limit")),
        }
    }

    pub fn is_stale(&self, now: i64) -> bool {
        now - self.captured_at > STALE_SECONDS
    }

    /// The measured limits that still apply at `now`. A window whose reset time
    /// has passed is dropped: Claude Code does the same, and its percentage no
    /// longer describes the current window.
    pub fn limits(&self, now: i64) -> Vec<Limit> {
        let mut out = Vec::new();
        let pct = |w: &Window, period: LimitPeriod| {
            let (start, end) = period.ending_at(w.resets_at);
            Limit {
                unit: Unit::Percent,
                period,
                origin: Origin::Measured,
                used: w.used_percentage,
                limit: 100.0,
                start,
                end,
            }
        };
        if let Some(w) = self.five_hour.as_ref().filter(|w| w.resets_at > now) {
            out.push(pct(w, LimitPeriod::FiveHour));
        }
        if let Some(w) = self.seven_day.as_ref().filter(|w| w.resets_at > now) {
            out.push(pct(w, LimitPeriod::SevenDay));
        }
        if let Some(s) = self.spend_limit.as_ref().filter(|s| s.resets_at > now) {
            match (s.used_usd, s.limit_usd, s.period) {
                (Some(used), Some(limit), Some(period)) => {
                    let (start, end) = period.ending_at(s.resets_at);
                    out.push(Limit {
                        unit: Unit::Dollars,
                        period,
                        origin: Origin::Measured,
                        used,
                        limit,
                        start,
                        end,
                    });
                }
                // The dollar fields come on a slower schedule and can be
                // absent, so fall back to the percent that always comes.
                _ => {
                    let w = Window {
                        used_percentage: s.used_percentage,
                        resets_at: s.resets_at,
                    };
                    out.push(pct(&w, s.period.unwrap_or(LimitPeriod::Monthly)));
                }
            }
        }
        out
    }
}

/// `rate_limits.json` in the usage folder.
pub fn snapshot_path(usage_dir: &Path) -> PathBuf {
    usage_dir.join("rate_limits.json")
}

/// Stores the snapshot. Best effort: the statusline must never fail on this.
pub fn save(usage_dir: &Path, snap: &Snapshot) {
    if std::fs::create_dir_all(usage_dir).is_err() {
        return;
    }
    let _ = write_atomic_private(&snapshot_path(usage_dir), snap.to_value().to_string());
}

/// The stored snapshot, or `None` when there is none or it is unreadable.
pub fn load(usage_dir: &Path) -> Option<Snapshot> {
    let text = std::fs::read_to_string(snapshot_path(usage_dir)).ok()?;
    let v: Value = serde_json::from_str(&text).ok()?;
    Some(Snapshot::from_value(&v))
}

/// The manual budgets from the config: `(period, usd)` for each key that is
/// set above zero.
pub fn manual_budgets(home: &Path) -> Vec<(LimitPeriod, f64)> {
    [
        ("usage.budget.monthlyUsd", LimitPeriod::Monthly),
        ("usage.budget.weeklyUsd", LimitPeriod::Weekly),
    ]
    .into_iter()
    .filter_map(|(key, period)| {
        let value = super::host::config_value(key, home)?;
        let usd = value.as_f64().filter(|n| n.is_finite() && *n > 0.0)?;
        Some((period, usd))
    })
    .collect()
}

/// Every known limit at `now`: measured first, then manual. Empty when nothing
/// is known, which the views show as the "limit unknown" empty state.
pub fn all_limits(
    snapshot: Option<&Snapshot>,
    budgets: &[(LimitPeriod, f64)],
    events: &[UsageEvent],
    now: i64,
    week: WeekStart,
) -> Vec<Limit> {
    let mut out = snapshot.map(|s| s.limits(now)).unwrap_or_default();
    for (period, usd) in budgets {
        out.push(Limit::manual_budget(*period, *usd, events, now, week));
    }
    out
}

/// The empty state shown where a limit would be.
pub const UNKNOWN_HINT: &str = "limit unknown: set usage.budget.monthlyUsd or usage.budget.weeklyUsd, or sign in with a Pro or Max plan and use the playbook statusline to read the plan limits";

/// The oldest event the limits report needs: the start of the previous month,
/// week and day, whichever is earliest.
pub fn events_since(now: i64, week: WeekStart) -> i64 {
    [Period::Today, Period::Week(week), Period::Month]
        .into_iter()
        .map(|p| p.windows(now).previous.start)
        .min()
        .unwrap_or(0)
}

/// Cost so far this period and in the previous one over the same elapsed span.
pub fn burn_pair(events: &[UsageEvent], period: Period, now: i64) -> (Burn, Burn) {
    let w = period.windows(now);
    let (current, previous) = w.split(events);
    // `+ 0.0` turns the -0.0 of an empty sum into 0.0.
    let cost = |e: &[UsageEvent]| e.iter().map(|e| e.cost_usd).sum::<f64>() + 0.0;
    (
        Burn::new(cost(&current), w.current.seconds()),
        Burn::new(cost(&previous), w.previous.seconds()),
    )
}

/// The limits and burn rates as JSON, for `--json`.
pub fn report_json(
    snapshot: Option<&Snapshot>,
    budgets: &[(LimitPeriod, f64)],
    events: &[UsageEvent],
    now: i64,
    week: WeekStart,
) -> Value {
    let limits = all_limits(snapshot, budgets, events, now, week);
    let burn: serde_json::Map<String, Value> = [Period::Today, Period::Week(week), Period::Month]
        .into_iter()
        .map(|p| {
            let (current, previous) = burn_pair(events, p, now);
            (
                p.key().to_string(),
                json!({"current": current.to_json(), "previous": previous.to_json()}),
            )
        })
        .collect();
    json!({
        "known": !limits.is_empty(),
        "hint": limits.is_empty().then_some(UNKNOWN_HINT),
        "snapshot_captured_at": snapshot.map(|s| s.captured_at),
        "snapshot_stale": snapshot.map(|s| s.is_stale(now)),
        "limits": limits.iter().map(|l| l.to_json(now)).collect::<Vec<_>>(),
        "burn": burn,
    })
}

/// A dollar or percent amount with its unit.
pub fn amount(unit: Unit, n: f64) -> String {
    match unit {
        Unit::Dollars => format!("${n:.2}"),
        Unit::Percent => format!("{n:.1}%"),
    }
}

/// One limit as a plain sentence for `--summary`.
pub fn describe(l: &Limit, now: i64) -> String {
    let source = match l.origin {
        Origin::Measured => "measured from Claude Code",
        Origin::Manual => "manual limit",
    };
    let head = format!(
        "{} {}: {} of {} used ({source})",
        l.period.key().replace('_', " "),
        match l.unit {
            Unit::Dollars => "budget",
            Unit::Percent => "window",
        },
        amount(l.unit, l.used),
        amount(l.unit, l.limit),
    );
    let Some(h) = l.headroom(now) else {
        return head;
    };
    let per = |n: Option<f64>| n.map_or("n/a".to_string(), |n| amount(l.unit, n));
    let state = match h.state {
        State::Exceeded => "limit reached".to_string(),
        State::Unknown => "too early to project".to_string(),
        State::Under => "under".to_string(),
        State::OnTrack => "on track".to_string(),
        State::WillExceed { at } => format!(
            "will exceed at about {} UTC",
            crate::common::time::iso_utc(at)
                .replace('T', " ")
                .trim_end_matches('Z')
        ),
    };
    format!(
        "{head}; {} left, avg {}/h {}/day, allowed {}/h {}/day, projected {}, {state}",
        amount(l.unit, h.remaining),
        per(h.avg_per_hour),
        per(h.avg_per_day),
        per(h.allowed_per_hour),
        per(h.allowed_per_day),
        per(h.projected),
    )
}

/// The limits section of `--summary`.
pub fn summary_section(
    snapshot: Option<&Snapshot>,
    budgets: &[(LimitPeriod, f64)],
    events: &[UsageEvent],
    now: i64,
    week: WeekStart,
) -> String {
    let limits = all_limits(snapshot, budgets, events, now, week);
    let mut out = String::from("\nLimits and burn rate (UTC)\n");
    if limits.is_empty() {
        out.push_str(&format!("  {UNKNOWN_HINT}\n"));
    }
    for l in &limits {
        out.push_str(&format!("  {}\n", describe(l, now)));
    }
    if snapshot.is_some_and(|s| s.is_stale(now)) {
        out.push_str("  note: the plan limits were read over 6 hours ago\n");
    }
    for p in [Period::Today, Period::Week(week), Period::Month] {
        let (c, v) = burn_pair(events, p, now);
        let rate = |b: &Burn| match (b.per_hour, b.per_day) {
            (Some(h), Some(d)) => format!("${h:.2}/h ${d:.2}/day"),
            _ => "n/a".to_string(),
        };
        out.push_str(&format!(
            "  {}: {} so far, {}; {} {}\n",
            p.label(),
            amount(Unit::Dollars, c.cost_usd),
            rate(&c),
            p.previous_label(),
            rate(&v)
        ));
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    const NOW: i64 = 1_800_000_000;

    fn input(extra: &str) -> String {
        format!(r#"{{"model":{{"id":"m"}},"rate_limits":{{{extra}}}}}"#)
    }

    #[test]
    fn reads_both_subscription_windows() {
        let s = Snapshot::from_statusline(
            &input(&format!(
                r#""five_hour":{{"used_percentage":23.5,"resets_at":{}}},
                   "seven_day":{{"used_percentage":41.2,"resets_at":{}}}"#,
                NOW + 3600,
                NOW + 86_400
            )),
            NOW,
        )
        .unwrap();
        let limits = s.limits(NOW);
        assert_eq!(limits.len(), 2);
        assert_eq!(limits[0].period, LimitPeriod::FiveHour);
        assert_eq!(limits[0].used, 23.5);
        assert_eq!(limits[0].origin, Origin::Measured);
        assert_eq!(limits[0].end, NOW + 3600);
        assert_eq!(limits[1].period, LimitPeriod::SevenDay);
    }

    #[test]
    fn one_missing_window_is_fine() {
        let s = Snapshot::from_statusline(
            &input(&format!(
                r#""seven_day":{{"used_percentage":1,"resets_at":{}}}"#,
                NOW + 5
            )),
            NOW,
        )
        .unwrap();
        assert!(s.five_hour.is_none());
        assert_eq!(s.limits(NOW).len(), 1);
    }

    #[test]
    fn api_key_input_has_nothing() {
        assert!(Snapshot::from_statusline(r#"{"cost":{"total_cost_usd":1}}"#, NOW).is_none());
        assert!(Snapshot::from_statusline("not json", NOW).is_none());
        assert!(Snapshot::from_statusline(&input(""), NOW).is_none());
        assert!(Snapshot::from_statusline(
            &input(r#""five_hour":{"used_percentage":"x","resets_at":1}"#),
            NOW
        )
        .is_none());
    }

    #[test]
    fn an_expired_window_is_dropped() {
        let s = Snapshot::from_statusline(
            &input(&format!(
                r#""five_hour":{{"used_percentage":90,"resets_at":{}}}"#,
                NOW - 1
            )),
            NOW - 100,
        )
        .unwrap();
        assert!(s.limits(NOW).is_empty());
    }

    #[test]
    fn a_gateway_spend_limit_reads_dollars_when_present() {
        let s = Snapshot::from_statusline(
            &input(&format!(
                r#""spend_limit":{{"used_percentage":62.8,"resets_at":{},"used_usd":314.12,"limit_usd":500,"period":"monthly"}}"#,
                NOW + 86_400
            )),
            NOW,
        )
        .unwrap();
        let l = &s.limits(NOW)[0];
        assert_eq!((l.unit, l.period), (Unit::Dollars, LimitPeriod::Monthly));
        assert_eq!((l.used, l.limit), (314.12, 500.0));
    }

    #[test]
    fn a_gateway_spend_limit_falls_back_to_percent() {
        let s = Snapshot::from_statusline(
            &input(&format!(
                r#""spend_limit":{{"used_percentage":62.8,"resets_at":{}}}"#,
                NOW + 86_400
            )),
            NOW,
        )
        .unwrap();
        let l = &s.limits(NOW)[0];
        assert_eq!((l.unit, l.used, l.limit), (Unit::Percent, 62.8, 100.0));
    }

    #[test]
    fn the_file_round_trips_and_holds_no_identity() {
        let dir = std::env::temp_dir().join(format!("pb-limits-{}", std::process::id()));
        let s = Snapshot::from_statusline(
            &input(&format!(
                r#""five_hour":{{"used_percentage":5,"resets_at":{}}}"#,
                NOW + 60
            )),
            NOW,
        )
        .unwrap();
        save(&dir, &s);
        assert_eq!(load(&dir), Some(s));
        let text = std::fs::read_to_string(snapshot_path(&dir)).unwrap();
        assert!(!text.contains("model"));
        let _ = std::fs::remove_dir_all(&dir);
        assert_eq!(load(&dir), None);
    }

    #[test]
    fn a_snapshot_goes_stale_after_six_hours() {
        let s = Snapshot {
            captured_at: NOW,
            ..Snapshot::default()
        };
        assert!(!s.is_stale(NOW + STALE_SECONDS));
        assert!(s.is_stale(NOW + STALE_SECONDS + 1));
    }

    #[test]
    fn nothing_known_is_an_empty_list() {
        let l = all_limits(None, &[], &[], NOW, WeekStart::Monday);
        assert!(l.is_empty());
    }

    #[test]
    fn manual_budgets_follow_the_snapshot() {
        let l = all_limits(
            None,
            &[(LimitPeriod::Monthly, 100.0), (LimitPeriod::Weekly, 30.0)],
            &[],
            NOW,
            WeekStart::Monday,
        );
        assert_eq!(l.len(), 2);
        assert!(l.iter().all(|x| x.origin == Origin::Manual));
    }
}
