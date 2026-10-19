// SPDX-FileCopyrightText: 2026 Igor Santos
// SPDX-License-Identifier: Apache-2.0

//! The plan and the cached usage numbers Claude Code keeps in its own
//! `~/.claude.json`. [`read`] takes only the plan tier, the usage windows and
//! the spend limit, and keeps nothing that names a person or an organization.
//! [`report`] also lists the names of the account fields and, behind `raw`,
//! the values of six plan fields that hold no identity. Nothing here is
//! documented by Anthropic, so every read can come back empty and every
//! caller treats empty as "unknown".
//!
//! The plan is a guess from the rate limit tier names. Only the two tiers seen
//! on real Max accounts are recognized. Any other value, including every
//! Enterprise value, maps to [`Plan::EnterpriseOrUnknown`] instead of being
//! guessed at.

use crate::claude_code::parse_iso8601_utc;
use crate::limits::{LimitPeriod, Unit};
use crate::limits_source::{self, Window};
use crate::period::elapsed_text;
use serde_json::Value;
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::time::SystemTime;

/// The account fields that describe a plan and carry no identity. Only these
/// are ever shown with their values, and only behind `--raw`.
const PLAN_FIELDS: [&str; 6] = [
    "billingType",
    "organizationType",
    "seatTier",
    "organizationRateLimitTier",
    "userRateLimitTier",
    "hasExtraUsageEnabled",
];

/// How far ahead of this clock a fetch time may be before it is not believed.
pub(crate) const CLOCK_SKEW_SECONDS: i64 = 300;

/// What the account looks like from the tier names.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Plan {
    Max5x,
    Max20x,
    /// Anything else: Enterprise, Team, Pro, an API account or a value that
    /// this version does not know.
    EnterpriseOrUnknown,
}

impl Plan {
    /// A stable name for scripts.
    pub fn key(self) -> &'static str {
        match self {
            Plan::Max5x => "max_5x",
            Plan::Max20x => "max_20x",
            Plan::EnterpriseOrUnknown => "enterprise_or_unknown",
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Plan::Max5x => "Max 5x",
            Plan::Max20x => "Max 20x",
            Plan::EnterpriseOrUnknown => "Enterprise or unknown plan",
        }
    }

    /// Whether this plan is known to have the 5 hour and weekly windows.
    pub fn has_windows(self) -> bool {
        !matches!(self, Plan::EnterpriseOrUnknown)
    }

    fn from_tiers(tiers: &[&str]) -> Plan {
        let has = |needle: &str| tiers.iter().any(|t| t.to_lowercase().contains(needle));
        if has("max_20x") {
            Plan::Max20x
        } else if has("max_5x") {
            Plan::Max5x
        } else {
            Plan::EnterpriseOrUnknown
        }
    }
}

/// The monthly spend limit of an account, in dollars.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct SpendLimit {
    pub limit_usd: f64,
    pub used_usd: Option<f64>,
}

/// The usage numbers Claude Code last fetched.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct UsageCache {
    /// When Claude Code fetched them, UTC seconds.
    pub fetched_at: i64,
    pub five_hour: Option<Window>,
    pub seven_day: Option<Window>,
    pub spend: Option<SpendLimit>,
}

impl UsageCache {
    /// Seconds since the fetch, never negative.
    pub fn age(&self, now: i64) -> i64 {
        (now - self.fetched_at).max(0)
    }

    /// Whether the fetch time is later than `now` by more than a clock can
    /// drift. Such a time is wrong, so the numbers are never called fresh.
    pub fn is_from_the_future(&self, now: i64) -> bool {
        self.fetched_at > now + CLOCK_SKEW_SECONDS
    }

    pub fn is_fresh(&self, now: i64) -> bool {
        !self.is_from_the_future(now) && self.age(now) <= limits_source::STALE_SECONDS
    }
}

/// What was read from the config file.
#[derive(Debug, Clone, PartialEq)]
pub struct Account {
    pub plan: Plan,
    pub cache: Option<UsageCache>,
}

impl Default for Account {
    fn default() -> Self {
        Account {
            plan: Plan::EnterpriseOrUnknown,
            cache: None,
        }
    }
}

fn load(path: &Path) -> Option<Value> {
    serde_json::from_str(&std::fs::read_to_string(path).ok()?).ok()
}

/// An ISO 8601 time with an optional fraction and an optional `Z` or
/// `+hh:mm` / `-hh:mm` offset, as UTC seconds. Claude Code writes
/// `2026-09-03T05:00:00.123456+00:00`.
pub(crate) fn parse_time(text: &str) -> Option<i64> {
    let (date, time) = text.split_once('T')?;
    let cut = time.find(['.', 'Z', '+', '-']).unwrap_or(time.len());
    let base = parse_iso8601_utc(&format!("{date}T{}Z", &time[..cut]))?;
    let rest = &time[cut..];
    let offset = match rest.find(['+', '-']) {
        None => 0,
        Some(i) => {
            let sign = if rest[i..].starts_with('-') { -1 } else { 1 };
            let (h, m) = rest[i + 1..].split_once(':')?;
            sign * (h.parse::<i64>().ok()? * 3600 + m.parse::<i64>().ok()? * 60)
        }
    };
    Some(base - offset)
}

pub(crate) fn window(v: Option<&Value>) -> Option<Window> {
    let v = v?;
    let pct = v.get("utilization")?.as_f64()?;
    let at = parse_time(v.get("resets_at")?.as_str()?)?;
    (pct.is_finite() && pct >= 0.0).then_some(Window {
        used_percentage: pct,
        resets_at: at,
    })
}

/// `{amount_minor, currency, exponent}` as dollars, only for USD.
fn usd(v: Option<&Value>) -> Option<f64> {
    let v = v?;
    let currency = v.get("currency")?.as_str()?;
    let minor = v.get("amount_minor")?.as_f64()?;
    let exponent = v.get("exponent")?.as_i64()?;
    ((0..=6).contains(&exponent) && currency.eq_ignore_ascii_case("USD") && minor >= 0.0)
        .then(|| minor / 10f64.powi(exponent as i32))
}

fn spend(v: Option<&Value>) -> Option<SpendLimit> {
    let v = v?;
    if v.get("enabled").and_then(Value::as_bool) == Some(false) {
        return None;
    }
    let limit_usd = usd(v.get("limit")).filter(|n| *n > 0.0)?;
    Some(SpendLimit {
        limit_usd,
        used_usd: usd(v.get("used")),
    })
}

fn cache(root: &Value) -> Option<UsageCache> {
    let c = root.get("cachedUsageUtilization")?;
    let u = c.get("utilization")?;
    Some(UsageCache {
        fetched_at: c.get("fetchedAtMs")?.as_i64()? / 1000,
        five_hour: window(u.get("five_hour")),
        seven_day: window(u.get("seven_day")),
        spend: spend(u.get("spend")),
    })
}

fn account(root: &Value) -> Account {
    let tier = |k: &str| root.pointer(&format!("/oauthAccount/{k}"))?.as_str();
    let tiers: Vec<&str> = ["organizationRateLimitTier", "userRateLimitTier"]
        .iter()
        .filter_map(|k| tier(k))
        .collect();
    Account {
        plan: Plan::from_tiers(&tiers),
        cache: cache(root),
    }
}

/// Reads the plan and the usage cache from the config file at `path`. A
/// missing, unreadable or odd file gives the default: unknown plan, no cache.
pub fn read(path: &Path) -> Account {
    load(path).map_or_else(Account::default, |root| account(&root))
}

/// What the last read of the config file returned, and which file state it
/// came from.
struct Seen {
    path: PathBuf,
    modified: Option<SystemTime>,
    len: u64,
    account: Account,
}

static LAST: Mutex<Option<Seen>> = Mutex::new(None);

/// Like [`read`], for the terminal view that reads on every refresh. The file
/// is parsed again only when its size or modification time changed. A file
/// that exists but does not parse, as while Claude Code is rewriting it,
/// keeps the last good answer instead of resetting the plan to unknown.
pub fn read_cached(path: &Path) -> Account {
    let Ok(meta) = std::fs::metadata(path) else {
        return Account::default();
    };
    let (modified, len) = (meta.modified().ok(), meta.len());
    let Ok(mut last) = LAST.lock() else {
        return read(path);
    };
    let same_file = |s: &Seen| s.path == path;
    if let Some(seen) = last.as_ref().filter(|s| same_file(s)) {
        if seen.modified == modified && seen.len == len {
            return seen.account.clone();
        }
    }
    match load(path) {
        Some(root) => {
            let account = account(&root);
            *last = Some(Seen {
                path: path.to_path_buf(),
                modified,
                len,
                account: account.clone(),
            });
            account
        }
        None => last
            .as_ref()
            .filter(|s| same_file(s))
            .map_or_else(Account::default, |s| s.account.clone()),
    }
}

fn names(v: Option<&Value>) -> Vec<String> {
    v.and_then(Value::as_object)
        .map(|o| o.keys().cloned().collect())
        .unwrap_or_default()
}

/// A plan field value that is safe to print: a flag, a number or a short
/// lowercase tier name. Anything else could hold a name or an id.
fn plain_value(v: &Value) -> String {
    match v {
        Value::Bool(_) | Value::Number(_) => v.to_string(),
        Value::String(t)
            if !t.is_empty()
                && t.len() <= 40
                && t.chars()
                    .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_') =>
        {
            v.to_string()
        }
        _ => "<other value>".to_string(),
    }
}

fn list(items: &[String]) -> String {
    if items.is_empty() {
        "none".to_string()
    } else {
        items.join(", ")
    }
}

/// One window of the cache. `raw` is the field as stored: a value that is
/// there but did not parse is reported as such and not as absent.
fn window_line(name: &str, w: Option<&Window>, raw: Option<&Value>, now: i64) -> String {
    match w {
        None if raw.is_some_and(|v| !v.is_null()) => {
            format!("    {name}: present but not readable")
        }
        None => format!("    {name}: absent"),
        Some(w) if w.resets_at > now => format!(
            "    {name}: present, {:.0}% used, resets in {}",
            w.used_percentage,
            elapsed_text(w.resets_at - now)
        ),
        Some(w) => format!(
            "    {name}: present, its reset was {} ago, so it no longer applies",
            elapsed_text(now - w.resets_at)
        ),
    }
}

fn cache_lines(root: &Value, now: i64, raw: bool) -> Vec<String> {
    let mut out = Vec::new();
    let key = "  Usage cache (cachedUsageUtilization):";
    let Some(c) = cache(root) else {
        out.push(match root.get("cachedUsageUtilization") {
            None | Some(Value::Null) => format!("{key} not found"),
            Some(v) => format!(
                "{key} found but not readable (fields: {})",
                list(&names(Some(v)))
            ),
        });
        return out;
    };
    let state = if c.is_from_the_future(now) {
        "its time is in the future, so it is not trusted".to_string()
    } else if c.is_fresh(now) {
        "fresh".to_string()
    } else {
        "stale".to_string()
    };
    out.push(format!(
        "{key} found, {} old ({state})",
        elapsed_text(c.age(now))
    ));
    let u = |k: &str| root.pointer(&format!("/cachedUsageUtilization/utilization/{k}"));
    out.push(window_line(
        "five_hour",
        c.five_hour.as_ref(),
        u("five_hour"),
        now,
    ));
    out.push(window_line(
        "seven_day",
        c.seven_day.as_ref(),
        u("seven_day"),
        now,
    ));
    let s = u("spend").filter(|v| !v.is_null());
    out.push(match (&c.spend, s) {
        (Some(_), _) => "    monthly spend limit: present, in dollars".to_string(),
        (None, Some(s)) if s.get("enabled").and_then(Value::as_bool) == Some(false) => {
            "    monthly spend limit: the cache says it is off".to_string()
        }
        (None, Some(_)) => {
            "    monthly spend limit: present but not usable (zero or not in dollars)".to_string()
        }
        (None, None) => "    monthly spend limit: absent".to_string(),
    });
    if raw {
        if let Some(s) = &c.spend {
            let used = s
                .used_usd
                .map_or("unknown".to_string(), |n| format!("{n:.2}"));
            out.push(format!(
                "    raw: spend limit {:.2} USD, used {used} USD",
                s.limit_usd
            ));
        }
    }
    let valued: Vec<String> = root
        .pointer("/cachedUsageUtilization/utilization")
        .and_then(Value::as_object)
        .map(|o| {
            o.iter()
                .filter(|(_, v)| !v.is_null())
                .map(|(k, _)| k.clone())
                .collect()
        })
        .unwrap_or_default();
    out.push(format!("    fields with a value: {}", list(&valued)));
    out
}

fn statusline_lines(usage_dir: &Path, now: i64) -> Vec<String> {
    let Some(s) = limits_source::load(usage_dir) else {
        return vec![
            "  Statusline reading (rate_limits.json): not found".to_string(),
            "    it appears after a session that uses `playbook statusline` on a plan with usage windows".to_string(),
        ];
    };
    let age = (now - s.captured_at).max(0);
    let state = if s.is_stale(now) { "stale" } else { "fresh" };
    let yes = |b: bool| if b { "present" } else { "absent" };
    vec![
        format!(
            "  Statusline reading (rate_limits.json): found, {} old ({state})",
            elapsed_text(age)
        ),
        format!("    five_hour: {}", yes(s.five_hour.is_some())),
        format!("    seven_day: {}", yes(s.seven_day.is_some())),
        format!("    spend_limit: {}", yes(s.spend_limit.is_some())),
    ]
}

/// What the panels will use, given the sources above.
fn verdict_lines(s: &limits_source::Sources, now: i64) -> Vec<String> {
    use crate::limits::Origin;
    use crate::query::WeekStart;
    let limits = limits_source::all_limits(s, &[], now, WeekStart::default());
    let mut out = vec!["What pbtop will show".to_string()];
    for (period, name) in [
        (LimitPeriod::FiveHour, "5 hour window"),
        (LimitPeriod::SevenDay, "weekly window"),
    ] {
        let from = match limits.iter().find(|l| l.period == period) {
            Some(l) if l.origin == Origin::Cached => "from the Claude Code cache",
            Some(_) => "from the statusline reading",
            None => "not known",
        };
        out.push(format!("  {name}: {from}"));
    }
    if let Some(note) = limits_source::windows_note(s, &limits) {
        out.push(format!("  {note}"));
    }
    out.push(match limits_source::monthly(s, now) {
        limits_source::Monthly::Measured => {
            "  monthly limit: from the statusline reading (a manual budget is shown beside it)"
                .to_string()
        }
        limits_source::Monthly::Server {
            limit_usd,
            age,
            fresh,
            ..
        } => format!(
            "  monthly limit: {} billed by the server, fetched {} ago{}",
            limits_source::amount(Unit::Dollars, limit_usd),
            elapsed_text(age),
            if fresh { "" } else { " (stale)" }
        ),
        limits_source::Monthly::Cached {
            limit_usd,
            age,
            fresh,
            ..
        } => format!(
            "  monthly limit: {} from the Claude Code cache, {} old{}",
            limits_source::amount(Unit::Dollars, limit_usd),
            elapsed_text(age),
            if fresh { "" } else { " (stale)" }
        ),
        limits_source::Monthly::Manual(usd) => {
            format!(
                "  monthly limit: {} from usage.budget.monthlyUsd",
                limits_source::amount(Unit::Dollars, usd)
            )
        }
        limits_source::Monthly::Unset => {
            "  monthly limit: none (set usage.budget.monthlyUsd)".to_string()
        }
    });
    out
}

/// The text of `playbook usage doctor`: which sources of limits exist on this
/// machine. It lists field names, ages, the plan label, window percentages and
/// manual budgets. Values of the plan fields and the cached spend limit appear
/// only with `raw`. Emails, names, ids and organization names are never read
/// into the text.
pub fn report(
    claude_json: &Path,
    usage_dir: &Path,
    budgets: &[(LimitPeriod, f64)],
    now: i64,
    raw: bool,
) -> String {
    let mut out = vec!["Where pbtop can read limits on this machine".to_string()];
    match load(claude_json) {
        None => out.push("  Claude config file: not found or not readable".to_string()),
        Some(root) => {
            out.push("  Claude config file: found".to_string());
            let fields = root.get("oauthAccount");
            out.push(format!(
                "    account fields present (names only): {}",
                list(&names(fields))
            ));
            out.push(format!("    plan: {}", account(&root).plan.label()));
            if raw {
                for key in PLAN_FIELDS {
                    let value = fields
                        .and_then(|a| a.get(key))
                        .map_or("absent".to_string(), plain_value);
                    out.push(format!("    raw: {key} = {value}"));
                }
            }
            out.extend(cache_lines(&root, now, raw));
        }
    }
    out.extend(statusline_lines(usage_dir, now));
    out.push("  Manual budgets (usage.budget.*):".to_string());
    if budgets.is_empty() {
        out.push("    none set".to_string());
    }
    for (period, usd) in budgets {
        out.push(format!("    {}: ${usd:.2}", period.key()));
    }
    let sources =
        limits_source::Sources::new(Some(usage_dir), claude_json, budgets.to_vec(), false);
    out.push(String::new());
    out.extend(verdict_lines(&sources, now));
    if !raw {
        out.push(String::new());
        out.push(
            "Add --raw to also print the plan field values and the cached spend limit (no email, name or id).".to_string(),
        );
    }
    out.join("\n")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::limits_source::Snapshot;
    use std::path::PathBuf;

    const NOW: i64 = 1_788_400_000;
    // 2026-09-03T05:00:00Z, 3h 13m after NOW.
    const RESET: &str = "2026-09-03T05:00:00.123456+00:00";

    /// A folder holding one `.claude.json`, removed when it goes out of scope.
    struct Home(PathBuf);

    impl Home {
        fn new(name: &str, body: &str) -> Home {
            let dir = std::env::temp_dir().join(format!("pb-plan-{}-{name}", std::process::id()));
            std::fs::create_dir_all(&dir).unwrap();
            std::fs::write(dir.join(".claude.json"), body).unwrap();
            Home(dir)
        }

        fn json(&self) -> PathBuf {
            self.0.join(".claude.json")
        }
    }

    impl Drop for Home {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    fn cache_body(fetched_ms: i64, spend: &str) -> String {
        format!(
            r#"{{"oauthAccount":{{"organizationRateLimitTier":"default_claude_max_5x",
              "emailAddress":"a@b.c","displayName":"Sentinel Name","accountUuid":"sentinel-uuid-1",
              "organizationName":"Sentinel Org","organizationUuid":"sentinel-uuid-2"}},
            "cachedUsageUtilization":{{"fetchedAtMs":{fetched_ms},"utilization":{{
              "five_hour":{{"utilization":40,"resets_at":"{RESET}"}},
              "seven_day":null,
              "spend":{spend}}}}}}}"#
        )
    }

    fn report_of(home: &Home, now: i64, raw: bool) -> String {
        report(&home.json(), &home.0, &[], now, raw)
    }

    #[test]
    fn a_max_tier_name_gives_that_plan() {
        let max5 = Plan::from_tiers(&["default_claude_max_5x"]);
        let max20 = Plan::from_tiers(&["x", "default_claude_max_20x"]);

        assert_eq!(max5, Plan::Max5x);
        assert_eq!(max20, Plan::Max20x);
    }

    #[test]
    fn any_other_tier_name_is_enterprise_or_unknown() {
        let plan = Plan::from_tiers(&["default_raven", "something_new"]);

        assert_eq!(plan, Plan::EnterpriseOrUnknown);
        assert_eq!(plan.label(), "Enterprise or unknown plan");
        assert!(!plan.has_windows());
    }

    #[test]
    fn a_missing_file_reads_as_an_unknown_plan_with_no_cache() {
        let account = read(Path::new("/nonexistent/.claude.json"));

        assert_eq!(account, Account::default());
    }

    #[test]
    fn a_file_that_is_not_json_reads_as_unknown() {
        let home = Home::new("garbage", "not json {");

        assert_eq!(read(&home.json()), Account::default());
    }

    #[test]
    fn the_cache_gives_windows_and_a_dollar_spend_limit() {
        let spend = r#"{"enabled":true,"limit":{"amount_minor":50000,"currency":"USD","exponent":2},"used":{"amount_minor":1234,"currency":"USD","exponent":2}}"#;
        let home = Home::new("full", &cache_body(NOW * 1000, spend));

        let account = read(&home.json());

        let c = account.cache.unwrap();
        assert_eq!(account.plan, Plan::Max5x);
        assert_eq!(c.fetched_at, NOW);
        let w = c.five_hour.unwrap();
        assert_eq!(w.used_percentage, 40.0);
        assert_eq!(w.resets_at, 1_788_411_600);
        assert!(c.seven_day.is_none());
        assert_eq!(
            c.spend,
            Some(SpendLimit {
                limit_usd: 500.0,
                used_usd: Some(12.34)
            })
        );
    }

    #[test]
    fn a_spend_limit_that_is_off_or_not_in_dollars_is_ignored() {
        let off =
            r#"{"enabled":false,"limit":{"amount_minor":50000,"currency":"USD","exponent":2}}"#;
        let eur =
            r#"{"enabled":true,"limit":{"amount_minor":50000,"currency":"EUR","exponent":2}}"#;
        let zero = r#"{"enabled":true,"limit":{"amount_minor":0,"currency":"USD","exponent":2}}"#;

        for (name, body) in [("off", off), ("eur", eur), ("zero", zero)] {
            let home = Home::new(name, &cache_body(NOW * 1000, body));

            assert!(read(&home.json()).cache.unwrap().spend.is_none(), "{name}");
        }
    }

    #[test]
    fn times_with_a_fraction_an_offset_or_a_z_all_parse() {
        let z = parse_time("2026-09-03T05:00:00Z");
        let fraction = parse_time("2026-09-03T05:00:00.5Z");
        let zero_offset = parse_time("2026-09-03T05:00:00+00:00");
        let plus = parse_time("2026-09-03T07:00:00+02:00");
        let minus = parse_time("2026-09-03T00:00:00-05:00");

        for t in [fraction, zero_offset, plus, minus] {
            assert_eq!(t, z);
        }
        assert_eq!(z, Some(1_788_411_600));
        assert_eq!(parse_time("soon"), None);
    }

    #[test]
    fn malformed_windows_and_amounts_are_none() {
        let windows = [
            r#"{"utilization":-1,"resets_at":"2026-09-03T05:00:00Z"}"#,
            r#"{"utilization":"40","resets_at":"2026-09-03T05:00:00Z"}"#,
            r#"{"utilization":40,"resets_at":"tomorrow"}"#,
            r#"{"utilization":40,"resets_at":1788411600}"#,
        ];
        let amounts = [
            r#"{"amount_minor":-5,"currency":"USD","exponent":2}"#,
            r#"{"amount_minor":5,"currency":"USD","exponent":9}"#,
            r#"{"amount_minor":5,"currency":"USD","exponent":-1}"#,
            r#"{"amount_minor":"5","currency":"USD","exponent":2}"#,
        ];

        for w in windows {
            assert!(
                window(serde_json::from_str(w).ok().as_ref()).is_none(),
                "{w}"
            );
        }
        for a in amounts {
            assert!(usd(serde_json::from_str(a).ok().as_ref()).is_none(), "{a}");
        }
    }

    #[test]
    fn the_cache_is_fresh_for_six_hours_and_then_stale() {
        let c = UsageCache {
            fetched_at: NOW - 6 * 3600,
            ..UsageCache::default()
        };

        assert!(c.is_fresh(NOW));
        assert!(!c.is_fresh(NOW + 1));
    }

    #[test]
    fn a_fetch_time_in_the_future_is_never_fresh() {
        let c = UsageCache {
            fetched_at: NOW + 3600,
            ..UsageCache::default()
        };

        assert!(c.is_from_the_future(NOW));
        assert!(!c.is_fresh(NOW));
    }

    #[test]
    fn the_report_lists_field_names_and_ages_but_no_values() {
        let home = Home::new("report", &cache_body((NOW - 7200) * 1000, "null"));

        let text = report_of(&home, NOW, false);

        assert!(text.contains("organizationRateLimitTier"), "{text}");
        assert!(text.contains("plan: Max 5x"), "{text}");
        assert!(text.contains("found, 2h 0m old (fresh)"), "{text}");
        assert!(
            text.contains("five_hour: present, 40% used, resets in 3h 13m"),
            "{text}"
        );
        assert!(text.contains("seven_day: absent"), "{text}");
        assert!(text.contains("monthly spend limit: absent"), "{text}");
        assert!(!text.contains("default_claude_max_5x"), "{text}");
        assert!(!text.contains("raw:"), "{text}");
    }

    #[test]
    fn neither_report_prints_an_email_name_id_or_organization() {
        let home = Home::new("privacy", &cache_body(NOW * 1000, "null"));

        for raw in [false, true] {
            let text = report_of(&home, NOW, raw);

            for secret in [
                "a@b.c",
                "Sentinel Name",
                "sentinel-uuid-1",
                "Sentinel Org",
                "sentinel-uuid-2",
            ] {
                assert!(!text.contains(secret), "raw={raw} {secret}: {text}");
            }
        }
    }

    #[test]
    fn the_raw_report_adds_plan_field_values_and_the_spend_limit() {
        let spend = r#"{"enabled":true,"limit":{"amount_minor":50000,"currency":"USD","exponent":2},"used":{"amount_minor":1234,"currency":"USD","exponent":2}}"#;
        let home = Home::new("raw", &cache_body(NOW * 1000, spend));

        let text = report(
            &home.json(),
            &home.0,
            &[(LimitPeriod::Monthly, 100.0)],
            NOW,
            true,
        );

        assert!(
            text.contains(r#"organizationRateLimitTier = "default_claude_max_5x""#),
            "{text}"
        );
        assert!(text.contains("billingType = absent"), "{text}");
        assert!(
            text.contains("raw: spend limit 500.00 USD, used 12.34 USD"),
            "{text}"
        );
        assert!(text.contains("monthly: $100.00"), "{text}");
    }

    #[test]
    fn a_raw_plan_value_that_is_not_a_plain_tier_name_is_hidden() {
        let body = r#"{"oauthAccount":{"organizationType":"Acme Corp (id 7)","seatTier":{"name":"x"},"billingType":"stripe_subscription"}}"#;
        let home = Home::new("odd", body);

        let text = report_of(&home, NOW, true);

        assert!(text.contains("organizationType = <other value>"), "{text}");
        assert!(text.contains("seatTier = <other value>"), "{text}");
        assert!(
            text.contains(r#"billingType = "stripe_subscription""#),
            "{text}"
        );
        assert!(!text.contains("Acme"), "{text}");
    }

    #[test]
    fn a_window_past_its_reset_says_so() {
        let home = Home::new("past", &cache_body(NOW * 1000, "null"));

        let text = report_of(&home, NOW + 5 * 3600, false);

        assert!(
            text.contains("five_hour: present, its reset was 1h 46m ago"),
            "{text}"
        );
        assert!(!text.contains("% used"), "{text}");
    }

    #[test]
    fn a_window_that_does_not_parse_is_present_but_not_readable() {
        let body = r#"{"cachedUsageUtilization":{"fetchedAtMs":1788400000000,"utilization":{"five_hour":{"utilization":"lots"}}}}"#;
        let home = Home::new("unreadable-window", body);

        let text = report_of(&home, NOW, false);

        assert!(
            text.contains("five_hour: present but not readable"),
            "{text}"
        );
    }

    #[test]
    fn a_cache_without_a_fetch_time_is_found_but_not_readable() {
        let body = r#"{"cachedUsageUtilization":{"utilization":{"five_hour":null}}}"#;
        let home = Home::new("no-time", body);

        let text = report_of(&home, NOW, false);

        assert!(
            text.contains("found but not readable (fields: utilization)"),
            "{text}"
        );
    }

    #[test]
    fn a_fetch_time_in_the_future_is_called_out() {
        let home = Home::new("future", &cache_body((NOW + 7200) * 1000, "null"));

        let text = report_of(&home, NOW, false);

        assert!(
            text.contains("its time is in the future, so it is not trusted"),
            "{text}"
        );
    }

    #[test]
    fn each_spend_state_has_its_own_line() {
        let cases = [
            (
                "on",
                r#"{"enabled":true,"limit":{"amount_minor":100,"currency":"USD","exponent":2}}"#,
                "present, in dollars",
            ),
            (
                "off",
                r#"{"enabled":false,"limit":{"amount_minor":100,"currency":"USD","exponent":2}}"#,
                "the cache says it is off",
            ),
            (
                "eur",
                r#"{"enabled":true,"limit":{"amount_minor":100,"currency":"EUR","exponent":2}}"#,
                "present but not usable",
            ),
        ];

        for (name, spend, expected) in cases {
            let home = Home::new(&format!("spend-{name}"), &cache_body(NOW * 1000, spend));

            let text = report_of(&home, NOW, false);

            assert!(
                text.contains(&format!("monthly spend limit: {expected}")),
                "{name}: {text}"
            );
        }
    }

    #[test]
    fn a_statusline_reading_is_listed_with_its_age_and_windows() {
        let home = Home::new("statusline", &cache_body(NOW * 1000, "null"));
        let snap = Snapshot {
            captured_at: NOW - 600,
            five_hour: Some(Window {
                used_percentage: 10.0,
                resets_at: NOW + 3600,
            }),
            ..Snapshot::default()
        };
        limits_source::save(&home.0, &snap);

        let text = report_of(&home, NOW, false);

        assert!(
            text.contains("Statusline reading (rate_limits.json): found, 10m old (fresh)"),
            "{text}"
        );
        assert!(text.contains("five_hour: present"), "{text}");
        assert!(text.contains("spend_limit: absent"), "{text}");
    }

    #[test]
    fn the_report_ends_with_what_pbtop_will_show() {
        let spend =
            r#"{"enabled":true,"limit":{"amount_minor":50000,"currency":"USD","exponent":2}}"#;
        let home = Home::new("verdict", &cache_body((NOW - 600) * 1000, spend));

        let text = report_of(&home, NOW, false);

        assert!(text.contains("What pbtop will show"), "{text}");
        assert!(
            text.contains("5 hour window: from the Claude Code cache"),
            "{text}"
        );
        assert!(text.contains("weekly window: not known"), "{text}");
        assert!(
            text.contains("monthly limit: $500.00 from the Claude Code cache, 10m old"),
            "{text}"
        );
    }

    #[test]
    fn an_unknown_plan_with_no_windows_says_not_available() {
        let home = Home::new(
            "verdict-enterprise",
            r#"{"oauthAccount":{"organizationType":"x"}}"#,
        );

        let text = report(
            &home.json(),
            &home.0,
            &[(LimitPeriod::Monthly, 80.0)],
            NOW,
            false,
        );

        assert!(text.contains("5 hour window: not known"), "{text}");
        assert!(text.contains("not available for this plan"), "{text}");
        assert!(
            text.contains("monthly limit: $80.00 from usage.budget.monthlyUsd"),
            "{text}"
        );
    }

    #[test]
    fn a_missing_config_file_is_reported_without_failing() {
        let dir = std::env::temp_dir().join(format!("pb-plan-{}-none", std::process::id()));

        let text = report(&dir.join(".claude.json"), &dir, &[], NOW, false);

        assert!(text.contains("Claude config file: not found"), "{text}");
        assert!(
            text.contains("Statusline reading (rate_limits.json): not found"),
            "{text}"
        );
    }

    #[test]
    fn a_stale_cache_and_no_budget_say_so_in_the_verdict() {
        let spend =
            r#"{"enabled":true,"limit":{"amount_minor":50000,"currency":"USD","exponent":2}}"#;
        let stale = Home::new(
            "verdict-stale",
            &cache_body((NOW - 48 * 3600) * 1000, spend),
        );
        let none = Home::new("verdict-none", &cache_body(NOW * 1000, "null"));

        let stale_text = report_of(&stale, NOW, false);
        let none_text = report_of(&none, NOW, false);

        assert!(
            stale_text.contains("$500.00 from the Claude Code cache, 2d 0h old (stale)"),
            "{stale_text}"
        );
        assert!(
            none_text.contains("monthly limit: none (set usage.budget.monthlyUsd)"),
            "{none_text}"
        );
    }

    #[test]
    fn a_statusline_monthly_limit_is_named_in_the_verdict() {
        let home = Home::new("verdict-measured", &cache_body(NOW * 1000, "null"));
        let snap = Snapshot {
            captured_at: NOW - 60,
            spend_limit: Some(limits_source::Spend {
                used_percentage: 10.0,
                resets_at: NOW + 86_400,
                used_usd: Some(10.0),
                limit_usd: Some(100.0),
                period: Some(LimitPeriod::Monthly),
            }),
            ..Snapshot::default()
        };
        limits_source::save(&home.0, &snap);

        let text = report_of(&home, NOW, false);

        assert!(
            text.contains("monthly limit: from the statusline reading"),
            "{text}"
        );
    }

    #[test]
    fn a_file_that_stops_parsing_keeps_the_last_good_answer() {
        let home = Home::new("cached-read", &cache_body(NOW * 1000, "null"));
        let first = read_cached(&home.json());

        std::fs::write(home.json(), "{ half a file").unwrap();
        let during = read_cached(&home.json());
        std::fs::write(
            home.json(),
            r#"{"oauthAccount":{"organizationRateLimitTier":"default_claude_max_20x"}}"#,
        )
        .unwrap();
        let after = read_cached(&home.json());

        assert_eq!(first.plan, Plan::Max5x);
        assert_eq!(during, first);
        assert_eq!(after.plan, Plan::Max20x);
    }

    #[test]
    fn a_missing_file_reads_as_unknown_through_the_cache_too() {
        let missing =
            std::env::temp_dir().join(format!("pb-plan-{}-gone/.claude.json", std::process::id()));

        assert_eq!(read_cached(&missing), Account::default());
    }
}
