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

use super::burnup::burnup_for;
use super::limits::{spent, Burn, CrossState, Crossing, Limit, LimitPeriod, Origin, State, Unit};
use super::period::Period;
use super::plan::{Account, Capabilities, DisplayLine, LoginKind, PlanChoice};
use super::query::{month_first_day, WeekStart};
use super::spend::{dollars, Comparison, Money, NotCompared, Reason, Scope, SpendFigure};
use super::UsageEvent;
use crate::util::atomic::write_atomic_private;
use serde_json::{json, Value};
use std::path::{Path, PathBuf};

/// The most days the plain burn-up table prints: a month is at most 31.
const BURNUP_TABLE_ROWS: usize = 31;

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
    /// The figure the server billed, when one was fetched.
    pub server: Option<SpendFigure>,
    /// The `usage.plan` setting.
    pub plan_choice: PlanChoice,
    /// Why no server figure is shown, when a fetch was meant to give one.
    pub spend_unavailable: Option<Reason>,
}

impl Sources {
    /// Reads the sources. `usage_dir` is the folder of the usage database.
    pub fn load(usage_dir: Option<&Path>, claude_json: &Path, home: &Path) -> Sources {
        Sources {
            plan_choice: super::host::config_value("usage.plan", home)
                .and_then(|v| v.as_str().and_then(PlanChoice::parse))
                .unwrap_or_default(),
            ..Sources::new(usage_dir, claude_json, manual_budgets(home), true)
        }
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
            server: None,
            plan_choice: PlanChoice::Auto,
            spend_unavailable: None,
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
    /// The spend limit the server reported. `fresh` is false once the figure
    /// is over six hours old.
    Server {
        limit_usd: f64,
        used_usd: Option<f64>,
        fetched_at: i64,
        age: i64,
        fresh: bool,
    },
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

/// The first second of the UTC month that holds `now`.
pub(crate) fn utc_month_start(now: i64) -> i64 {
    let utc = crate::tz::Zone::Utc;
    utc.day_start(month_first_day(utc.day_of(now)))
}

/// The monthly limit to use. A measured one comes first. Then a server figure
/// under six hours old that is newer than the cache, because it is the amount
/// billed. Then a cached spend limit under six hours old, because it is what
/// Claude Code enforces. Then the manual budget, because it is what the user
/// chose on purpose. An older figure is the last resort, the later fetched of
/// the two. A server figure fetched before this month began describes the
/// last month and is never used.
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
    // The server bills by the UTC month, whatever zone this machine is in.
    let month_start = utc_month_start(now);
    let server = s
        .server
        .as_ref()
        .filter(|f| !f.is_from_the_future(now) && f.fetched_at >= month_start)
        .and_then(|f| {
            f.limit_usd().map(|limit_usd| Monthly::Server {
                limit_usd,
                used_usd: f.used_usd(),
                fetched_at: f.fetched_at,
                age: f.age(now),
                fresh: f.is_fresh(now),
            })
        });
    let fetched = |m: &Monthly| match m {
        Monthly::Server { fetched_at, .. } | Monthly::Cached { fetched_at, .. } => *fetched_at,
        _ => i64::MIN,
    };
    match (server, cached, manual) {
        (Some(sv @ Monthly::Server { fresh: true, .. }), c, _)
            if !matches!(c, Some(Monthly::Cached { fresh: true, .. }))
                || c.is_some_and(|c| fetched(&sv) > fetched(&c)) =>
        {
            sv
        }
        (_, Some(c @ Monthly::Cached { fresh: true, .. }), _) => c,
        (_, _, Some(usd)) => Monthly::Manual(usd),
        (Some(sv), Some(c), None) => {
            if fetched(&sv) > fetched(&c) {
                sv
            } else {
                c
            }
        }
        (Some(only), None, None) | (None, Some(only), None) => only,
        (None, None, None) => Monthly::Unset,
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
    let figure_wins = matches!(choice, Monthly::Cached { .. } | Monthly::Server { .. });
    for (period, usd) in &s.budgets {
        if *period == LimitPeriod::Monthly && figure_wins {
            continue;
        }
        out.push(Limit::manual_budget(*period, *usd, events, now, week));
    }
    let figure = match choice {
        Monthly::Cached {
            limit_usd,
            used_usd,
            fetched_at,
            fresh,
            ..
        } => Some((Origin::Cached, limit_usd, used_usd, fetched_at, fresh)),
        Monthly::Server {
            limit_usd,
            used_usd,
            fetched_at,
            fresh,
            ..
        } => Some((Origin::Server, limit_usd, used_usd, fetched_at, fresh)),
        _ => None,
    };
    if let Some((origin, limit_usd, used_usd, fetched_at, fresh)) = figure {
        let mut l = Limit::manual_budget(LimitPeriod::Monthly, limit_usd, events, now, week);
        l.origin = origin;
        // The server's own reset time is the true end of its period.
        let reset = s.server.as_ref().and_then(|f| f.resets_at);
        if let (Origin::Server, Some(r)) = (origin, reset.filter(|r| *r > now)) {
            (l.start, l.end) = LimitPeriod::Monthly.ending_at(r);
            l.used = spent(events, l.start, now);
        }
        // A fresh figure knows the spend of every machine on the account, so
        // it replaces this machine's estimate, unless it was fetched before
        // this month began.
        if let Some(used) = used_usd.filter(|_| fresh && fetched_at >= l.start) {
            l.used = used;
        }
        out.push(l);
    }
    out
}

/// The server billed amount next to this machine's estimate for the month
/// that holds `now`. `None` when no server figure is used for the monthly
/// limit. The inner error says why the two were not compared.
pub fn compare_server(
    s: &Sources,
    events: &[UsageEvent],
    now: i64,
) -> Option<Result<Comparison, NotCompared>> {
    if !matches!(monthly(s, now), Monthly::Server { .. }) {
        return None;
    }
    let figure = s.server.clone()?;
    // The server billed up to the moment it was asked, so only the local
    // spend up to then is comparable.
    let until = now.min(figure.fetched_at);
    let local = spent(events, utc_month_start(now), until);
    Some(Comparison::new(figure, local))
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
pub fn plan_label(s: &Sources) -> String {
    super::plan::plan_label(s.account.plan, s.plan_choice)
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

/// What the best source for this moment carries, with the `usage.plan`
/// setting applied.
fn capabilities(s: &Sources, now: i64) -> Capabilities {
    let base = if let Some(f) = server_in_use(s, now) {
        Capabilities::from_figure(f)
    } else if let Some(c) = s.account.cache.as_ref().filter(|c| c.is_fresh(now)) {
        Capabilities::from_cache(c)
    } else if let Some(sn) = &s.snapshot {
        Capabilities::from_snapshot(sn)
    } else {
        Capabilities::from_cache(&super::plan::UsageCache::default())
    };
    // The login kind is not read here: a view never reads the environment.
    // The fetch reads it and decides before asking, so this stays `Unknown`.
    base.for_login(LoginKind::Unknown, s.plan_choice)
}

fn capabilities_json(s: &Sources, now: i64) -> Value {
    let c = capabilities(s, now);
    json!({
        "lines": c.lines().iter().map(DisplayLine::key).collect::<Vec<_>>(),
        "scope": c.scope.key(),
        "local_only": c.local_only,
        "note": c.note(s.account.plan, s.plan_choice),
    })
}

fn money_json(m: &Money) -> Value {
    json!({"minor": m.minor, "decimals": m.decimals, "currency": m.currency})
}

/// The server figure in use and its comparison with the local estimate, or
/// `null` when none is used.
fn spend_json(s: &Sources, events: &[UsageEvent], now: i64) -> Value {
    let Some(f) = server_in_use(s, now) else {
        return Value::Null;
    };
    let (comparison, skipped) = match compare_server(s, events, now) {
        Some(Ok(c)) => (
            json!({
                "billed_usd": c.billed_usd,
                "local_estimate_usd": c.local_estimate_usd,
                "delta_usd": c.delta_usd,
                "delta_pct": c.delta_pct,
                "disagrees": c.disagrees,
            }),
            Value::Null,
        ),
        Some(Err(why)) => (Value::Null, json!(why.to_string())),
        None => (Value::Null, Value::Null),
    };
    json!({
        "fetched_at": f.fetched_at,
        "age_seconds": f.age(now),
        "fresh": f.is_fresh(now),
        "limit": f.limit.as_ref().map(money_json),
        "used": f.used.as_ref().map(money_json),
        "percent": f.percent,
        "severity": f.severity,
        "resets_at": f.resets_at,
        "limit_reached": f.limit_reached,
        "scope": f.scope.key(),
        "comparison": comparison,
        "comparison_skipped": skipped,
    })
}

/// Whether this is the monthly dollar limit the burn-up chart follows.
pub(crate) fn is_monthly_dollars(l: &Limit) -> bool {
    l.period == LimitPeriod::Monthly && l.unit == Unit::Dollars
}

/// The limits and burn rates as JSON, for `--json`.
pub fn report_json(s: &Sources, events: &[UsageEvent], now: i64, week: WeekStart) -> Value {
    let snapshot = s.snapshot.as_ref();
    let limits = all_limits(s, events, now, week);
    let burnup = burnup_for(s, &limits, events, now);
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
        "capabilities": capabilities_json(s, now),
        "spend": spend_json(s, events, now),
        "spend_unavailable": s.spend_unavailable.map(Reason::key),
        "cache": s.account.cache.as_ref().map(|c| json!({
            "fetched_at": c.fetched_at,
            "age_seconds": c.age(now),
            "fresh": c.is_fresh(now),
        })),
        "limits": limits.iter().map(|l| {
            let mut v = l.to_json(now);
            if let (Some(map), true) = (v.as_object_mut(), is_monthly_dollars(l)) {
                map.insert("series".into(), burnup.series_json());
            }
            v
        }).collect::<Vec<_>>(),
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
        Origin::Server => "server billed",
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

/// An instant as text, in the chosen zone.
fn stamp(at: i64) -> String {
    crate::tz::iso(at).replace('T', " ")
}

/// One threshold as a phrase.
pub(crate) fn crossing_text(c: &Crossing) -> String {
    let pct = c.pct;
    match c.state {
        CrossState::Crossed { at } => format!("{pct}% crossed about {}", stamp(at)),
        CrossState::Projected { at } => format!("{pct}% projected about {}", stamp(at)),
        CrossState::Beyond { period_end } => {
            format!("{pct}% not before the period ends {}", stamp(period_end))
        }
        CrossState::Unknown => format!("{pct}% too early to say"),
    }
}

/// How the scope of a figure is named. Nothing says "your spend": whose
/// spend it is stays as the server reported it.
fn scope_text(scope: Scope) -> &'static str {
    match scope {
        Scope::Member => "member scope",
        Scope::Org => "organization scope",
        Scope::Unspecified => "as reported by the server",
    }
}

/// The server figure used for the monthly limit, if one is.
pub(crate) fn server_in_use(s: &Sources, now: i64) -> Option<&SpendFigure> {
    if matches!(monthly(s, now), Monthly::Server { .. }) {
        s.server.as_ref()
    } else {
        None
    }
}

/// The "server billed" sentence: the amounts, the share, what is left, the
/// reset time, the severity and whose spend it is.
fn billed_line(f: &SpendFigure, limits: &[Limit]) -> String {
    let mut parts = Vec::new();
    match (f.used_usd(), f.limit_usd()) {
        (Some(used), Some(limit)) => {
            parts.push(format!("{} of {}", dollars(used), dollars(limit)));
            if let Some(p) = f.percent {
                parts.push(format!("{p:.1}%"));
            }
            parts.push(format!("{} left", dollars((limit - used).max(0.0))));
        }
        (None, Some(limit)) => parts.push(format!(
            "limit {}, amount used not reported",
            dollars(limit)
        )),
        _ => {}
    }
    if let Some(l) = limits.iter().find(|l| l.origin == Origin::Server) {
        parts.push(format!("resets {}", stamp(f.resets_at.unwrap_or(l.end))));
    }
    if let Some(sev) = &f.severity {
        parts.push(format!("severity {sev}"));
    }
    parts.push(scope_text(f.scope).to_string());
    format!("server billed: {}", parts.join(", "))
}

/// The lines about the server spend limit for `--summary`: what the server
/// billed, how it compares with the local estimate, the plan, when 50, 80 and
/// 100 percent are crossed, and the one line saying why there is no figure
/// when there is none in use.
pub fn spend_lines(s: &Sources, limits: &[Limit], events: &[UsageEvent], now: i64) -> Vec<String> {
    let mut out = Vec::new();
    let figure = server_in_use(s, now);
    if let Some(f) = figure {
        out.push(billed_line(f, limits));
        match compare_server(s, events, now) {
            Some(Ok(c)) => out.push(c.line()),
            Some(Err(why)) => out.push(format!("local estimate not compared: {why}")),
            None => {}
        }
        out.push(format!("plan: {}", plan_label(s)));
    }
    // A stale server figure leaves the amount used at this machine's estimate.
    let local_used = figure.is_some_and(|f| !f.is_fresh(now));
    for l in limits {
        if let Some(c) = l.thresholds(now) {
            let basis = if local_used && l.origin == Origin::Server {
                ", from the local estimate"
            } else {
                ""
            };
            out.push(format!(
                "{} crossings ({}{basis}): {}",
                l.period.key().replace('_', " "),
                l.origin.key(),
                c.iter().map(crossing_text).collect::<Vec<_>>().join(", ")
            ));
        }
    }
    if figure.is_none() {
        out.extend(s.spend_unavailable.map(Reason::line));
    }
    out
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
    for line in spend_lines(s, &limits, events, now) {
        out.push_str(&format!("  {line}\n"));
    }
    if limits.iter().any(is_monthly_dollars) {
        out.push_str("  burn-up, cumulative spend this period\n");
        for line in burnup_for(s, &limits, events, now).table(BURNUP_TABLE_ROWS) {
            out.push_str(&format!("    {line}\n"));
        }
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
    if let Some(f) = s
        .server
        .as_ref()
        .filter(|_| limits.iter().any(|l| l.origin == Origin::Server))
    {
        out.push_str(&format!(
            "  note: the server billed figure was fetched {} ago\n",
            super::period::elapsed_text(f.age(now))
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
    fn the_plan_setting_changes_only_the_label_in_the_json() {
        let s = Sources {
            plan_choice: PlanChoice::Team,
            budgets: vec![(LimitPeriod::Monthly, 100.0)],
            ..Sources::default()
        };

        let mut v = report_json(&s, &[], NOW, WeekStart::Monday);
        let mut base = report_json(
            &Sources {
                plan_choice: PlanChoice::Auto,
                ..s.clone()
            },
            &[],
            NOW,
            WeekStart::Monday,
        );

        assert_eq!(v["plan_label"], "Team (set by you)");
        for key in ["plan_label", "capabilities"] {
            v.as_object_mut().unwrap().remove(key);
            base.as_object_mut().unwrap().remove(key);
        }
        assert_eq!(v, base);
    }

    #[test]
    fn load_reads_the_plan_setting_from_the_config_store() {
        let home = crate::util::scratch_dir("limits-load-plan");
        let dir = home.join(".config/playbook");
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("config.json"), r#"{"usage":{"plan":"team"}}"#).unwrap();
        let none = crate::util::scratch_dir("limits-load-plan-none");

        let set = Sources::load(None, &home.join(".claude.json"), &home);
        let unset = Sources::load(None, &none.join(".claude.json"), &none);

        assert_eq!(plan_label(&set), "Team (set by you)");
        assert_eq!(unset.plan_choice, PlanChoice::Auto);
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
            ..Sources::default()
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

    use crate::spend::{Money, SpendFigure};

    const HOUR: i64 = 3600;
    const MONTH_START: i64 = 1_798_761_600;

    fn dollars(minor: i64) -> Option<Money> {
        Some(Money {
            minor,
            decimals: 2,
            currency: Some("USD".to_string()),
        })
    }

    fn server_at(fetched_at: i64, limit_minor: i64, used_minor: Option<i64>) -> SpendFigure {
        SpendFigure {
            limit: dollars(limit_minor),
            used: used_minor.and_then(dollars),
            percent: None,
            severity: None,
            resets_at: None,
            limit_reached: None,
            scope: crate::spend::Scope::Unspecified,
            windows: Vec::new(),
            fetched_at,
            origin: Origin::Server,
        }
    }

    fn monthly_of(l: &[Limit]) -> Vec<&Limit> {
        l.iter()
            .filter(|x| x.period == LimitPeriod::Monthly)
            .collect()
    }

    #[test]
    fn a_measured_limit_beats_a_fresh_server_figure() {
        let s = Sources {
            snapshot: Some(gateway_snapshot()),
            server: Some(server_at(NOW - 60, 50_000, Some(20_000))),
            budgets: vec![(LimitPeriod::Monthly, 100.0)],
            ..Sources::default()
        };

        assert_eq!(monthly(&s, NOW), Monthly::Measured);
        let l = all_limits(&s, &[], NOW, WeekStart::Monday);
        assert!(monthly_of(&l).iter().all(|x| x.origin != Origin::Server));
    }

    #[test]
    fn a_fresh_server_figure_replaces_the_local_sum_and_the_manual_budget() {
        let s = Sources {
            server: Some(server_at(NOW - 60, 150_000, Some(105_950))),
            budgets: vec![(LimitPeriod::Monthly, 100.0)],
            ..Sources::default()
        };

        let l = all_limits(&s, &[cost(NOW - 10, 3.0)], NOW, WeekStart::Monday);

        let m = monthly_of(&l);
        assert_eq!(m.len(), 1);
        assert_eq!(m[0].origin, Origin::Server);
        assert_eq!(m[0].limit, 1500.0);
        assert_eq!(m[0].used, 1059.5);
        assert_eq!(m[0].to_json(NOW)["origin"], "server");
    }

    #[test]
    fn the_later_fetched_of_a_fresh_server_figure_and_a_fresh_cache_wins() {
        let cache_5h_old = Account {
            ..cache(5 * HOUR, spend(500.0, Some(120.0)), None)
        };
        let cache_1m_old = cache(60, spend(500.0, Some(120.0)), None);

        let server_newer = Sources {
            server: Some(server_at(NOW - 60, 150_000, None)),
            account: cache_5h_old,
            ..Sources::default()
        };
        let server_older = Sources {
            server: Some(server_at(NOW - 5 * HOUR, 150_000, None)),
            account: cache_1m_old,
            ..Sources::default()
        };

        assert!(matches!(
            monthly(&server_newer, NOW),
            Monthly::Server { .. }
        ));
        assert!(matches!(
            monthly(&server_older, NOW),
            Monthly::Cached { .. }
        ));
    }

    #[test]
    fn a_fresh_measured_cache_is_not_outranked_by_an_equally_old_server_figure() {
        let s = Sources {
            server: Some(server_at(NOW - 60, 150_000, None)),
            account: cache(60, spend(500.0, Some(120.0)), None),
            ..Sources::default()
        };

        assert!(matches!(monthly(&s, NOW), Monthly::Cached { .. }));
    }

    #[test]
    fn a_fresh_server_figure_is_used_when_it_is_the_only_source() {
        let s = Sources {
            server: Some(server_at(NOW - 60, 150_000, Some(1))),
            ..Sources::default()
        };

        assert!(matches!(
            monthly(&s, NOW),
            Monthly::Server { fresh: true, .. }
        ));
    }

    #[test]
    fn a_stale_server_figure_loses_to_a_fresh_cache_and_to_the_manual_budget() {
        let stale = server_at(NOW - 7 * HOUR, 150_000, Some(105_950));
        let with_cache = Sources {
            server: Some(stale.clone()),
            account: cache(600, spend(500.0, Some(120.0)), None),
            ..Sources::default()
        };
        let with_manual = Sources {
            server: Some(stale),
            budgets: vec![(LimitPeriod::Monthly, 100.0)],
            ..Sources::default()
        };

        assert!(matches!(monthly(&with_cache, NOW), Monthly::Cached { .. }));
        assert_eq!(monthly(&with_manual, NOW), Monthly::Manual(100.0));
    }

    #[test]
    fn a_stale_server_figure_is_the_last_resort_and_keeps_the_local_sum() {
        let s = Sources {
            server: Some(server_at(NOW - 7 * HOUR, 150_000, Some(105_950))),
            ..Sources::default()
        };

        assert!(matches!(
            monthly(&s, NOW),
            Monthly::Server { fresh: false, .. }
        ));
        let l = all_limits(&s, &[cost(NOW - 10, 3.0)], NOW, WeekStart::Monday);
        assert_eq!(monthly_of(&l)[0].used, 3.0);
    }

    #[test]
    fn the_later_of_a_stale_server_figure_and_a_stale_cache_is_the_last_resort() {
        let server_later = Sources {
            server: Some(server_at(NOW - 8 * HOUR, 150_000, None)),
            account: cache(9 * HOUR, spend(500.0, None), None),
            ..Sources::default()
        };
        let cache_later = Sources {
            server: Some(server_at(NOW - 9 * HOUR, 150_000, None)),
            account: cache(8 * HOUR, spend(500.0, None), None),
            ..Sources::default()
        };

        assert!(matches!(
            monthly(&server_later, NOW),
            Monthly::Server { .. }
        ));
        assert!(matches!(monthly(&cache_later, NOW), Monthly::Cached { .. }));
    }

    #[test]
    fn a_server_figure_fetched_before_the_month_began_is_ignored() {
        let s = Sources {
            server: Some(server_at(MONTH_START - 1, 150_000, Some(105_950))),
            ..Sources::default()
        };

        assert_eq!(monthly(&s, MONTH_START + HOUR), Monthly::Unset);
    }

    #[test]
    fn a_server_figure_from_the_future_or_without_a_dollar_limit_is_ignored() {
        let future = Sources {
            server: Some(server_at(NOW + HOUR, 150_000, None)),
            ..Sources::default()
        };
        let mut euro = server_at(NOW - 60, 150_000, None);
        euro.limit.as_mut().unwrap().currency = Some("EUR".to_string());
        let other = Sources {
            server: Some(euro),
            ..Sources::default()
        };

        assert_eq!(monthly(&future, NOW), Monthly::Unset);
        assert_eq!(monthly(&other, NOW), Monthly::Unset);
    }

    #[test]
    fn the_summary_says_server_billed_and_how_old_the_figure_is() {
        let s = Sources {
            server: Some(server_at(NOW - 600, 150_000, Some(105_950))),
            ..Sources::default()
        };

        let text = summary_section(&s, &[], NOW, WeekStart::Monday);

        assert!(
            text.contains("monthly budget: $1059.50 of $1500.00 used (server billed)"),
            "{text}"
        );
        assert!(
            text.contains("server billed figure was fetched 10m ago"),
            "{text}"
        );
    }

    #[test]
    fn a_fresh_server_figure_beats_a_stale_cache_and_the_manual_budget() {
        let s = Sources {
            server: Some(server_at(NOW - 60, 150_000, None)),
            account: cache(7 * HOUR, spend(500.0, Some(120.0)), None),
            budgets: vec![(LimitPeriod::Monthly, 100.0)],
            ..Sources::default()
        };

        assert!(matches!(
            monthly(&s, NOW),
            Monthly::Server { fresh: true, .. }
        ));
    }

    #[test]
    fn the_comparison_ignores_local_spend_after_the_figure_was_fetched() {
        let s = Sources {
            server: Some(server_at(NOW - 5 * HOUR, 150_000, Some(90_000))),
            ..Sources::default()
        };
        let events = [cost(NOW - 6 * HOUR, 900.0), cost(NOW - HOUR, 400.0)];

        let c = compare_server(&s, &events, NOW).unwrap().unwrap();

        assert_eq!(c.local_estimate_usd, 900.0);
        assert!(!c.disagrees);
    }

    #[test]
    fn the_month_is_the_utc_month_in_any_zone() {
        assert_eq!(utc_month_start(MONTH_START), MONTH_START);
        assert_eq!(utc_month_start(MONTH_START - 1), MONTH_START - 31 * 86_400);
        assert_eq!(utc_month_start(NOW), MONTH_START);
    }

    #[test]
    fn the_server_figure_is_compared_with_the_local_sum_of_the_month() {
        let s = Sources {
            server: Some(server_at(NOW - 60, 150_000, Some(105_950))),
            ..Sources::default()
        };
        let events = [
            cost(MONTH_START + 10, 900.0),
            cost(MONTH_START - 10, 5000.0),
        ];

        let c = compare_server(&s, &events, NOW).unwrap().unwrap();

        assert_eq!(c.local_estimate_usd, 900.0);
        assert!(c.disagrees);
    }

    #[test]
    fn nothing_is_compared_when_no_server_figure_is_used() {
        let s = Sources {
            budgets: vec![(LimitPeriod::Monthly, 100.0)],
            ..Sources::default()
        };

        assert!(compare_server(&s, &[], NOW).is_none());
    }

    fn with_server_summary() -> String {
        crate::tz::set(crate::tz::Zone::Utc);
        let s = Sources {
            server: Some(server_at(NOW - 600, 150_000, Some(105_950))),
            ..Sources::default()
        };
        let events = [cost(MONTH_START + 10, 900.0)];
        summary_section(&s, &events, NOW, WeekStart::Monday)
    }

    #[test]
    fn the_summary_shows_the_server_billed_line_the_comparison_and_the_plan() {
        let text = with_server_summary();

        assert!(
            text.contains(
                "  server billed: $1,059.50 of $1,500.00, $440.50 left, resets 2027-02-01 00:00:00Z, as reported by the server\n"
            ),
            "{text}"
        );
        assert!(
            text.contains("  local estimate (this machine only) $900.00 differs from server billed $1,059.50\n"),
            "{text}"
        );
        assert!(
            text.contains("  plan: Enterprise or unknown plan\n"),
            "{text}"
        );
        assert!(!text.contains("wrong"), "{text}");
    }

    #[test]
    fn the_summary_dates_every_crossing_from_the_fixed_clock() {
        let text = with_server_summary();

        assert!(
            text.contains(
                "  monthly crossings (server): 50% crossed about 2027-01-11 03:30:40Z, 80% projected about 2027-01-17 05:37:04Z, 100% projected about 2027-01-21 07:01:20Z\n"
            ),
            "{text}"
        );
    }

    #[test]
    fn a_figure_without_an_amount_used_says_so_and_is_not_compared() {
        crate::tz::set(crate::tz::Zone::Utc);
        let s = Sources {
            server: Some(server_at(NOW - 60, 150_000, None)),
            ..Sources::default()
        };

        let text = summary_section(&s, &[], NOW, WeekStart::Monday);

        assert!(
            text.contains("limit $1,500.00, amount used not reported"),
            "{text}"
        );
        assert!(
            text.contains("local estimate not compared: amount used not reported, not compared"),
            "{text}"
        );
    }

    #[test]
    fn the_summary_names_scope_and_severity_when_the_server_gave_them() {
        crate::tz::set(crate::tz::Zone::Utc);
        let mut f = server_at(NOW - 60, 150_000, Some(105_950));
        f.scope = Scope::Org;
        f.severity = Some("warning".to_string());
        f.percent = Some(70.6);
        let s = Sources {
            server: Some(f),
            ..Sources::default()
        };

        let text = summary_section(&s, &[], NOW, WeekStart::Monday);

        assert!(
            text.contains(
                "$440.50 left, resets 2027-02-01 00:00:00Z, severity warning, organization scope"
            ),
            "{text}"
        );
        assert!(text.contains("$1,059.50 of $1,500.00, 70.6%, "), "{text}");
    }

    #[test]
    fn each_unavailable_reason_prints_one_line_and_nothing_else_about_the_server() {
        for reason in [
            Reason::Disabled,
            Reason::NoConsent,
            Reason::NoToken,
            Reason::Unauthorized,
            Reason::RateLimited,
            Reason::Timeout,
            Reason::Schema,
            Reason::UnsupportedLogin,
            Reason::Network,
            Reason::Internal,
            Reason::Fetching,
        ] {
            let s = Sources {
                spend_unavailable: Some(reason),
                ..Sources::default()
            };

            let text = summary_section(&s, &[], NOW, WeekStart::Monday);
            let v = report_json(&s, &[], NOW, WeekStart::Monday);

            assert_eq!(
                text.matches("spend limit unavailable").count(),
                1,
                "{reason:?}"
            );
            assert!(text.contains(&format!("  {}\n", reason.line())), "{text}");
            assert!(!text.contains("server billed"), "{text}");
            assert_eq!(v["spend_unavailable"], reason.key());
            assert!(v["spend"].is_null());
        }
    }

    #[test]
    fn without_a_server_figure_or_a_reason_the_summary_adds_no_spend_line() {
        let s = Sources {
            budgets: vec![(LimitPeriod::Monthly, 100.0)],
            ..Sources::default()
        };

        let text = summary_section(&s, &[], NOW, WeekStart::Monday);
        let v = report_json(&s, &[], NOW, WeekStart::Monday);

        assert!(!text.contains("server billed"), "{text}");
        assert!(!text.contains("spend limit unavailable"), "{text}");
        assert!(v["spend"].is_null() && v["spend_unavailable"].is_null());
    }

    #[test]
    fn the_json_adds_spend_capabilities_and_thresholds_without_changing_old_keys() {
        let s = Sources {
            server: Some(server_at(NOW - 600, 150_000, Some(105_950))),
            ..Sources::default()
        };
        let events = [cost(MONTH_START + 10, 900.0)];

        let v = report_json(&s, &events, NOW, WeekStart::Monday);

        assert_eq!(v["spend"]["comparison"]["disagrees"], true);
        assert_eq!(v["spend"]["limit"]["minor"], 150_000);
        assert_eq!(v["spend"]["scope"], "unspecified");
        assert_eq!(v["spend"]["comparison_skipped"], Value::Null);
        assert_eq!(
            v["capabilities"]["lines"],
            json!(["monthly_cap", "local_estimate"])
        );
        assert_eq!(v["capabilities"]["local_only"], false);
        assert_eq!(v["limits"][0]["origin"], "server");
        assert_eq!(v["limits"][0]["thresholds"].as_array().unwrap().len(), 3);
        for key in [
            "known",
            "hint",
            "plan",
            "plan_label",
            "windows_note",
            "cache",
            "limits",
            "burn",
        ] {
            assert!(v.get(key).is_some(), "{key}");
        }
    }

    #[test]
    fn a_skipped_comparison_is_named_in_the_json() {
        let mut f = server_at(NOW - 60, 150_000, Some(105_950));
        f.used.as_mut().unwrap().currency = Some("EUR".to_string());
        let s = Sources {
            server: Some(f),
            ..Sources::default()
        };

        let v = report_json(&s, &[], NOW, WeekStart::Monday);

        // The limit is in dollars, the amount used in euros: no comparison.
        assert_eq!(v["spend"]["comparison"], Value::Null);
        assert_eq!(
            v["spend"]["comparison_skipped"],
            "currency EUR, not compared"
        );
    }

    #[test]
    fn a_manual_budget_gets_its_own_crossings_line_and_no_server_text() {
        crate::tz::set(crate::tz::Zone::Utc);
        let s = Sources {
            budgets: vec![(LimitPeriod::Monthly, 1500.0)],
            ..Sources::default()
        };
        let events = [cost(MONTH_START + 10, 1059.5)];

        let text = summary_section(&s, &events, NOW, WeekStart::Monday);

        assert!(
            text.contains("  monthly crossings (manual): 50% crossed about"),
            "{text}"
        );
        assert!(!text.contains("server billed"), "{text}");
    }

    #[test]
    fn a_server_figure_that_is_not_in_use_prints_and_reports_nothing() {
        let s = Sources {
            server: Some(server_at(MONTH_START - 1, 150_000, Some(105_950))),
            spend_unavailable: Some(Reason::Network),
            ..Sources::default()
        };

        let text = summary_section(&s, &[], NOW, WeekStart::Monday);
        let v = report_json(&s, &[], NOW, WeekStart::Monday);

        assert!(!text.contains("server billed"), "{text}");
        assert!(text.contains("spend limit unavailable (network)"), "{text}");
        assert!(v["spend"].is_null());
    }

    #[test]
    fn a_stale_figure_in_use_hides_the_unavailable_line_and_labels_its_crossings() {
        crate::tz::set(crate::tz::Zone::Utc);
        let s = Sources {
            server: Some(server_at(NOW - 7 * HOUR, 150_000, Some(105_950))),
            spend_unavailable: Some(Reason::Network),
            ..Sources::default()
        };

        let text = summary_section(&s, &[cost(NOW - 10, 3.0)], NOW, WeekStart::Monday);

        assert!(text.contains("server billed:"), "{text}");
        assert!(!text.contains("spend limit unavailable"), "{text}");
        assert!(
            text.contains("monthly crossings (server, from the local estimate):"),
            "{text}"
        );
    }

    #[test]
    fn the_summary_prints_the_burn_up_table_for_a_monthly_dollar_limit_only() {
        crate::tz::set(crate::tz::Zone::Utc);
        let with = Sources {
            budgets: vec![(LimitPeriod::Monthly, 100.0)],
            ..Sources::default()
        };
        let weekly = Sources {
            budgets: vec![(LimitPeriod::Weekly, 100.0)],
            ..Sources::default()
        };
        let events = [
            cost(MONTH_START + 10, 25.0),
            cost(MONTH_START + 3 * 86_400, 25.0),
        ];

        let text = summary_section(&with, &events, NOW, WeekStart::Monday);
        let other = summary_section(&weekly, &events, NOW, WeekStart::Monday);

        assert!(
            text.contains("  burn-up, cumulative spend this period\n"),
            "{text}"
        );
        assert!(
            text.contains("    2027-01-01        $25.00     25.0%\n"),
            "{text}"
        );
        assert!(
            text.contains("    2027-01-04        $50.00     50.0%\n"),
            "{text}"
        );
        assert!(!other.contains("burn-up"), "{other}");
    }

    #[test]
    fn the_json_gives_the_monthly_dollar_limit_a_day_series_and_no_other_limit() {
        let s = Sources {
            budgets: vec![(LimitPeriod::Monthly, 100.0), (LimitPeriod::Weekly, 40.0)],
            ..Sources::default()
        };
        let events = [cost(MONTH_START + 10, 25.0)];

        let v = report_json(&s, &events, NOW, WeekStart::Monday);

        let limits = v["limits"].as_array().unwrap();
        let monthly = limits.iter().find(|l| l["period"] == "monthly").unwrap();
        let weekly = limits.iter().find(|l| l["period"] == "weekly").unwrap();
        assert_eq!(monthly["series"]["days"][0]["day"], "2027-01-01");
        assert_eq!(monthly["series"]["days"][0]["cumulative_usd"], 25.0);
        assert!(monthly["series"]["projected_end_usd"].is_number());
        assert!(weekly.get("series").is_none());
    }
}
