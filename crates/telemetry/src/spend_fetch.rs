// SPDX-FileCopyrightText: 2026 Igor Santos
// SPDX-License-Identifier: Apache-2.0

//! Asking for the server figure, and remembering the answer. The program that
//! embeds usage supplies the fetcher through the host; this module never
//! touches a credential or the network itself. It keeps the answer for a short
//! time in a small file of numbers, so the terminal view and a one-shot report
//! share one request per window (two programs that find the cache out of date
//! at the same moment may each ask once), and a failure is remembered for longer so a
//! bad token cannot cause a retry storm.

use super::config::{DEFAULT_SPEND_CACHE_SECONDS, SPEND_CACHE_SECONDS};
use super::host::Host;
use super::limits::Origin;
use super::limits_source::{utc_month_start, Window};
use super::plan::{self, LoginKind, PlanChoice, CLOCK_SKEW_SECONDS};
use super::spend::{self, Money, Reason, Scope, SpendFigure, MAX_DECIMALS};
use serde::{Deserialize, Serialize};
use std::panic::{catch_unwind, AssertUnwindSafe};
use std::path::{Path, PathBuf};
use std::sync::Mutex;

/// The name of the cache file in the data folder.
pub const CACHE_FILE: &str = "usage-spend.json";

/// How long a failure is remembered, in seconds.
pub const NEGATIVE_SECONDS: i64 = 300;

/// How long a refused token is remembered, in seconds.
pub const REFUSED_SECONDS: i64 = 900;

/// The largest amount in minor units that is read back from the cache.
const MAX_MINOR_UNITS: i64 = 1_000_000_000_000_000;

/// The largest percentage that is read back from the cache.
const MAX_PERCENT: f64 = 1_000_000.0;

/// What a fetch gives: a figure, `None` for an account with no spend limit, or
/// the class of the failure.
pub type Outcome = Result<Option<SpendFigure>, Reason>;

/// Asks the server. One call, no retry. Implemented by the embedding program,
/// which holds the credential and the network code.
pub trait SpendFetcher: Send + Sync {
    fn fetch(&self, now: i64) -> Outcome;
}

impl SpendFetcher for Box<dyn SpendFetcher> {
    fn fetch(&self, now: i64) -> Outcome {
        self.as_ref().fetch(now)
    }
}

#[derive(Debug, Serialize, Deserialize)]
struct MoneyDoc {
    minor: i64,
    decimals: u8,
    currency: Option<String>,
}

#[derive(Debug, Serialize, Deserialize)]
struct WindowDoc {
    name: String,
    used_percentage: f64,
    resets_at: i64,
}

#[derive(Debug, Serialize, Deserialize)]
struct FigureDoc {
    limit: Option<MoneyDoc>,
    used: Option<MoneyDoc>,
    percent: Option<f64>,
    severity: Option<String>,
    resets_at: Option<i64>,
    limit_reached: Option<bool>,
    scope: String,
    windows: Vec<WindowDoc>,
}

/// The cache file. It holds numbers, a time and a reason key, and no field a
/// credential could be stored in.
#[derive(Debug, Serialize, Deserialize)]
struct Entry {
    fetched_at: i64,
    error: Option<String>,
    figure: Option<FigureDoc>,
}

impl MoneyDoc {
    fn of(m: &Money) -> MoneyDoc {
        MoneyDoc {
            minor: m.minor,
            decimals: m.decimals,
            currency: m.currency.clone(),
        }
    }

    fn money(self) -> Option<Money> {
        let word = |s: &str| spend::word(Some(&serde_json::Value::from(s)));
        let currency = match self.currency {
            Some(c) => Some(word(&c)?),
            None => None,
        };
        ((0..=MAX_MINOR_UNITS).contains(&self.minor) && self.decimals <= MAX_DECIMALS).then_some(
            Money {
                minor: self.minor,
                decimals: self.decimals,
                currency,
            },
        )
    }
}

impl FigureDoc {
    fn of(f: &SpendFigure) -> FigureDoc {
        FigureDoc {
            limit: f.limit.as_ref().map(MoneyDoc::of),
            used: f.used.as_ref().map(MoneyDoc::of),
            percent: f.percent,
            severity: f.severity.clone(),
            resets_at: f.resets_at,
            limit_reached: f.limit_reached,
            scope: f.scope.key().to_string(),
            windows: f
                .windows
                .iter()
                .map(|(name, w)| WindowDoc {
                    name: name.clone(),
                    used_percentage: w.used_percentage,
                    resets_at: w.resets_at,
                })
                .collect(),
        }
    }

    /// The figure, or `None` when any value is out of range. Nothing is
    /// repaired: a cache that holds one bad value is ignored whole.
    fn figure(self, fetched_at: i64) -> Option<SpendFigure> {
        let word = |s: &str| spend::word(Some(&serde_json::Value::from(s)));
        let sound = |p: f64| p.is_finite() && (0.0..=MAX_PERCENT).contains(&p);
        if !self.percent.is_none_or(sound) {
            return None;
        }
        let severity = match self.severity {
            Some(s) => Some(word(&s)?),
            None => None,
        };
        let mut windows = Vec::new();
        for w in self.windows {
            if !sound(w.used_percentage) {
                return None;
            }
            windows.push((
                word(&w.name)?,
                Window {
                    used_percentage: w.used_percentage,
                    resets_at: w.resets_at,
                },
            ));
        }
        Some(SpendFigure {
            limit: match self.limit {
                Some(m) => Some(m.money()?),
                None => None,
            },
            used: match self.used {
                Some(m) => Some(m.money()?),
                None => None,
            },
            percent: self.percent,
            severity,
            resets_at: self.resets_at,
            limit_reached: self.limit_reached,
            scope: Scope::from_key(&self.scope),
            windows,
            fetched_at,
            origin: Origin::Server,
        })
    }
}

/// How long a failure of this class is remembered.
fn negative_seconds(reason: Reason) -> i64 {
    match reason {
        Reason::Unauthorized => REFUSED_SECONDS,
        _ => NEGATIVE_SECONDS,
    }
}

/// A fetcher behind the cache file. `ttl` is how long a good answer is kept.
pub struct CachedFetcher<F> {
    inner: F,
    path: PathBuf,
    ttl: i64,
    gate: Mutex<()>,
}

impl<F: SpendFetcher> CachedFetcher<F> {
    pub fn new(inner: F, dir: &Path, ttl_seconds: i64) -> CachedFetcher<F> {
        CachedFetcher {
            inner,
            path: dir.join(CACHE_FILE),
            ttl: ttl_seconds,
            gate: Mutex::new(()),
        }
    }

    /// The entry on disk when it is sound, or `None` for a missing file, bad
    /// JSON, a time in the future or before this month, or any value out of
    /// range.
    fn read(&self, now: i64) -> Option<(i64, Outcome)> {
        read_entry(&self.path, now)
    }

    /// The answer kept within its time, without asking.
    pub fn cached(&self, now: i64) -> Option<Outcome> {
        let (at, outcome) = self.read(now)?;
        let keep = match &outcome {
            Ok(_) => self.ttl,
            Err(reason) => negative_seconds(*reason),
        };
        (now - at < keep).then_some(outcome)
    }

    /// The last answer kept, however old, without asking.
    pub fn last(&self, now: i64) -> Option<Outcome> {
        self.read(now).map(|(_, outcome)| outcome)
    }

    /// The answer in the cache when it is still good, else one request. A
    /// panic in the fetcher is caught and becomes the `internal` class.
    pub fn get(&self, now: i64) -> Outcome {
        let _one_at_a_time = self.gate.lock().unwrap_or_else(|e| e.into_inner());
        if let Some(outcome) = self.cached(now) {
            return outcome;
        }
        let outcome = catch_unwind(AssertUnwindSafe(|| self.inner.fetch(now)))
            .unwrap_or(Err(Reason::Internal));
        self.store(now, &outcome);
        outcome
    }

    /// Writes the answer. A folder that cannot be written only means the next
    /// call asks again.
    fn store(&self, now: i64, outcome: &Outcome) {
        let entry = Entry {
            fetched_at: now,
            error: outcome.as_ref().err().map(|r| r.key().to_string()),
            figure: outcome
                .as_ref()
                .ok()
                .and_then(Option::as_ref)
                .map(FigureDoc::of),
        };
        if let Ok(json) = serde_json::to_string(&entry) {
            let _ = crate::util::atomic::write_atomic_private(&self.path, json);
        }
    }
}

fn read_entry(path: &Path, now: i64) -> Option<(i64, Outcome)> {
    let entry: Entry = serde_json::from_str(&std::fs::read_to_string(path).ok()?).ok()?;
    if entry.fetched_at > now + CLOCK_SKEW_SECONDS || entry.fetched_at < utc_month_start(now) {
        return None;
    }
    let at = entry.fetched_at;
    let outcome = match (entry.error, entry.figure) {
        (Some(key), None) => Err(Reason::from_key(&key).filter(|r| *r != Reason::Fetching)?),
        (None, figure) => Ok(match figure {
            Some(doc) => Some(doc.figure(at)?),
            None => None,
        }),
        (Some(_), Some(_)) => return None,
    };
    Some((at, outcome))
}

/// The age of the answer in the cache and the class of its failure, for the
/// status command. It reads one small file of numbers.
pub fn cache_status(dir: &Path, now: i64) -> Option<(i64, Option<Reason>)> {
    let (at, outcome) = read_entry(&dir.join(CACHE_FILE), now)?;
    Some(((now - at).max(0), outcome.err()))
}

/// Removes the cache file. A missing file is fine.
pub fn clear_cache(dir: &Path) {
    let _ = std::fs::remove_file(dir.join(CACHE_FILE));
}

/// What the views get: the figure, or the reason there is none to show.
#[derive(Debug, Default, Clone, PartialEq)]
pub struct Views {
    pub figure: Option<SpendFigure>,
    pub unavailable: Option<Reason>,
}

/// The one place that asks the host for a fetcher. `network` is false for a
/// first screen, which then shows only what the cache holds. Every failure
/// becomes a reason, never an error. `env` reads the environment, which tells
/// the login kind.
pub fn for_views(
    host: &dyn Host,
    dir: &Path,
    claude_json: &Path,
    now: i64,
    network: bool,
    env: &dyn Fn(&str) -> Option<String>,
) -> Views {
    let home = host.home_dir();
    if host.config_value("usage.spend.enabled", &home) != Some(serde_json::Value::Bool(true)) {
        return Views::default();
    }
    // A program without the feature (pbtop) shows nothing about it.
    let Some(fetcher) = host.spend_fetcher() else {
        return Views::default();
    };
    let choice = host
        .config_value("usage.plan", &home)
        .and_then(|v| v.as_str().and_then(PlanChoice::parse))
        .unwrap_or_default();
    let login = LoginKind::detect(plan::has_oauth_account(claude_json), env);
    let reason = |r| Views {
        figure: None,
        unavailable: Some(r),
    };
    if choice.is_local_only() || matches!(login, LoginKind::ApiKey | LoginKind::Cloud) {
        return reason(Reason::UnsupportedLogin);
    }
    if !host.spend_ack() {
        return reason(Reason::NoConsent);
    }
    let ttl = host
        .config_value("usage.spend.cacheSeconds", &home)
        .and_then(|v| v.as_i64())
        .filter(|n| SPEND_CACHE_SECONDS.contains(n))
        .unwrap_or(DEFAULT_SPEND_CACHE_SECONDS);
    let cache = CachedFetcher::new(fetcher, dir, ttl);
    let outcome = if network {
        Some(cache.get(now))
    } else {
        cache.last(now)
    };
    match outcome {
        Some(Ok(figure)) => Views {
            figure,
            unavailable: None,
        },
        Some(Err(r)) => reason(r),
        None => reason(Reason::Fetching),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::Arc;

    const NOW: i64 = 1_788_400_000;
    const TTL: i64 = 60;
    const MIN_TTL: i64 = *SPEND_CACHE_SECONDS.start();
    const MAX_TTL: i64 = *SPEND_CACHE_SECONDS.end();

    struct Counting {
        calls: Arc<AtomicUsize>,
        answer: Outcome,
    }

    impl SpendFetcher for Counting {
        fn fetch(&self, _now: i64) -> Outcome {
            self.calls.fetch_add(1, Ordering::SeqCst);
            self.answer.clone()
        }
    }

    struct Panicking;

    impl SpendFetcher for Panicking {
        fn fetch(&self, _now: i64) -> Outcome {
            panic!("the fetcher blew up");
        }
    }

    fn scratch(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("spend-fetch-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn figure(at: i64) -> SpendFigure {
        match spend::parse(
            r#"{"extra_usage":{"monthly_limit":50000,"used_credits":12345,
                "decimal_places":2,"currency":"USD","utilization":24.69},
                "spend":{"severity":"normal"},"five_hour":{"utilization":10,
                "resets_at":"2026-10-20T18:00:00Z"}}"#,
            at,
        ) {
            Ok(spend::Parsed::Figure(f)) => f,
            other => panic!("fixture did not parse: {other:?}"),
        }
    }

    fn counting(answer: Outcome) -> (Counting, Arc<AtomicUsize>) {
        let calls = Arc::new(AtomicUsize::new(0));
        (
            Counting {
                calls: calls.clone(),
                answer,
            },
            calls,
        )
    }

    #[test]
    fn a_good_answer_is_kept_for_the_ttl_and_asked_again_after_it() {
        let dir = scratch("ttl");
        let (fetcher, calls) = counting(Ok(Some(figure(NOW))));
        let cache = CachedFetcher::new(fetcher, &dir, TTL);

        cache.get(NOW).unwrap();
        cache.get(NOW + TTL - 1).unwrap();
        assert_eq!(calls.load(Ordering::SeqCst), 1);
        cache.get(NOW + TTL).unwrap();

        assert_eq!(calls.load(Ordering::SeqCst), 2);
    }

    #[test]
    fn the_cached_figure_equals_the_fetched_one() {
        let dir = scratch("roundtrip");
        let (fetcher, _) = counting(Ok(Some(figure(NOW))));
        let cache = CachedFetcher::new(fetcher, &dir, TTL);
        let fetched = cache.get(NOW).unwrap();

        let read = cache.cached(NOW + 1).unwrap().unwrap();

        assert_eq!(read, fetched);
    }

    #[test]
    fn a_refused_token_is_remembered_for_fifteen_minutes() {
        let dir = scratch("refused");
        let (fetcher, calls) = counting(Err(Reason::Unauthorized));
        let cache = CachedFetcher::new(fetcher, &dir, TTL);

        cache.get(NOW).unwrap_err();
        cache.get(NOW + REFUSED_SECONDS - 1).unwrap_err();
        assert_eq!(calls.load(Ordering::SeqCst), 1);
        cache.get(NOW + REFUSED_SECONDS).unwrap_err();

        assert_eq!(calls.load(Ordering::SeqCst), 2);
    }

    #[test]
    fn any_other_failure_is_remembered_for_five_minutes() {
        let dir = scratch("negative");
        let (fetcher, calls) = counting(Err(Reason::Network));
        let cache = CachedFetcher::new(fetcher, &dir, TTL);

        cache.get(NOW).unwrap_err();
        cache.get(NOW + NEGATIVE_SECONDS - 1).unwrap_err();
        assert_eq!(calls.load(Ordering::SeqCst), 1);
        cache.get(NOW + NEGATIVE_SECONDS).unwrap_err();

        assert_eq!(calls.load(Ordering::SeqCst), 2);
    }

    #[test]
    fn an_account_with_no_limit_is_cached_as_no_figure() {
        let dir = scratch("empty");
        let (fetcher, calls) = counting(Ok(None));
        let cache = CachedFetcher::new(fetcher, &dir, TTL);

        assert_eq!(cache.get(NOW), Ok(None));
        assert_eq!(cache.get(NOW + 1), Ok(None));

        assert_eq!(calls.load(Ordering::SeqCst), 1);
    }

    #[test]
    fn the_cache_file_holds_no_token_field_and_not_the_fake_secret() {
        let dir = scratch("secret");
        let (fetcher, _) = counting(Ok(Some(figure(NOW))));
        let cache = CachedFetcher::new(fetcher, &dir, TTL);
        cache.get(NOW).unwrap();

        let text = std::fs::read_to_string(dir.join(CACHE_FILE)).unwrap();

        for needle in ["token", "Bearer", "authorization"] {
            assert!(
                !text.to_lowercase().contains(&needle.to_lowercase()),
                "{needle}"
            );
        }
    }

    #[test]
    fn a_planted_cache_with_a_bad_value_is_ignored() {
        let dir = scratch("planted");
        let money = |minor: i64, decimals: u8| {
            format!(r#"{{"minor":{minor},"decimals":{decimals},"currency":"USD"}}"#)
        };
        let doc = |fetched_at: i64, limit: String, percent: &str, severity: &str| {
            format!(
                r#"{{"fetched_at":{fetched_at},"error":null,"figure":{{"limit":{limit},
                "used":null,"percent":{percent},"severity":{severity},"resets_at":null,
                "limit_reached":null,"scope":"org","windows":[]}}}}"#
            )
        };
        let cases = [
            (
                "future",
                doc(NOW + CLOCK_SKEW_SECONDS + 1, money(1, 2), "1", "null"),
            ),
            (
                "last month",
                doc(utc_month_start(NOW) - 1, money(1, 2), "1", "null"),
            ),
            (
                "currency",
                doc(
                    NOW,
                    r#"{"minor":1,"decimals":2,"currency":"a b"}"#.to_string(),
                    "1",
                    "null",
                ),
            ),
            ("decimals", doc(NOW, money(1, 99), "1", "null")),
            ("negative", doc(NOW, money(-5, 2), "1", "null")),
            ("percent", doc(NOW, money(1, 2), "-3", "null")),
            (
                "severity",
                doc(NOW, money(1, 2), "1", r#""has spaces and <b>""#),
            ),
            ("garbage", "{not json".to_string()),
            (
                "fetching",
                format!(r#"{{"fetched_at":{NOW},"error":"fetching","figure":null}}"#),
            ),
            (
                "unknown error",
                format!(r#"{{"fetched_at":{NOW},"error":"nope","figure":null}}"#),
            ),
            (
                "both",
                doc(NOW, money(1, 2), "1", "null")
                    .replace(r#""error":null"#, r#""error":"network""#),
            ),
            (
                "window name",
                doc(NOW, money(1, 2), "1", "null").replace(
                    r#""windows":[]"#,
                    r#""windows":[{"name":"a b","used_percentage":1,"resets_at":1}]"#,
                ),
            ),
        ];
        for (name, body) in cases {
            std::fs::write(dir.join(CACHE_FILE), body).unwrap();
            let (fetcher, calls) = counting(Err(Reason::Network));
            let cache = CachedFetcher::new(fetcher, &dir, TTL);

            assert_eq!(cache.cached(NOW), None, "{name}");
            assert_eq!(cache.last(NOW), None, "{name}");
            cache.get(NOW).unwrap_err();
            assert_eq!(calls.load(Ordering::SeqCst), 1, "{name}");
        }
    }

    #[test]
    fn a_panicking_fetcher_becomes_internal_and_is_not_asked_again_within_the_window() {
        let dir = scratch("panic");
        let cache = CachedFetcher::new(Panicking, &dir, TTL);

        assert_eq!(cache.get(NOW), Err(Reason::Internal));
        assert_eq!(cache.cached(NOW + 1), Some(Err(Reason::Internal)));
        assert_eq!(cache.cached(NOW + NEGATIVE_SECONDS), None);
    }

    #[test]
    fn a_poisoned_gate_is_recovered() {
        let dir = scratch("poison");
        let (fetcher, _) = counting(Ok(None));
        let cache = Arc::new(CachedFetcher::new(fetcher, &dir, TTL));
        let held = cache.clone();
        let _ = std::thread::spawn(move || {
            let _guard = held.gate.lock().unwrap();
            panic!("poison the gate");
        })
        .join();

        assert_eq!(cache.get(NOW), Ok(None));
    }

    #[test]
    fn concurrent_calls_make_one_request() {
        let dir = scratch("concurrent");
        let (fetcher, calls) = counting(Ok(Some(figure(NOW))));
        let cache = Arc::new(CachedFetcher::new(fetcher, &dir, TTL));

        let threads: Vec<_> = (0..4)
            .map(|_| {
                let c = cache.clone();
                std::thread::spawn(move || c.get(NOW).unwrap())
            })
            .collect();
        for t in threads {
            t.join().unwrap();
        }

        assert_eq!(calls.load(Ordering::SeqCst), 1);
    }

    #[test]
    fn an_unwritable_folder_still_returns_the_answer() {
        let dir = scratch("unwritable").join("missing").join("deeper");
        let (fetcher, _) = counting(Ok(Some(figure(NOW))));
        let cache = CachedFetcher::new(fetcher, &dir, TTL);

        assert!(cache.get(NOW).unwrap().is_some());
    }

    #[test]
    fn the_status_gives_the_age_and_the_class_of_the_last_failure() {
        let dir = scratch("status");
        let (fetcher, _) = counting(Err(Reason::RateLimited));
        CachedFetcher::new(fetcher, &dir, TTL).get(NOW).unwrap_err();

        assert_eq!(
            cache_status(&dir, NOW + 30),
            Some((30, Some(Reason::RateLimited)))
        );
        clear_cache(&dir);
        assert_eq!(cache_status(&dir, NOW + 30), None);
    }

    /// A host whose config, acknowledgement and fetcher the test sets.
    struct FakeHost {
        config: serde_json::Value,
        ack: bool,
        has_fetcher: bool,
        calls: Arc<AtomicUsize>,
        answer: Outcome,
    }

    impl FakeHost {
        fn on(answer: Outcome) -> FakeHost {
            FakeHost {
                config: serde_json::json!({"usage.spend.enabled": true}),
                ack: true,
                has_fetcher: true,
                calls: Arc::new(AtomicUsize::new(0)),
                answer,
            }
        }
    }

    impl Host for FakeHost {
        fn program(&self) -> &str {
            "playbook"
        }
        fn command(&self, args: &str) -> String {
            format!("playbook {args}")
        }
        fn setup_command(&self, part: &str) -> String {
            part.to_string()
        }
        fn launch_args(&self) -> Vec<String> {
            Vec::new()
        }
        fn repos_marker(&self) -> Option<&str> {
            None
        }
        fn home_dir(&self) -> PathBuf {
            PathBuf::from("/nonexistent-home")
        }
        fn config_value(&self, key: &str, _home: &Path) -> Option<serde_json::Value> {
            self.config.get(key).cloned()
        }
        fn state_dir_from(&self, home: &Path) -> PathBuf {
            home.to_path_buf()
        }
        fn state_dir(&self) -> PathBuf {
            PathBuf::new()
        }
        fn sessions(
            &self,
            _paths: &super::super::run::Paths,
            _home: &Path,
        ) -> Result<(Vec<super::super::session_view::Record>, i64), String> {
            Ok((Vec::new(), 0))
        }
        fn spend_fetcher(&self) -> Option<Box<dyn SpendFetcher>> {
            self.has_fetcher.then(|| -> Box<dyn SpendFetcher> {
                Box::new(Counting {
                    calls: self.calls.clone(),
                    answer: self.answer.clone(),
                })
            })
        }
        fn spend_ack(&self) -> bool {
            self.ack
        }
    }

    fn views(host: &FakeHost, dir: &Path, network: bool, env: &[(&str, &str)]) -> Views {
        let env: Vec<(String, String)> = env
            .iter()
            .map(|(k, v)| (k.to_string(), v.to_string()))
            .collect();
        for_views(
            host,
            dir,
            &dir.join(".claude.json"),
            NOW,
            network,
            &|name| env.iter().find(|(k, _)| k == name).map(|(_, v)| v.clone()),
        )
    }

    #[test]
    fn the_default_off_state_makes_no_call_and_shows_no_line() {
        let dir = scratch("off");
        let mut host = FakeHost::on(Ok(None));
        host.config = serde_json::json!({});

        let v = views(&host, &dir, true, &[]);

        assert_eq!(v, Views::default());
        assert_eq!(host.calls.load(Ordering::SeqCst), 0);
    }

    #[test]
    fn a_program_without_a_fetcher_shows_nothing_whatever_the_key_says() {
        let dir = scratch("nofetcher");
        let mut host = FakeHost::on(Ok(None));
        host.has_fetcher = false;
        host.ack = false;

        assert_eq!(views(&host, &dir, true, &[]), Views::default());
    }

    #[test]
    fn the_key_without_the_acknowledgement_makes_no_call_and_asks_for_consent() {
        let dir = scratch("noack");
        let mut host = FakeHost::on(Ok(None));
        host.ack = false;

        let v = views(&host, &dir, true, &[]);

        assert_eq!(v.unavailable, Some(Reason::NoConsent));
        assert_eq!(host.calls.load(Ordering::SeqCst), 0);
    }

    #[test]
    fn the_key_with_the_acknowledgement_makes_one_call() {
        let dir = scratch("ack");
        let host = FakeHost::on(Ok(Some(figure(NOW))));

        let v = views(&host, &dir, true, &[]);
        views(&host, &dir, true, &[]);

        assert_eq!(
            v,
            Views {
                figure: Some(figure(NOW)),
                unavailable: None
            }
        );
        assert_eq!(host.calls.load(Ordering::SeqCst), 1);
    }

    #[test]
    fn a_login_without_an_oauth_token_makes_no_call() {
        let dir = scratch("login");
        let host = FakeHost::on(Ok(None));
        for env in [
            [("ANTHROPIC_API_KEY", "set")],
            [("CLAUDE_CODE_USE_BEDROCK", "1")],
        ] {
            let v = views(&host, &dir, true, &env);
            assert_eq!(v.unavailable, Some(Reason::UnsupportedLogin));
        }
        let mut local = FakeHost::on(Ok(None));
        local.config = serde_json::json!({"usage.spend.enabled": true, "usage.plan": "api"});
        assert_eq!(
            views(&local, &dir, true, &[]).unavailable,
            Some(Reason::UnsupportedLogin)
        );
        assert_eq!(
            host.calls.load(Ordering::SeqCst) + local.calls.load(Ordering::SeqCst),
            0
        );
    }

    #[test]
    fn a_first_screen_never_asks_and_says_it_is_fetching() {
        let dir = scratch("first");
        let host = FakeHost::on(Ok(Some(figure(NOW))));

        let first = views(&host, &dir, false, &[]);
        assert_eq!(first.unavailable, Some(Reason::Fetching));
        assert_eq!(host.calls.load(Ordering::SeqCst), 0);

        views(&host, &dir, true, &[]);
        let later = views(&host, &dir, false, &[]);
        assert!(later.figure.is_some());
        assert_eq!(host.calls.load(Ordering::SeqCst), 1);
    }

    #[test]
    fn a_first_screen_shows_an_expired_figure_without_asking() {
        let dir = scratch("expired");
        let host = FakeHost::on(Ok(Some(figure(NOW))));
        views(&host, &dir, true, &[]);

        let later = for_views(
            &host,
            &dir,
            &dir.join(".claude.json"),
            NOW + 2 * TTL,
            false,
            &|_| None,
        );

        assert!(later.figure.is_some());
        assert_eq!(host.calls.load(Ordering::SeqCst), 1);
    }

    #[test]
    fn the_cache_seconds_setting_is_used_in_range_and_ignored_out_of_range() {
        let ask_again_after = |setting: i64| {
            let dir = scratch(&format!("ttl-{setting}"));
            let mut host = FakeHost::on(Ok(None));
            host.config = serde_json::json!({
                "usage.spend.enabled": true,
                "usage.spend.cacheSeconds": setting,
            });
            let claude = dir.join(".claude.json");
            let calls = |at: i64| {
                for_views(&host, &dir, &claude, at, true, &|_| None);
                host.calls.load(Ordering::SeqCst)
            };
            calls(NOW);
            (1..=MAX_TTL).find(|s| calls(NOW + s) > 1).unwrap_or(0)
        };

        assert_eq!(ask_again_after(MIN_TTL), MIN_TTL);
        assert_eq!(ask_again_after(1), DEFAULT_SPEND_CACHE_SECONDS);
    }

    #[test]
    fn every_reason_reads_back_from_its_key() {
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
            assert_eq!(Reason::from_key(reason.key()), Some(reason));
        }
        assert_eq!(Reason::from_key("nope"), None);
    }

    #[test]
    fn an_oauth_account_in_the_config_file_is_a_claude_ai_login() {
        let dir = scratch("oauth");
        let write = |name: &str, body: &str| {
            let path = dir.join(name);
            std::fs::write(&path, body).unwrap();
            path
        };

        assert!(plan::has_oauth_account(&write(
            "with.json",
            r#"{"oauthAccount":{"x":1}}"#
        )));
        assert!(!plan::has_oauth_account(&write("without.json", "{}")));
        assert!(!plan::has_oauth_account(&write("bad.json", "{oops")));
        assert!(!plan::has_oauth_account(&dir.join("absent.json")));
    }

    #[test]
    fn each_failure_class_becomes_its_reason() {
        for reason in [
            Reason::NoToken,
            Reason::Unauthorized,
            Reason::RateLimited,
            Reason::Timeout,
            Reason::Schema,
            Reason::Network,
        ] {
            let dir = scratch(reason.key());
            let host = FakeHost::on(Err(reason));
            assert_eq!(views(&host, &dir, true, &[]).unavailable, Some(reason));
        }
    }

    #[test]
    fn only_this_module_asks_the_host_for_a_fetcher() {
        let src = Path::new(env!("CARGO_MANIFEST_DIR")).join("src");
        let mut stack = vec![src];
        let mut users = Vec::new();
        while let Some(dir) = stack.pop() {
            for entry in std::fs::read_dir(&dir).unwrap().flatten() {
                let path = entry.path();
                if path.is_dir() {
                    stack.push(path);
                } else if path.extension().is_some_and(|e| e == "rs")
                    && std::fs::read_to_string(&path)
                        .unwrap()
                        .contains(".spend_fetcher()")
                {
                    users.push(path.file_name().unwrap().to_string_lossy().into_owned());
                }
            }
        }
        users.sort();
        assert_eq!(users, ["spend_fetch.rs"]);
    }
}
