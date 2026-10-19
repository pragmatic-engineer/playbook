// SPDX-FileCopyrightText: 2026 Igor Santos
// SPDX-License-Identifier: Apache-2.0

//! The server side spend limit: the amount the server billed against the
//! monthly limit an account has. The figure comes from one JSON document whose
//! shape Anthropic has not published, so the parser is tolerant. It reads only
//! the fields it knows, ignores the rest, treats `null` as absent and never
//! panics on odd input. Nothing here touches the network or a credential.
//!
//! Money carries its own number of decimals and currency from the document.
//! It is never assumed to be dollars: a figure is compared with the local
//! estimate, and shown as a dollar limit, only when the currency says USD.

use super::limits::Origin;
use super::limits_source::{Window, STALE_SECONDS};
use super::plan::{self, CLOCK_SKEW_SECONDS};
use serde_json::{Map, Value};
use std::fmt;

/// The most decimal places an amount may carry. More is rejected, so a hostile
/// document cannot ask for a power of ten that overflows.
pub const MAX_DECIMALS: u8 = 6;

/// The largest amount in minor units that is read. It sits well below 2^53, so
/// the conversion to `f64` is exact for every accepted amount.
const MAX_MINOR_UNITS: f64 = 1e15;

/// The currency code of dollars.
pub const USD: &str = "USD";

/// The longest currency code that is kept. Anything longer or with other
/// characters than letters and digits is treated as absent, so a hostile
/// document cannot put text into a label.
const MAX_CURRENCY_LEN: usize = 8;

/// A difference below this many dollars is never a disagreement.
pub const DISAGREE_MIN_USD: f64 = 1.0;

/// A difference below this share of the billed amount, in percent, is never a
/// disagreement.
pub const DISAGREE_PCT: f64 = 10.0;

const PERCENT: f64 = 100.0;

/// Top level keys that are not usage windows.
const NON_WINDOW_KEYS: [&str; 2] = ["extra_usage", "spend"];

/// A dollar amount for a sentence: `$1,059.50`.
pub fn dollars(n: f64) -> String {
    let text = format!("{:.2}", n.abs());
    let (whole, cents) = text.split_once('.').unwrap_or((&text, "00"));
    let mut grouped = String::new();
    for (i, digit) in whole.chars().enumerate() {
        if i > 0 && (whole.len() - i) % GROUP == 0 {
            grouped.push(',');
        }
        grouped.push(digit);
    }
    let sign = if n < 0.0 { "-" } else { "" };
    format!("{sign}${grouped}.{cents}")
}

/// Digits between thousands separators.
const GROUP: usize = 3;

/// Why no server figure is shown. The words are the ones the output uses.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Reason {
    Disabled,
    NoConsent,
    NoToken,
    Unauthorized,
    RateLimited,
    Timeout,
    Schema,
    UnsupportedLogin,
    Network,
    Internal,
    /// A fetch is under way and has not answered.
    Fetching,
}

impl Reason {
    pub fn key(self) -> &'static str {
        match self {
            Reason::Disabled => "disabled",
            Reason::NoConsent => "no-consent",
            Reason::NoToken => "no-token",
            Reason::Unauthorized => "unauthorized",
            Reason::RateLimited => "rate-limited",
            Reason::Timeout => "timeout",
            Reason::Schema => "schema",
            Reason::UnsupportedLogin => "unsupported-login",
            Reason::Network => "network",
            Reason::Internal => "internal",
            Reason::Fetching => "fetching",
        }
    }

    /// The one line that stands where the server figure would be.
    pub fn line(self) -> String {
        match self {
            Reason::NoConsent => {
                "spend limit unavailable (run `playbook usage spend enable`)".to_string()
            }
            other => format!("spend limit unavailable ({})", other.key()),
        }
    }
}

/// Why a document gave no figure.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SpendError {
    /// Not JSON, not an object, or a number that cannot be right.
    Schema,
}

/// What a well formed document held.
#[derive(Debug, Clone, PartialEq)]
pub enum Parsed {
    Figure(SpendFigure),
    /// Valid, but no limit and no window: "no spend limit set on this account".
    Empty,
}

/// An amount in minor units with the decimals and currency the document gave.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Money {
    pub minor: i64,
    pub decimals: u8,
    pub currency: Option<String>,
}

impl Money {
    pub fn to_f64(&self) -> f64 {
        self.minor as f64 / 10f64.powi(i32::from(self.decimals))
    }

    /// Whether the document said this is dollars.
    pub fn is_usd(&self) -> bool {
        self.currency
            .as_deref()
            .is_some_and(|c| c.eq_ignore_ascii_case(USD))
    }

    /// The amount as dollars, only for USD.
    pub fn usd(&self) -> Option<f64> {
        self.is_usd().then(|| self.to_f64())
    }

    /// The amount with exactly `decimals` places and no currency sign, as
    /// `1059.50`.
    pub fn format(&self) -> String {
        let decimals = self.decimals.min(MAX_DECIMALS);
        let sign = if self.minor < 0 { "-" } else { "" };
        let units = self.minor.unsigned_abs();
        if decimals == 0 {
            return format!("{sign}{units}");
        }
        let scale = 10u64.pow(u32::from(decimals));
        format!(
            "{sign}{}.{:0width$}",
            units / scale,
            units % scale,
            width = usize::from(decimals)
        )
    }
}

/// Whose spend the figure counts, when the document says. The field name is
/// not documented, so a document that does not carry one is `Unspecified` and
/// the label says "as reported by the server".
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Scope {
    Member,
    Org,
    Unspecified,
}

impl Scope {
    pub fn key(self) -> &'static str {
        match self {
            Scope::Member => "member",
            Scope::Org => "org",
            Scope::Unspecified => "unspecified",
        }
    }

    fn parse(v: Option<&Value>) -> Scope {
        match v
            .and_then(Value::as_str)
            .map(str::to_ascii_lowercase)
            .as_deref()
        {
            Some("member") => Scope::Member,
            Some("org") => Scope::Org,
            _ => Scope::Unspecified,
        }
    }
}

/// The server's view of the monthly spend limit at one instant.
#[derive(Debug, Clone, PartialEq)]
pub struct SpendFigure {
    pub limit: Option<Money>,
    pub used: Option<Money>,
    pub percent: Option<f64>,
    pub severity: Option<String>,
    /// When the limit resets, UTC seconds, if the document said.
    pub resets_at: Option<i64>,
    pub limit_reached: Option<bool>,
    pub scope: Scope,
    /// Every top level usage window, by the document's own name, sorted by it.
    pub windows: Vec<(String, Window)>,
    /// When the figure was fetched, UTC seconds.
    pub fetched_at: i64,
    pub origin: Origin,
}

impl SpendFigure {
    /// The limit in dollars, when it is one.
    pub fn limit_usd(&self) -> Option<f64> {
        self.limit
            .as_ref()
            .and_then(Money::usd)
            .filter(|n| *n > 0.0)
    }

    /// The amount used in dollars, when it is dollars.
    pub fn used_usd(&self) -> Option<f64> {
        self.used.as_ref().and_then(Money::usd)
    }

    /// Seconds since the fetch, never negative.
    pub fn age(&self, now: i64) -> i64 {
        (now - self.fetched_at).max(0)
    }

    /// Whether the fetch time is later than `now` by more than a clock can
    /// drift. Such a time is wrong, so the figure is never called fresh.
    pub fn is_from_the_future(&self, now: i64) -> bool {
        self.fetched_at > now + CLOCK_SKEW_SECONDS
    }

    /// A figure older than six hours is stale, whatever its origin.
    pub fn is_fresh(&self, now: i64) -> bool {
        !self.is_from_the_future(now) && self.age(now) <= STALE_SECONDS
    }
}

/// A number from a JSON value. Absent for a missing key, `null`, a string that
/// is not a number and any other type. A number that is negative, not finite or
/// larger than any amount can be is a schema error, since the whole document
/// cannot be trusted then.
fn number(v: Option<&Value>) -> Result<Option<f64>, SpendError> {
    let n = match v {
        Some(Value::Number(n)) => n.as_f64(),
        Some(Value::String(s)) => s.trim().parse::<f64>().ok(),
        _ => None,
    };
    match n {
        None => Ok(None),
        Some(n) if n.is_finite() && (0.0..=MAX_MINOR_UNITS).contains(&n) => Ok(Some(n)),
        Some(_) => Err(SpendError::Schema),
    }
}

fn decimals(v: Option<&Value>) -> Result<Option<u8>, SpendError> {
    match number(v)? {
        None => Ok(None),
        Some(d) if d.fract() == 0.0 && d <= f64::from(MAX_DECIMALS) => Ok(Some(d as u8)),
        Some(_) => Err(SpendError::Schema),
    }
}

fn text(v: Option<&Value>) -> Option<String> {
    v.and_then(Value::as_str)
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_string)
}

/// A reset time as UTC seconds, from an integer or an ISO 8601 string.
fn time(v: Option<&Value>) -> Option<i64> {
    match v? {
        Value::Number(n) => n.as_i64(),
        Value::String(s) => plan::parse_time(s),
        _ => None,
    }
}

fn object<'a>(root: &'a Map<String, Value>, key: &str) -> Option<&'a Map<String, Value>> {
    root.get(key).and_then(Value::as_object)
}

/// Reads one document. `fetched_at` is when it was fetched.
pub fn parse(body: &str, fetched_at: i64) -> Result<Parsed, SpendError> {
    let root: Value = serde_json::from_str(body).map_err(|_| SpendError::Schema)?;
    let root = root.as_object().ok_or(SpendError::Schema)?;
    let extra = object(root, "extra_usage");
    let field = |key: &str| extra.and_then(|e| e.get(key));
    let spend = object(root, "spend");

    let places = decimals(field("decimal_places"))?;
    let currency = text(field("currency"))
        .filter(|c| c.len() <= MAX_CURRENCY_LEN && c.chars().all(|ch| ch.is_ascii_alphanumeric()));
    let money = |key: &str| -> Result<Option<Money>, SpendError> {
        Ok(number(field(key))?
            .zip(places)
            .map(|(minor, decimals)| Money {
                minor: minor.round() as i64,
                decimals,
                currency: currency.clone(),
            }))
    };
    let limit = money("monthly_limit")?;
    let used = money("used_credits")?;
    let percent = match number(field("utilization"))? {
        Some(p) => Some(p),
        None => number(spend.and_then(|s| s.get("percent")))?,
    };

    let mut windows: Vec<(String, Window)> = root
        .iter()
        .filter(|(key, _)| !NON_WINDOW_KEYS.contains(&key.as_str()))
        .filter_map(|(key, v)| plan::window(Some(v)).map(|w| (key.clone(), w)))
        .collect();
    windows.sort_by(|a, b| a.0.cmp(&b.0));

    if limit.is_none() && windows.is_empty() {
        return Ok(Parsed::Empty);
    }
    Ok(Parsed::Figure(SpendFigure {
        limit,
        used,
        percent,
        severity: text(spend.and_then(|s| s.get("severity"))),
        resets_at: time(field("resets_at")),
        limit_reached: root.get("spend_limit_reached").and_then(Value::as_bool),
        scope: match Scope::parse(root.get("scope")) {
            Scope::Unspecified => Scope::parse(field("scope")),
            stated => stated,
        },
        windows,
        fetched_at,
        origin: Origin::Server,
    }))
}

/// Why a figure was not compared with the local estimate.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum NotCompared {
    /// The figure is in this currency, not dollars.
    Currency(String),
    /// The document gave no currency.
    CurrencyNotReported,
    /// The figure has no amount used.
    UsedUnknown,
}

impl fmt::Display for NotCompared {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            NotCompared::Currency(c) => write!(f, "currency {c}, not compared"),
            NotCompared::CurrencyNotReported => write!(f, "currency not reported, not compared"),
            NotCompared::UsedUnknown => write!(f, "amount used not reported, not compared"),
        }
    }
}

/// The amount the server billed next to the local estimate for the same period.
#[derive(Debug, Clone, PartialEq)]
pub struct Comparison {
    pub billed: SpendFigure,
    pub billed_usd: f64,
    pub local_estimate_usd: f64,
    /// Local minus billed. Positive when this machine's estimate is larger.
    pub delta_usd: f64,
    /// The delta as a percent of the billed amount, `None` when nothing was
    /// billed.
    pub delta_pct: Option<f64>,
    pub disagrees: bool,
}

impl Comparison {
    /// Compares the billed amount with `local_usd`. The two disagree when they
    /// differ by at least the larger of [`DISAGREE_MIN_USD`] and
    /// [`DISAGREE_PCT`] of the billed amount.
    pub fn new(billed: SpendFigure, local_usd: f64) -> Result<Comparison, NotCompared> {
        let used = billed.used.as_ref().ok_or(NotCompared::UsedUnknown)?;
        let billed_usd = match (used.usd(), used.currency.as_deref()) {
            (Some(usd), _) => usd,
            (None, Some(other)) => return Err(NotCompared::Currency(other.to_string())),
            (None, None) => return Err(NotCompared::CurrencyNotReported),
        };
        let delta_usd = local_usd - billed_usd;
        let floor = DISAGREE_MIN_USD.max(billed_usd * DISAGREE_PCT / PERCENT);
        Ok(Comparison {
            delta_pct: (billed_usd > 0.0).then(|| delta_usd / billed_usd * PERCENT),
            disagrees: delta_usd.abs() >= floor,
            billed,
            billed_usd,
            local_estimate_usd: local_usd,
            delta_usd,
        })
    }

    /// One sentence with both numbers. A difference is "differs", never
    /// "wrong": another machine, another seat or an unpriced model explains it.
    pub fn line(&self) -> String {
        if self.disagrees {
            format!(
                "local estimate (this machine only) {} differs from server billed {}",
                dollars(self.local_estimate_usd),
                dollars(self.billed_usd)
            )
        } else {
            format!(
                "local estimate (this machine only) {} is close to server billed {}",
                dollars(self.local_estimate_usd),
                dollars(self.billed_usd)
            )
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    const NOW: i64 = 1_788_400_000;

    fn fixture(name: &str) -> String {
        let path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("tests/fixtures/spend")
            .join(format!("{name}.json"));
        std::fs::read_to_string(path).unwrap()
    }

    fn figure(name: &str) -> SpendFigure {
        match parse(&fixture(name), NOW) {
            Ok(Parsed::Figure(f)) => f,
            other => panic!("{name}: expected a figure, got {other:?}"),
        }
    }

    fn billed(used: Option<Money>) -> SpendFigure {
        SpendFigure {
            used,
            ..figure("ok")
        }
    }

    fn usd(minor: i64) -> Money {
        Money {
            minor,
            decimals: 2,
            currency: Some(USD.to_string()),
        }
    }

    #[test]
    fn the_ok_fixture_gives_limit_used_and_percent() {
        let f = figure("ok");

        assert_eq!(f.limit.as_ref().unwrap().format(), "1500.00");
        assert_eq!(f.used.as_ref().unwrap().format(), "1059.50");
        assert_eq!(f.limit_usd(), Some(1500.0));
        assert_eq!(f.used_usd(), Some(1059.5));
        assert_eq!(f.percent, Some(70.6));
        assert_eq!(f.severity.as_deref(), Some("warning"));
        assert_eq!(f.limit_reached, Some(false));
        assert_eq!(f.fetched_at, NOW);
        assert_eq!(f.origin, Origin::Server);
    }

    #[test]
    fn windows_come_with_their_own_names_sorted() {
        let f = figure("ok");

        let names: Vec<_> = f.windows.iter().map(|(n, _)| n.as_str()).collect();
        assert_eq!(names, ["five_hour", "seven_day"]);
        assert_eq!(f.windows[0].1.used_percentage, 12.5);
    }

    #[test]
    fn null_windows_are_absent_and_not_an_error() {
        let f = figure("nulls");

        assert!(f.windows.is_empty());
        assert_eq!(f.limit_usd(), Some(1500.0));
    }

    #[test]
    fn a_partial_document_leaves_used_and_percent_unknown() {
        let f = figure("partial");

        assert_eq!(f.limit_usd(), Some(1500.0));
        assert!(f.used.is_none());
        assert!(f.percent.is_none());
    }

    #[test]
    fn unknown_fields_change_nothing() {
        assert_eq!(figure("unknown-fields"), figure("ok"));
    }

    #[test]
    fn numeric_strings_parse_like_numbers_and_other_strings_are_absent() {
        let f = figure("string-numbers");

        assert_eq!(f.limit, figure("ok").limit);
        assert_eq!(f.used, figure("ok").used);
        assert_eq!(f.percent, Some(70.6));
    }

    #[test]
    fn a_non_numeric_string_is_absent() {
        let body =
            r#"{"extra_usage":{"monthly_limit":150000,"used_credits":"lots","decimal_places":2}}"#;

        let Ok(Parsed::Figure(f)) = parse(body, NOW) else {
            panic!("expected a figure");
        };

        assert!(f.used.is_none());
    }

    #[test]
    fn an_empty_object_is_empty() {
        assert_eq!(parse(&fixture("empty-object"), NOW), Ok(Parsed::Empty));
        assert_eq!(
            parse(r#"{"spend_limit_reached":false}"#, NOW),
            Ok(Parsed::Empty)
        );
    }

    #[test]
    fn garbage_is_a_schema_error_and_never_a_panic() {
        let truncated = &fixture("ok")[..40];
        let huge = r#"{"extra_usage":{"monthly_limit":1e300,"decimal_places":2}}"#;
        let negative = r#"{"extra_usage":{"monthly_limit":-5,"decimal_places":2}}"#;

        for body in [
            fixture("garbage").as_str(),
            truncated,
            "[1,2,3]",
            "null",
            "42",
            "",
            huge,
            negative,
        ] {
            assert_eq!(parse(body, NOW), Err(SpendError::Schema), "{body}");
        }
    }

    #[test]
    fn decimals_beyond_the_cap_are_rejected() {
        assert_eq!(
            parse(&fixture("bad-decimals"), NOW),
            Err(SpendError::Schema)
        );
        for places in ["7", "2.5", "-1"] {
            let body =
                format!(r#"{{"extra_usage":{{"monthly_limit":1,"decimal_places":{places}}}}}"#);
            assert_eq!(parse(&body, NOW), Err(SpendError::Schema), "{places}");
        }
    }

    #[test]
    fn amounts_without_decimals_are_absent() {
        let body = r#"{"extra_usage":{"monthly_limit":150000},"five_hour":{"utilization":1,"resets_at":"2026-09-03T05:00:00Z"}}"#;

        let Ok(Parsed::Figure(f)) = parse(body, NOW) else {
            panic!("expected a figure");
        };

        assert!(f.limit.is_none());
        assert_eq!(f.windows.len(), 1);
    }

    #[test]
    fn money_formats_with_its_decimals() {
        let m = |minor, decimals| Money {
            minor,
            decimals,
            currency: None,
        };

        assert_eq!(m(105_950, 2).format(), "1059.50");
        assert_eq!(m(5, 2).format(), "0.05");
        assert_eq!(m(7, 0).format(), "7");
        assert_eq!(m(1_234_567, 6).format(), "1.234567");
        assert_eq!(m(105_950, 2).to_f64(), 1059.5);
    }

    #[test]
    fn money_never_panics_or_misplaces_a_sign_when_built_by_hand() {
        let m = |minor, decimals| Money {
            minor,
            decimals,
            currency: None,
        };

        assert_eq!(m(-50, 2).format(), "-0.50");
        assert_eq!(m(-105_950, 2).format(), "-1059.50");
        assert_eq!(m(i64::MIN, 2).format(), "-92233720368547758.08");
        assert_eq!(m(1_234_567, 200).format(), "1.234567");
    }

    #[test]
    fn the_spend_percent_is_used_when_utilization_is_missing() {
        let with =
            r#"{"extra_usage":{"monthly_limit":1,"decimal_places":2},"spend":{"percent":71}}"#;
        let junk =
            r#"{"extra_usage":{"monthly_limit":1,"decimal_places":2},"spend":{"percent":"lots"}}"#;

        let Ok(Parsed::Figure(a)) = parse(with, NOW) else {
            panic!("expected a figure");
        };
        let Ok(Parsed::Figure(b)) = parse(junk, NOW) else {
            panic!("expected a figure");
        };

        assert_eq!(a.percent, Some(71.0));
        assert_eq!(b.percent, None);
    }

    #[test]
    fn the_reset_time_reads_as_iso_text_or_whole_seconds() {
        let iso = r#"{"extra_usage":{"monthly_limit":1,"decimal_places":2,"resets_at":"2026-10-01T00:00:00Z"}}"#;
        let secs =
            r#"{"extra_usage":{"monthly_limit":1,"decimal_places":2,"resets_at":1790000000}}"#;
        let junk = r#"{"extra_usage":{"monthly_limit":1,"decimal_places":2,"resets_at":"soon"}}"#;
        let resets = |body: &str| match parse(body, NOW) {
            Ok(Parsed::Figure(f)) => f.resets_at,
            other => panic!("expected a figure, got {other:?}"),
        };

        assert_eq!(resets(iso), Some(1_790_812_800));
        assert_eq!(resets(secs), Some(1_790_000_000));
        assert_eq!(resets(junk), None);
    }

    #[test]
    fn null_money_fields_and_a_null_currency_are_absent() {
        let body = r#"{"extra_usage":{"monthly_limit":150000,"used_credits":null,"decimal_places":2,"currency":null}}"#;

        let Ok(Parsed::Figure(f)) = parse(body, NOW) else {
            panic!("expected a figure");
        };

        assert!(f.used.is_none());
        assert_eq!(f.limit_usd(), None);
        assert_eq!(f.limit.unwrap().currency, None);
    }

    #[test]
    fn a_currency_with_other_than_letters_and_digits_or_too_long_is_absent() {
        for code in ["US D", "\\u001b[31mUSD", "ABCDEFGHI", ""] {
            let body = format!(
                r#"{{"extra_usage":{{"monthly_limit":1,"decimal_places":2,"currency":"{code}"}}}}"#
            );

            let Ok(Parsed::Figure(f)) = parse(&body, NOW) else {
                panic!("expected a figure for {code}");
            };

            assert_eq!(f.limit.unwrap().currency, None, "{code}");
        }
    }

    #[test]
    fn only_usd_converts_to_dollars() {
        let eur = Money {
            currency: Some("EUR".to_string()),
            ..usd(100)
        };
        let none = Money {
            currency: None,
            ..usd(100)
        };

        assert_eq!(usd(100).usd(), Some(1.0));
        assert_eq!(eur.usd(), None);
        assert_eq!(none.usd(), None);
    }

    #[test]
    fn a_figure_is_stale_after_six_hours_and_never_fresh_from_the_future() {
        let f = figure("ok");

        assert!(f.is_fresh(NOW + STALE_SECONDS));
        assert!(!f.is_fresh(NOW + STALE_SECONDS + 1));
        assert!(!f.is_fresh(NOW - CLOCK_SKEW_SECONDS - 1));
        assert_eq!(f.age(NOW - 10), 0);
    }

    #[test]
    fn disagreement_follows_the_boundary_table() {
        let table: [(f64, f64, bool); 11] = [
            (0.0, 0.0, false),
            (0.0, 0.5, false),
            (0.0, 1.0, true),
            (10.0, 10.99, false),
            (10.0, 11.0, true),
            (100.0, 109.99, false),
            (100.0, 110.0, true),
            (1059.5, 900.0, true),
            (1059.5, 1055.0, false),
            (100.0, 80.0, true),
            (100.0, 95.0, false),
        ];

        for (billed_usd, local, disagrees) in table {
            let minor = (billed_usd * 100.0).round() as i64;

            let c = Comparison::new(billed(Some(usd(minor))), local).unwrap();

            assert_eq!(c.disagrees, disagrees, "billed {billed_usd} local {local}");
        }
    }

    #[test]
    fn the_comparison_keeps_the_delta_and_its_percent() {
        let c = Comparison::new(billed(Some(usd(105_950))), 900.0).unwrap();

        assert_eq!(c.delta_usd, 900.0 - 1059.5);
        assert!((c.delta_pct.unwrap() - (-15.054)).abs() < 0.001);
        assert!(Comparison::new(billed(Some(usd(0))), 0.5)
            .unwrap()
            .delta_pct
            .is_none());
    }

    #[test]
    fn another_currency_or_none_skips_the_comparison() {
        let eur = Money {
            currency: Some("EUR".to_string()),
            ..usd(105_950)
        };
        let none = Money {
            currency: None,
            ..usd(105_950)
        };

        let a = Comparison::new(billed(Some(eur)), 1.0).unwrap_err();
        let b = Comparison::new(billed(Some(none)), 1.0).unwrap_err();
        let c = Comparison::new(billed(None), 1.0).unwrap_err();

        assert_eq!(a.to_string(), "currency EUR, not compared");
        assert_eq!(b.to_string(), "currency not reported, not compared");
        assert_eq!(c, NotCompared::UsedUnknown);
    }

    #[test]
    fn dollars_group_thousands_and_keep_the_sign() {
        let table = [
            (0.0, "$0.00"),
            (5.5, "$5.50"),
            (999.99, "$999.99"),
            (1000.0, "$1,000.00"),
            (1059.5, "$1,059.50"),
            (1_234_567.891, "$1,234,567.89"),
            (-1234.5, "-$1,234.50"),
        ];

        for (n, want) in table {
            assert_eq!(dollars(n), want, "{n}");
        }
    }

    #[test]
    fn every_reason_has_a_distinct_key_and_one_line() {
        let all = [
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
        ];
        let keys: std::collections::HashSet<_> = all.iter().map(|r| r.key()).collect();

        assert_eq!(keys.len(), all.len());
        for r in all {
            assert!(r.line().starts_with("spend limit unavailable ("), "{r:?}");
            assert!(!r.line().contains('\n'));
        }
        assert_eq!(
            Reason::NoConsent.line(),
            "spend limit unavailable (run `playbook usage spend enable`)"
        );
        assert_eq!(
            Reason::Fetching.line(),
            "spend limit unavailable (fetching)"
        );
    }

    #[test]
    fn the_line_says_differs_with_both_numbers_and_never_wrong() {
        let far = Comparison::new(billed(Some(usd(105_950))), 900.0).unwrap();
        let near = Comparison::new(billed(Some(usd(105_950))), 1055.0).unwrap();

        assert!(far.line().contains("differs"));
        assert!(far.line().contains("$900.00") && far.line().contains("$1,059.50"));
        assert!(!far.line().contains("wrong"));
        assert!(near.line().contains("close to"));
    }
}
