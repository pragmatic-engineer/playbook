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
use super::plan::Account;
use super::query::{utc_month_bounds, WeekStart};
use super::spend::{BilledComparison, NotCompared, SpendFigure};
use super::UsageEvent;
use crate::util::atomic::write_atomic_private;
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

/// Everything limits are built from: the statusline reading, the manual
/// budgets and what Claude Code keeps in its config file.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct Sources {
    pub snapshot: Option<Snapshot>,
    pub budgets: Vec<(LimitPeriod, f64)>,
    pub account: Account,
}

impl Sources {
    /// Reads the sources. `usage_dir` is the folder of the usage database.
    pub fn load(usage_dir: Option<&Path>, claude_json: &Path, home: &Path) -> Sources {
        Sources::new(usage_dir, claude_json, manual_budgets(home), true)
    }

    /// The sources with the budgets given. The terminal view asks for the
    /// config file to be read through the cache (`cached`), since it reads on
    /// every refresh.
    pub fn new(
        usage_dir: Option<&Path>,
        claude_json: &Path,
        budgets: Vec<(LimitPeriod, f64)>,
        cached: bool,
    ) -> Sources {
        Sources {
            snapshot: usage_dir.and_then(load),
            budgets,
            account: if cached {
                super::plan::read_cached(claude_json)
            } else {
                super::plan::read(claude_json)
            },
        }
    }

    /// The plan limits were read over six hours ago.
    pub fn stale(&self, now: i64) -> bool {
        self.snapshot.as_ref().is_some_and(|s| s.is_stale(now))
    }
}

/// Where the monthly dollar limit comes from.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Monthly {
    /// The statusline reported one (behind a Claude apps gateway). The cache
    /// is not used then, and a manual budget is shown beside it.
    Measured,
    /// The spend limit in the Claude Code cache. `fresh` is false once the
    /// cache is over six hours old.
    Cached {
        limit_usd: f64,
        used_usd: Option<f64>,
        fetched_at: i64,
        age: i64,
        fresh: bool,
    },
    /// `usage.budget.monthlyUsd`.
    Manual(f64),
    Unset,
}

/// The monthly limit to use. A measured one comes first. Then a cached spend
/// limit under six hours old, because it is what Claude Code enforces. Then
/// the manual budget, because it is what the user chose on purpose. An older
/// cache is the last resort.
pub fn monthly(s: &Sources, now: i64) -> Monthly {
    let measured = s.snapshot.as_ref().is_some_and(|sn| {
        sn.limits(now)
            .iter()
            .any(|l| l.period == LimitPeriod::Monthly)
    });
    if measured {
        return Monthly::Measured;
    }
    let manual = s
        .budgets
        .iter()
        .find(|(p, _)| *p == LimitPeriod::Monthly)
        .map(|(_, usd)| *usd);
    let cached = s
        .account
        .cache
        .as_ref()
        .filter(|c| !c.is_from_the_future(now))
        .and_then(|c| {
            c.spend.map(|spend| Monthly::Cached {
                limit_usd: spend.limit_usd,
                used_usd: spend.used_usd,
                fetched_at: c.fetched_at,
                age: c.age(now),
                fresh: c.is_fresh(now),
            })
        });
    match (cached, manual) {
        (Some(c @ Monthly::Cached { fresh: true, .. }), _) => c,
        (_, Some(usd)) => Monthly::Manual(usd),
        (Some(c), None) => c,
        (None, None) => Monthly::Unset,
    }
}

/// Every known limit at `now`: measured first, then cached, then manual.
/// Empty when nothing is known, which the views show as the "limit unknown"
/// empty state.
pub fn all_limits(s: &Sources, events: &[UsageEvent], now: i64, week: WeekStart) -> Vec<Limit> {
    let mut out = s
        .snapshot
        .as_ref()
        .map(|s| s.limits(now))
        .unwrap_or_default();
    // A cached window fills in a window the statusline has not reported, or
    // replaces one the statusline read before the cache was fetched. Only a
    // fresh cache is used: a percentage changes within minutes.
    if let Some(c) = s.account.cache.as_ref().filter(|c| c.is_fresh(now)) {
        let newer = s
            .snapshot
            .as_ref()
            .is_none_or(|sn| c.fetched_at > sn.captured_at);
        for (period, w) in [
            (LimitPeriod::FiveHour, &c.five_hour),
            (LimitPeriod::SevenDay, &c.seven_day),
        ] {
            let Some(w) = w.as_ref().filter(|w| w.resets_at > now) else {
                continue;
            };
            let known = out.iter().any(|l| l.period == period);
            if known && !newer {
                continue;
            }
            out.retain(|l| l.period != period);
            let (start, end) = period.ending_at(w.resets_at);
            out.push(Limit {
                unit: Unit::Percent,
                period,
                origin: Origin::Cached,
                used: w.used_percentage,
                limit: 100.0,
                start,
                end,
            });
        }
    }
    let choice = monthly(s, now);
    let cached_wins = matches!(choice, Monthly::Cached { .. });
    for (period, usd) in &s.budgets {
        if *period == LimitPeriod::Monthly && cached_wins {
            continue;
        }
        out.push(Limit::manual_budget(*period, *usd, events, now, week));
    }
    if let Monthly::Cached {
        limit_usd,
        used_usd,
        fetched_at,
        fresh,
        ..
    } = choice
    {
        let mut l = Limit::manual_budget(LimitPeriod::Monthly, limit_usd, events, now, week);
        l.origin = Origin::Cached;
        // A fresh cache knows the spend of every machine on the account, so
        // it replaces this machine's estimate, unless it was fetched before
        // this month began.
        let (month_start, month_end) = utc_month_bounds(now);
        if let Some(used) = used_usd.filter(|_| fresh && fetched_at >= month_start) {
            // The figure counts the UTC month, so the limit's window is that
            // month too, not the local one.
            l.start = month_start;
            l.end = month_end;
            l.used = used;
        }
        out.push(l);
    }
    out
}

/// The amount Claude Code reports as used next to this machine's estimate for
/// the UTC month that holds `now`. The figure is the statusline gateway
/// `spend_limit` when there is one, else the `spend` block of the usage cache.
/// `None` when neither carries an amount used this month, or the cache is dated
/// in the future. The inner error says why the two were not compared, so a
/// figure in another currency or without one is reported and never dropped.
pub fn compare_billed(
    s: &Sources,
    events: &[UsageEvent],
    now: i64,
) -> Option<Result<BilledComparison, NotCompared>> {
    let month_start = utc_month_bounds(now).0;
    let usable = |f: &SpendFigure| f.used.is_some() && f.fetched_at >= month_start;
    // A gateway figure counts its own period, so only a monthly one is
    // comparable with the local spend of the month.
    let statusline = s
        .snapshot
        .as_ref()
        .and_then(|sn| {
            let sp = sn
                .spend_limit
                .as_ref()
                .filter(|sp| sp.resets_at > now && sp.period == Some(LimitPeriod::Monthly))?;
            SpendFigure::from_usd(sp.limit_usd, sp.used_usd, sn.captured_at)
        })
        .filter(usable);
    let figure = statusline.or_else(|| {
        s.account
            .cache
            .as_ref()
            .filter(|c| !c.is_from_the_future(now))
            .and_then(|c| c.billed.clone())
            .filter(usable)
    })?;
    // Claude reported the amount at `fetched_at`, so only the local spend up
    // to then is comparable.
    let until = now.min(figure.fetched_at);
    let local = events
        .iter()
        .filter(|e| e.timestamp >= month_start && e.timestamp <= until)
        .map(|e| e.cost_usd)
        .sum::<f64>()
        + 0.0;
    Some(BilledComparison::new(figure, local))
}

/// The line for the 5 hour and weekly windows when none is known. A plan
/// known to have them asks for a reading. Any other plan, including one this
/// version cannot tell apart from Enterprise, is told they are not available
/// and where Pro and Max plans get them. `None` when one is known.
pub fn windows_note(s: &Sources, limits: &[Limit]) -> Option<&'static str> {
    if limits
        .iter()
        .any(|l| matches!(l.period, LimitPeriod::FiveHour | LimitPeriod::SevenDay))
    {
        return None;
    }
    Some(if s.account.plan.has_windows() {
        "5 hour and weekly windows: no recent reading, they appear after a session with the playbook statusline"
    } else {
        "5 hour and weekly windows: not available for this plan (Pro and Max get them from the playbook statusline)"
    })
}

/// The plan label, for views that name it.
pub fn plan_label(s: &Sources) -> &'static str {
    s.account.plan.label()
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
pub fn report_json(s: &Sources, events: &[UsageEvent], now: i64, week: WeekStart) -> Value {
    let snapshot = s.snapshot.as_ref();
    let limits = all_limits(s, events, now, week);
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
        "plan": s.account.plan.key(),
        "plan_label": plan_label(s),
        "windows_note": windows_note(s, &limits),
        "cache": s.account.cache.as_ref().map(|c| json!({
            "fetched_at": c.fetched_at,
            "age_seconds": c.age(now),
            "fresh": c.is_fresh(now),
        })),
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
        Origin::Cached => "cached by Claude Code",
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
            "will exceed at about {}",
            crate::tz::iso(at).replace('T', " ")
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
pub fn summary_section(s: &Sources, events: &[UsageEvent], now: i64, week: WeekStart) -> String {
    let limits = all_limits(s, events, now, week);
    let mut out = format!("\nLimits and burn rate ({})\n", crate::tz::label());
    if limits.is_empty() {
        out.push_str(&format!("  {UNKNOWN_HINT}\n"));
    }
    for l in &limits {
        out.push_str(&format!("  {}\n", describe(l, now)));
    }
    if s.stale(now) {
        out.push_str("  note: the plan limits were read over 6 hours ago\n");
    }
    if let Some(c) = s
        .account
        .cache
        .as_ref()
        .filter(|_| limits.iter().any(|l| l.origin == Origin::Cached))
    {
        out.push_str(&format!(
            "  note: the cached limits were fetched by Claude Code {} ago\n",
            super::period::elapsed_text(c.age(now))
        ));
    }
    if let Some(note) = windows_note(s, &limits) {
        out.push_str(&format!("  {note}\n"));
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
        let l = all_limits(&Sources::default(), &[], NOW, WeekStart::Monday);
        assert!(l.is_empty());
    }

    #[test]
    fn manual_budgets_follow_the_snapshot() {
        let s = Sources {
            budgets: vec![(LimitPeriod::Monthly, 100.0), (LimitPeriod::Weekly, 30.0)],
            ..Sources::default()
        };

        let l = all_limits(&s, &[], NOW, WeekStart::Monday);

        assert_eq!(l.len(), 2);
        assert!(l.iter().all(|x| x.origin == Origin::Manual));
    }

    fn cache(age: i64, spend: Option<SpendLimit>, five_hour: Option<Window>) -> Account {
        cache_at(NOW - age, spend, five_hour, None)
    }

    fn cache_at(
        fetched_at: i64,
        spend: Option<SpendLimit>,
        five_hour: Option<Window>,
        seven_day: Option<Window>,
    ) -> Account {
        Account {
            plan: crate::plan::Plan::EnterpriseOrUnknown,
            cache: Some(crate::plan::UsageCache {
                fetched_at,
                five_hour,
                seven_day,
                spend,
                billed: None,
            }),
        }
    }

    fn window_at(used_percentage: f64, resets_at: i64) -> Window {
        Window {
            used_percentage,
            resets_at,
        }
    }

    fn cost(timestamp: i64, cost_usd: f64) -> UsageEvent {
        UsageEvent {
            timestamp,
            cost_usd,
            ..UsageEvent::default()
        }
    }

    fn gateway_snapshot() -> Snapshot {
        Snapshot {
            captured_at: NOW - 60,
            spend_limit: Some(Spend {
                used_percentage: 10.0,
                resets_at: NOW + 86_400,
                used_usd: Some(10.0),
                limit_usd: Some(100.0),
                period: Some(LimitPeriod::Monthly),
            }),
            ..Snapshot::default()
        }
    }

    use crate::plan::SpendLimit;

    fn spend(limit_usd: f64, used_usd: Option<f64>) -> Option<SpendLimit> {
        Some(SpendLimit {
            limit_usd,
            used_usd,
        })
    }

    #[test]
    fn a_fresh_cached_spend_limit_wins_over_the_manual_monthly_budget() {
        let s = Sources {
            budgets: vec![(LimitPeriod::Monthly, 100.0), (LimitPeriod::Weekly, 30.0)],
            account: cache(600, spend(500.0, Some(120.0)), None),
            ..Sources::default()
        };

        let l = all_limits(&s, &[], NOW, WeekStart::Monday);

        let monthly: Vec<_> = l
            .iter()
            .filter(|x| x.period == LimitPeriod::Monthly)
            .collect();
        assert_eq!(monthly.len(), 1);
        assert_eq!(monthly[0].origin, Origin::Cached);
        assert_eq!(monthly[0].limit, 500.0);
        assert_eq!(monthly[0].used, 120.0);
        assert!(l
            .iter()
            .any(|x| x.period == LimitPeriod::Weekly && x.origin == Origin::Manual));
    }

    #[test]
    fn a_stale_cached_spend_limit_loses_to_the_manual_budget() {
        let s = Sources {
            budgets: vec![(LimitPeriod::Monthly, 100.0)],
            account: cache(48 * 3600, spend(500.0, Some(120.0)), None),
            ..Sources::default()
        };

        let l = all_limits(&s, &[], NOW, WeekStart::Monday);

        assert_eq!(l.len(), 1);
        assert_eq!(l[0].origin, Origin::Manual);
        assert_eq!(l[0].limit, 100.0);
    }

    #[test]
    fn a_stale_cached_spend_limit_is_used_when_no_budget_is_set_and_its_spend_is_local() {
        let s = Sources {
            account: cache(48 * 3600, spend(500.0, Some(120.0)), None),
            ..Sources::default()
        };

        let l = all_limits(&s, &[], NOW, WeekStart::Monday);

        assert_eq!(l.len(), 1);
        assert_eq!(l[0].origin, Origin::Cached);
        assert_eq!(l[0].limit, 500.0);
        assert_eq!(l[0].used, 0.0);
    }

    #[test]
    fn a_fresh_cached_window_fills_in_but_never_replaces_a_statusline_window() {
        let cached = Window {
            used_percentage: 70.0,
            resets_at: NOW + 3600,
        };
        let mut s = Sources {
            account: cache(600, None, Some(cached)),
            ..Sources::default()
        };

        let filled = all_limits(&s, &[], NOW, WeekStart::Monday);
        s.snapshot = Some(Snapshot {
            captured_at: NOW,
            five_hour: Some(Window {
                used_percentage: 10.0,
                resets_at: NOW + 7200,
            }),
            ..Snapshot::default()
        });
        let kept = all_limits(&s, &[], NOW, WeekStart::Monday);

        assert_eq!(
            (filled.len(), filled[0].origin, filled[0].used),
            (1, Origin::Cached, 70.0)
        );
        assert_eq!(
            (kept.len(), kept[0].origin, kept[0].used),
            (1, Origin::Measured, 10.0)
        );
    }

    #[test]
    fn a_cached_window_is_ignored_once_the_cache_is_stale() {
        let cached = Window {
            used_percentage: 70.0,
            resets_at: NOW + 3600,
        };
        let s = Sources {
            account: cache(7 * 3600, None, Some(cached)),
            ..Sources::default()
        };

        let l = all_limits(&s, &[], NOW, WeekStart::Monday);

        assert!(l.is_empty());
    }

    #[test]
    fn the_windows_note_says_not_available_for_a_plan_without_windows() {
        let s = Sources::default();

        let note = windows_note(&s, &[]);

        assert_eq!(
            note,
            Some("5 hour and weekly windows: not available for this plan (Pro and Max get them from the playbook statusline)")
        );
    }

    #[test]
    fn the_windows_note_asks_for_a_reading_on_a_max_plan() {
        let s = Sources {
            account: Account {
                plan: crate::plan::Plan::Max5x,
                cache: None,
            },
            ..Sources::default()
        };

        let note = windows_note(&s, &[]).unwrap();

        assert!(note.contains("no recent reading"), "{note}");
    }

    #[test]
    fn there_is_no_windows_note_when_a_window_is_known() {
        let snap = Snapshot::from_statusline(
            &input(&format!(
                r#""five_hour":{{"used_percentage":1.0,"resets_at":{}}}"#,
                NOW + 3600
            )),
            NOW,
        );
        let s = Sources {
            snapshot: snap,
            ..Sources::default()
        };
        let limits = all_limits(&s, &[], NOW, WeekStart::Monday);

        assert_eq!(windows_note(&s, &limits), None);
    }

    #[test]
    fn the_json_names_the_plan_and_the_cache_age() {
        let s = Sources {
            account: cache(600, spend(500.0, None), None),
            ..Sources::default()
        };

        let v = report_json(&s, &[], NOW, WeekStart::Monday);

        assert_eq!(v["plan"], "enterprise_or_unknown");
        assert_eq!(v["plan_label"], "Enterprise or unknown plan");
        assert_eq!(v["cache"]["age_seconds"], 600);
        assert_eq!(v["limits"][0]["origin"], "cached");
        assert!(v["windows_note"]
            .as_str()
            .unwrap()
            .contains("not available"));
    }

    #[test]
    fn a_measured_monthly_limit_hides_the_cache_and_keeps_the_manual_budget() {
        let s = Sources {
            snapshot: Some(gateway_snapshot()),
            budgets: vec![(LimitPeriod::Monthly, 100.0)],
            account: cache(600, spend(500.0, Some(120.0)), None),
        };

        let l = all_limits(&s, &[], NOW, WeekStart::Monday);

        let origins: Vec<_> = l.iter().map(|x| x.origin).collect();
        assert_eq!(origins, vec![Origin::Measured, Origin::Manual]);
        assert_eq!(monthly(&s, NOW), Monthly::Measured);
    }

    #[test]
    fn a_fresh_cached_used_amount_replaces_the_local_estimate() {
        let s = Sources {
            account: cache(600, spend(500.0, Some(10.0)), None),
            ..Sources::default()
        };
        let events = [cost(NOW - 3600, 50.0)];

        let l = all_limits(&s, &events, NOW, WeekStart::Monday);

        assert_eq!(l[0].used, 10.0);
    }

    #[test]
    fn a_fresh_cache_without_a_used_amount_uses_the_local_estimate() {
        let s = Sources {
            account: cache(600, spend(500.0, None), None),
            ..Sources::default()
        };
        let events = [cost(NOW - 3600, 50.0)];

        let l = all_limits(&s, &events, NOW, WeekStart::Monday);

        assert_eq!(l[0].used, 50.0);
    }

    #[test]
    fn a_cached_used_amount_fetched_before_the_month_began_is_ignored() {
        // 2027-01-01 01:00 UTC, with the cache fetched two hours earlier.
        let now = 1_798_761_600 + 3600;
        let s = Sources {
            account: cache_at(now - 7200, spend(500.0, Some(480.0)), None, None),
            ..Sources::default()
        };

        let l = all_limits(&s, &[], now, WeekStart::Monday);

        assert_eq!(l[0].used, 0.0);
    }

    #[test]
    fn a_cache_fetched_in_the_future_is_not_used_for_the_monthly_limit() {
        let mut s = Sources {
            account: cache(-7200, spend(500.0, Some(1.0)), None),
            ..Sources::default()
        };

        let none = monthly(&s, NOW);
        s.budgets = vec![(LimitPeriod::Monthly, 100.0)];
        let manual = monthly(&s, NOW);

        assert_eq!(none, Monthly::Unset);
        assert_eq!(manual, Monthly::Manual(100.0));
    }

    #[test]
    fn a_cached_weekly_window_is_used_and_an_expired_window_is_skipped() {
        let s = Sources {
            account: cache_at(
                NOW - 60,
                None,
                Some(window_at(30.0, NOW - 1)),
                Some(window_at(60.0, NOW + 86_400)),
            ),
            ..Sources::default()
        };

        let l = all_limits(&s, &[], NOW, WeekStart::Monday);

        assert_eq!(l.len(), 1);
        assert_eq!(
            (l[0].period, l[0].origin, l[0].used),
            (LimitPeriod::SevenDay, Origin::Cached, 60.0)
        );
    }

    #[test]
    fn a_cache_fetched_after_the_statusline_reading_replaces_its_window() {
        let snap = Snapshot {
            captured_at: NOW - 2 * 86_400,
            seven_day: Some(window_at(20.0, NOW + 86_400)),
            ..Snapshot::default()
        };
        let newer = Sources {
            snapshot: Some(snap.clone()),
            account: cache_at(NOW - 60, None, None, Some(window_at(85.0, NOW + 86_400))),
            ..Sources::default()
        };
        let older = Sources {
            snapshot: Some(Snapshot {
                captured_at: NOW,
                ..snap
            }),
            account: cache_at(NOW - 60, None, None, Some(window_at(85.0, NOW + 86_400))),
            ..Sources::default()
        };

        let replaced = all_limits(&newer, &[], NOW, WeekStart::Monday);
        let kept = all_limits(&older, &[], NOW, WeekStart::Monday);

        assert_eq!(
            (replaced.len(), replaced[0].origin, replaced[0].used),
            (1, Origin::Cached, 85.0)
        );
        assert_eq!(
            (kept.len(), kept[0].origin, kept[0].used),
            (1, Origin::Measured, 20.0)
        );
    }

    #[test]
    fn the_summary_names_the_cache_age_and_the_missing_windows() {
        let s = Sources {
            account: cache(600, spend(500.0, Some(120.0)), None),
            ..Sources::default()
        };

        let text = summary_section(&s, &[], NOW, WeekStart::Monday);

        assert!(
            text.contains("monthly budget: $120.00 of $500.00 used (cached by Claude Code)"),
            "{text}"
        );
        assert!(text.contains("fetched by Claude Code 10m ago"), "{text}");
        assert!(text.contains("not available for this plan"), "{text}");
    }

    #[test]
    fn the_summary_has_no_cache_note_when_a_manual_budget_won() {
        let s = Sources {
            budgets: vec![(LimitPeriod::Monthly, 100.0)],
            account: cache(48 * 3600, spend(500.0, Some(120.0)), None),
            ..Sources::default()
        };

        let text = summary_section(&s, &[], NOW, WeekStart::Monday);

        assert!(!text.contains("fetched by Claude Code"), "{text}");
        assert!(text.contains("manual limit"), "{text}");
    }

    #[test]
    fn the_cache_used_amount_is_compared_with_the_local_estimate_of_the_utc_month() {
        let mut account = cache(60, spend(1500.0, Some(1059.5)), None);
        let fetched = NOW - 60;
        let c = account.cache.as_mut().unwrap();
        c.billed = SpendFigure::from_usd(Some(1500.0), Some(1059.5), fetched);
        let s = Sources {
            account,
            ..Sources::default()
        };
        let month_start = utc_month_bounds(NOW).0;
        let events = [
            cost(month_start - 1, 50.0),
            cost(month_start, 3.0),
            cost(fetched, 4.0),
            cost(fetched + 1, 100.0),
        ];

        let c = compare_billed(&s, &events, NOW).unwrap().unwrap();

        assert_eq!(c.billed_usd, 1059.5);
        assert_eq!(c.local_estimate_usd, 7.0);
    }

    #[test]
    fn a_figure_in_another_currency_or_without_one_is_reported_not_dropped() {
        let money = |currency: Option<&str>| {
            Some(crate::spend::Money {
                minor: 105_950,
                decimals: 2,
                currency: currency.map(str::to_string),
            })
        };
        for (currency, text) in [
            (Some("EUR"), "currency EUR, not compared"),
            (None, "currency not reported or not readable, not compared"),
        ] {
            let mut account = cache(60, None, None);
            account.cache.as_mut().unwrap().billed = Some(SpendFigure {
                limit: None,
                used: money(currency),
                fetched_at: NOW - 60,
            });
            let s = Sources {
                account,
                ..Sources::default()
            };

            let r = compare_billed(&s, &[], NOW).unwrap();

            assert_eq!(r.unwrap_err().to_string(), text);
        }
    }

    #[test]
    fn nothing_is_compared_without_an_amount_used_this_utc_month() {
        let fig = |fetched_at, used: Option<f64>| {
            let mut account = cache_at(fetched_at, None, None, None);
            account.cache.as_mut().unwrap().billed =
                SpendFigure::from_usd(Some(100.0), used, fetched_at);
            Sources {
                account,
                ..Sources::default()
            }
        };

        assert!(compare_billed(&Sources::default(), &[], NOW).is_none());
        assert!(compare_billed(&fig(NOW - 60, None), &[], NOW).is_none());
        assert!(compare_billed(&fig(utc_month_bounds(NOW).0 - 1, Some(5.0)), &[], NOW).is_none());
        assert!(compare_billed(&fig(NOW + 3600, Some(5.0)), &[], NOW).is_none());
    }

    #[test]
    fn the_statusline_gateway_amount_is_compared_too() {
        let s = Sources {
            snapshot: Some(gateway_snapshot()),
            ..Sources::default()
        };

        let c = compare_billed(&s, &[cost(NOW - 120, 4.0)], NOW)
            .unwrap()
            .unwrap();

        assert_eq!(c.billed_usd, 10.0);
        assert_eq!(c.local_estimate_usd, 4.0);
    }

    #[test]
    fn the_cached_used_amount_counts_from_the_utc_month_not_the_local_one() {
        let month_start = utc_month_bounds(NOW).0;
        let s = Sources {
            account: cache_at(month_start, spend(500.0, Some(120.0)), None, None),
            ..Sources::default()
        };

        let l = all_limits(&s, &[], month_start + 60, WeekStart::Monday);

        let m: Vec<_> = l
            .iter()
            .filter(|x| x.period == LimitPeriod::Monthly)
            .collect();
        assert_eq!(m.len(), 1);
        assert_eq!(m[0].used, 120.0);
        assert_eq!((m[0].start, m[0].end), utc_month_bounds(NOW));
    }

    #[test]
    fn a_figure_fetched_at_the_utc_month_start_is_compared_and_the_clock_clamps() {
        let at = |fetched_at: i64| {
            let mut account = cache_at(fetched_at, None, None, None);
            account.cache.as_mut().unwrap().billed =
                SpendFigure::from_usd(Some(100.0), Some(5.0), fetched_at);
            Sources {
                account,
                ..Sources::default()
            }
        };
        let start = utc_month_bounds(NOW).0;

        assert!(compare_billed(&at(start), &[], NOW).is_some());

        // Fetched 120 s ahead of this clock, inside the skew allowance: spend
        // after `now` is not counted.
        let c = compare_billed(
            &at(NOW + 120),
            &[cost(NOW - 1, 1.0), cost(NOW, 2.0), cost(NOW + 60, 4.0)],
            NOW,
        )
        .unwrap()
        .unwrap();
        assert_eq!(c.local_estimate_usd, 3.0);
    }

    #[test]
    fn the_statusline_amount_wins_over_the_cache_only_when_monthly_and_usable() {
        let cached = |s: &mut Sources| {
            s.account = cache(60, None, None);
            s.account.cache.as_mut().unwrap().billed =
                SpendFigure::from_usd(Some(500.0), Some(77.0), NOW - 60);
        };
        let mut both = Sources {
            snapshot: Some(gateway_snapshot()),
            ..Sources::default()
        };
        cached(&mut both);
        let billed = |s: &Sources| compare_billed(s, &[], NOW).unwrap().unwrap().billed_usd;

        assert_eq!(billed(&both), 10.0);

        let weekly = both
            .snapshot
            .as_mut()
            .unwrap()
            .spend_limit
            .as_mut()
            .unwrap();
        weekly.period = Some(LimitPeriod::Weekly);
        assert_eq!(billed(&both), 77.0);

        let mut no_used = both.clone();
        let sp = no_used
            .snapshot
            .as_mut()
            .unwrap()
            .spend_limit
            .as_mut()
            .unwrap();
        sp.period = Some(LimitPeriod::Monthly);
        sp.used_usd = None;
        assert_eq!(billed(&no_used), 77.0);

        let mut ended = both.clone();
        let sp = ended
            .snapshot
            .as_mut()
            .unwrap()
            .spend_limit
            .as_mut()
            .unwrap();
        sp.period = Some(LimitPeriod::Monthly);
        sp.resets_at = NOW;
        assert_eq!(billed(&ended), 77.0);

        let mut old = both.clone();
        let snap = old.snapshot.as_mut().unwrap();
        snap.captured_at = utc_month_bounds(NOW).0 - 1;
        snap.spend_limit.as_mut().unwrap().period = Some(LimitPeriod::Monthly);
        assert_eq!(billed(&old), 77.0);
    }
}
