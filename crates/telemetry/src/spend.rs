// SPDX-FileCopyrightText: 2026 Igor Santos
// SPDX-License-Identifier: Apache-2.0

//! The dollar figures Claude Code itself reports for a spend limit: the
//! `spend` block of the usage cache it keeps in `~/.claude.json`, and the
//! `spend_limit` of the statusline reading. Nothing here touches the network
//! or a credential, and no setting defines a limit.
//!
//! Money carries its own number of decimals and currency from the source. It
//! is never assumed to be dollars: a figure is compared with the local
//! estimate, and shown as a dollar limit, only when the currency says USD.

use serde_json::Value;
use std::fmt;

/// The most decimal places an amount may carry. More is rejected, so a hostile
/// document cannot ask for a power of ten that overflows.
pub const MAX_DECIMALS: u8 = 6;

/// The largest amount in minor units that is read. It sits well below 2^53, so
/// the conversion to `f64` is exact for every accepted amount.
const MAX_MINOR_UNITS: f64 = 1e15;

/// The currency code of dollars.
const USD: &str = "USD";

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

/// An amount in minor units with the decimals and currency the document gave.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Money {
    pub minor: i64,
    pub decimals: u8,
    pub currency: Option<String>,
}

impl Money {
    /// Reads `{amount_minor, currency, exponent}`. `None` unless the amount is
    /// a whole, non-negative number of minor units and the exponent is a whole
    /// number from 0 to [`MAX_DECIMALS`]. A missing exponent or a fractional
    /// amount is a bad figure, never a guess. A currency that is missing or not
    /// plain letters and digits is left out.
    pub fn from_json(v: Option<&Value>) -> Option<Money> {
        let v = v?;
        let minor = v.get("amount_minor")?.as_f64()?;
        if !minor.is_finite() || minor.fract() != 0.0 || !(0.0..=MAX_MINOR_UNITS).contains(&minor) {
            return None;
        }
        let exponent = v.get("exponent")?.as_i64()?;
        let decimals = u8::try_from(exponent).ok().filter(|d| *d <= MAX_DECIMALS)?;
        let currency = v
            .get("currency")
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|c| {
                !c.is_empty()
                    && c.len() <= MAX_CURRENCY_LEN
                    && c.chars().all(|ch| ch.is_ascii_alphanumeric())
            })
            .map(str::to_string);
        Some(Money {
            minor: minor as i64,
            decimals,
            currency,
        })
    }

    /// Dollars to cents, for the statusline figure, which is plain dollars.
    pub fn from_usd(usd: f64) -> Option<Money> {
        let cents = (usd * PERCENT).round();
        (usd.is_finite() && (0.0..=MAX_MINOR_UNITS).contains(&cents)).then(|| Money {
            minor: cents as i64,
            decimals: 2,
            currency: Some(USD.to_string()),
        })
    }

    pub fn to_f64(&self) -> f64 {
        self.minor as f64 / 10f64.powi(i32::from(self.decimals.min(MAX_DECIMALS)))
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

/// The spend limit and the amount used at one instant, as Claude Code
/// reported them.
#[derive(Debug, Clone, PartialEq)]
pub struct SpendFigure {
    pub limit: Option<Money>,
    pub used: Option<Money>,
    /// When the figure was read, UTC seconds.
    pub fetched_at: i64,
}

impl SpendFigure {
    /// From the `spend` block of Claude Code's usage cache. `None` when the
    /// block says it is off or holds neither amount.
    pub fn from_cache(v: &Value, fetched_at: i64) -> Option<SpendFigure> {
        if v.get("enabled").and_then(Value::as_bool) == Some(false) {
            return None;
        }
        let limit = Money::from_json(v.get("limit"));
        let used = Money::from_json(v.get("used"));
        (limit.is_some() || used.is_some()).then_some(SpendFigure {
            limit,
            used,
            fetched_at,
        })
    }

    /// From the statusline `spend_limit`, whose dollars are plain numbers.
    pub fn from_usd(limit: Option<f64>, used: Option<f64>, fetched_at: i64) -> Option<SpendFigure> {
        let limit = limit.and_then(Money::from_usd);
        let used = used.and_then(Money::from_usd);
        (limit.is_some() || used.is_some()).then_some(SpendFigure {
            limit,
            used,
            fetched_at,
        })
    }

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
            NotCompared::CurrencyNotReported => {
                write!(f, "currency not reported or not readable, not compared")
            }
            NotCompared::UsedUnknown => write!(f, "amount used not reported, not compared"),
        }
    }
}

/// The amount Claude Code reports as used next to the local estimate for the same period.
#[derive(Debug, Clone, PartialEq)]
pub struct BilledComparison {
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

impl BilledComparison {
    /// Compares the billed amount with `local_usd`. The two disagree when they
    /// differ by at least the larger of [`DISAGREE_MIN_USD`] and
    /// [`DISAGREE_PCT`] of the billed amount.
    pub fn new(billed: SpendFigure, local_usd: f64) -> Result<BilledComparison, NotCompared> {
        let used = billed.used.as_ref().ok_or(NotCompared::UsedUnknown)?;
        let billed_usd = match (used.usd(), used.currency.as_deref()) {
            (Some(usd), _) => usd,
            (None, Some(other)) => return Err(NotCompared::Currency(other.to_string())),
            (None, None) => return Err(NotCompared::CurrencyNotReported),
        };
        let delta_usd = local_usd - billed_usd;
        let floor = DISAGREE_MIN_USD.max(billed_usd * DISAGREE_PCT / PERCENT);
        Ok(BilledComparison {
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
                "local estimate (this machine only) ${:.2} differs from ${:.2} reported by Claude",
                self.local_estimate_usd, self.billed_usd
            )
        } else {
            format!(
                "local estimate (this machine only) ${:.2} is close to ${:.2} reported by Claude",
                self.local_estimate_usd, self.billed_usd
            )
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    const NOW: i64 = 1_788_400_000;

    fn usd(minor: i64) -> Money {
        Money {
            minor,
            decimals: 2,
            currency: Some(USD.to_string()),
        }
    }

    fn billed(used: Option<Money>) -> SpendFigure {
        SpendFigure {
            limit: Some(usd(150_000)),
            used,
            fetched_at: NOW,
        }
    }

    fn cache_spend(limit: i64, used: i64, currency: &str, exponent: i64) -> Value {
        json!({
            "enabled": true,
            "limit": {"amount_minor": limit, "currency": currency, "exponent": exponent},
            "used": {"amount_minor": used, "currency": currency, "exponent": exponent},
        })
    }

    #[test]
    fn a_cache_spend_block_gives_limit_and_used_in_dollars() {
        let f = SpendFigure::from_cache(&cache_spend(150_000, 105_950, "USD", 2), NOW).unwrap();

        assert_eq!(f.limit.as_ref().unwrap().format(), "1500.00");
        assert_eq!(f.used.as_ref().unwrap().format(), "1059.50");
        assert_eq!(f.limit_usd(), Some(1500.0));
        assert_eq!(f.used_usd(), Some(1059.5));
        assert_eq!(f.fetched_at, NOW);
    }

    #[test]
    fn every_exponent_from_0_to_6_converts_and_7_is_rejected() {
        let expected = [
            105_950.0, 10_595.0, 1_059.5, 105.95, 10.595, 1.0595, 0.10595,
        ];

        for (exponent, want) in expected.iter().enumerate() {
            let m = Money::from_json(Some(&json!({
                "amount_minor": 105_950, "currency": "USD", "exponent": exponent
            })))
            .unwrap();

            assert!((m.to_f64() - want).abs() < 1e-9, "exponent {exponent}");
        }
        let seven = json!({"amount_minor": 105_950, "currency": "USD", "exponent": 7});
        assert_eq!(Money::from_json(Some(&seven)), None);
        let negative = json!({"amount_minor": 5, "currency": "USD", "exponent": -1});
        assert_eq!(Money::from_json(Some(&negative)), None);
    }

    #[test]
    fn a_missing_exponent_or_a_fractional_amount_is_a_bad_figure() {
        let no_exponent = json!({"amount_minor": 150_000, "currency": "USD"});
        let fractional = json!({"amount_minor": 1059.5, "currency": "USD", "exponent": 2});
        let negative = json!({"amount_minor": -5, "currency": "USD", "exponent": 2});
        let huge = json!({"amount_minor": 1e300, "currency": "USD", "exponent": 2});
        let text = json!({"amount_minor": "5", "currency": "USD", "exponent": 2});

        for v in [no_exponent, fractional, negative, huge, text] {
            assert_eq!(Money::from_json(Some(&v)), None, "{v}");
        }
        assert_eq!(Money::from_json(None), None);
    }

    #[test]
    fn a_block_that_is_off_or_has_no_amount_gives_no_figure() {
        let off = json!({"enabled": false, "limit": {"amount_minor": 1, "currency": "USD", "exponent": 2}});

        assert_eq!(SpendFigure::from_cache(&off, NOW), None);
        assert_eq!(
            SpendFigure::from_cache(&json!({"enabled": true}), NOW),
            None
        );
        assert_eq!(SpendFigure::from_cache(&json!(null), NOW), None);
    }

    #[test]
    fn a_currency_with_other_than_letters_and_digits_or_too_long_is_absent() {
        for code in ["US D", "\u{1b}[31mUSD", "ABCDEFGHI", ""] {
            let v = json!({"amount_minor": 1, "currency": code, "exponent": 2});

            assert_eq!(Money::from_json(Some(&v)).unwrap().currency, None, "{code}");
        }
    }

    #[test]
    fn the_same_dollars_give_equal_figures_from_the_cache_and_the_statusline() {
        let cache = SpendFigure::from_cache(&cache_spend(150_000, 105_950, "USD", 2), NOW);
        let statusline = SpendFigure::from_usd(Some(1500.0), Some(1059.5), NOW);

        assert_eq!(cache, statusline);
        assert!(SpendFigure::from_usd(None, None, NOW).is_none());
        assert!(SpendFigure::from_usd(Some(f64::NAN), Some(-1.0), NOW).is_none());
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
        assert_eq!(m(-50, 2).format(), "-0.50");
        assert_eq!(m(i64::MIN, 2).format(), "-92233720368547758.08");
        assert_eq!(m(1_234_567, 200).format(), "1.234567");
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

            let c = BilledComparison::new(billed(Some(usd(minor))), local).unwrap();

            assert_eq!(c.disagrees, disagrees, "billed {billed_usd} local {local}");
        }
    }

    #[test]
    fn the_comparison_keeps_the_delta_and_its_percent() {
        let c = BilledComparison::new(billed(Some(usd(105_950))), 900.0).unwrap();

        assert_eq!(c.delta_usd, 900.0 - 1059.5);
        assert!((c.delta_pct.unwrap() - (-15.054)).abs() < 0.001);
        assert!(BilledComparison::new(billed(Some(usd(0))), 0.5)
            .unwrap()
            .delta_pct
            .is_none());
    }

    #[test]
    fn another_currency_or_none_skips_the_comparison_with_a_reason() {
        let eur = Money {
            currency: Some("EUR".to_string()),
            ..usd(105_950)
        };
        let none = Money {
            currency: None,
            ..usd(105_950)
        };

        let a = BilledComparison::new(billed(Some(eur)), 1.0).unwrap_err();
        let b = BilledComparison::new(billed(Some(none)), 1.0).unwrap_err();
        let c = BilledComparison::new(billed(None), 1.0).unwrap_err();

        assert_eq!(a.to_string(), "currency EUR, not compared");
        assert_eq!(
            b.to_string(),
            "currency not reported or not readable, not compared"
        );
        assert_eq!(c, NotCompared::UsedUnknown);
    }

    #[test]
    fn the_line_says_differs_with_both_numbers_and_never_wrong() {
        let far = BilledComparison::new(billed(Some(usd(105_950))), 900.0).unwrap();
        let near = BilledComparison::new(billed(Some(usd(105_950))), 1055.0).unwrap();

        assert!(far.line().contains("differs"));
        assert!(far.line().contains("$900.00") && far.line().contains("$1059.50"));
        assert!(!far.line().contains("wrong"));
        assert!(near.line().contains("close to"));
    }
}
