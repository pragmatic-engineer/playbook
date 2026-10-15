// SPDX-FileCopyrightText: 2026 Igor Santos
// SPDX-License-Identifier: Apache-2.0

//! The release check that runs at the start of a command. `update.mode`
//! picks `notify` (the default), `auto` or `off`. The last result lives in the
//! `state` table under `update-check/cache`, so a command reads one row and
//! never waits on the network: a stale row starts a detached
//! `playbook update --refresh-check`, and the notice shows on a later run.
//! Every failure is silent and the check runs again after the next interval.

use super::download::Fetcher;
use super::resolve::compare;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::ffi::OsString;
use std::path::Path;
use std::time::Duration;

/// The `state` table key that holds the cached result.
pub const CACHE_KEY: &str = "update-check/cache";

/// The hard limit on the release list request.
pub const FETCH_TIMEOUT: Duration = Duration::from_millis(1500);

const DAY_SECS: u64 = 86_400;
const DEFAULT_INTERVAL_HOURS: u64 = 24;

/// What `update.mode` asks for.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Mode {
    Off,
    Notify,
    Auto,
}

impl Mode {
    pub fn parse(value: &str) -> Mode {
        match value {
            "off" => Mode::Off,
            "auto" => Mode::Auto,
            _ => Mode::Notify,
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Mode::Off => "off",
            Mode::Notify => "notify",
            Mode::Auto => "auto",
        }
    }
}

/// The two config keys the check reads.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Settings {
    pub mode: Mode,
    pub interval_secs: u64,
}

impl Settings {
    /// Reads `update.mode` and `update.checkIntervalHours` from the global
    /// tier. An unreadable store or a bad value falls back to the default.
    pub fn read(home: &Path) -> Settings {
        let mode = crate::config::resolve_valid("update.mode", home, None)
            .ok()
            .and_then(|(v, _, _)| v.as_str().map(Mode::parse))
            .unwrap_or(Mode::Notify);
        let hours = crate::config::resolve("update.checkIntervalHours", home, None)
            .ok()
            .and_then(|(v, _)| v.as_u64())
            .filter(|h| *h > 0)
            .unwrap_or(DEFAULT_INTERVAL_HOURS);
        Settings {
            mode,
            interval_secs: hours.saturating_mul(3600),
        }
    }
}

/// The cached result of the last check.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Cache {
    /// When the last check started, in unix seconds.
    #[serde(default)]
    pub checked_at: u64,
    /// The newest plain `vX.Y.Z` release, without the `v`.
    #[serde(default)]
    pub latest: Option<String>,
    /// When that release was published, as GitHub wrote it.
    #[serde(default)]
    pub published_at: Option<String>,
    /// `<version>@<day>` of the last notice, so it shows once a day.
    #[serde(default)]
    pub notified: Option<String>,
}

impl Cache {
    pub fn read(root: &Path) -> Result<Cache, String> {
        let raw = crate::state::get(root, CACHE_KEY).map_err(|e| e.to_string())?;
        Ok(raw
            .and_then(|text| serde_json::from_str(&text).ok())
            .unwrap_or_default())
    }

    /// Changes the stored row inside one transaction, so a refresh and a
    /// notice that run at the same time never undo each other.
    pub fn update(root: &Path, change: impl FnOnce(&mut Cache)) -> Result<(), String> {
        crate::state::transaction(root, |tx| {
            let mut cache: Cache = tx
                .get(CACHE_KEY)?
                .and_then(|text| serde_json::from_str(&text).ok())
                .unwrap_or_default();
            change(&mut cache);
            let text = serde_json::to_string(&cache)
                .map_err(|e| crate::state::StateError(e.to_string()))?;
            tx.set(CACHE_KEY, &text)
        })
        .map_err(|e| e.to_string())
    }
}

/// Whether the command in `args` (the full argv) may run the check. Hooks,
/// the status line, `version`, `json`, `update` itself and anything that asks
/// for JSON never do.
pub fn applies(args: &[OsString]) -> bool {
    let Some(first) = args.get(1).and_then(|a| a.to_str()) else {
        return false;
    };
    let skipped = [
        "hook",
        "statusline",
        "version",
        "--version",
        "-V",
        "json",
        "update",
        "upgrade",
        "help",
        "--help",
        "-h",
    ];
    if skipped.contains(&first) {
        return false;
    }
    !args.iter().any(|a| {
        a.to_str()
            .is_some_and(|s| s == "--json" || s.starts_with("--json="))
    })
}

/// Environment variables that mean a CI runner or a headless run.
const CI_VARS: [&str; 7] = [
    "CI",
    "GITHUB_ACTIONS",
    "GITLAB_CI",
    "BUILDKITE",
    "CIRCLECI",
    "JENKINS_URL",
    "TF_BUILD",
];

/// Whether the environment rules the check out: a CI variable set to
/// anything other than empty, `0` or `false`, or a headless run.
pub fn quiet_env(get: impl Fn(&str) -> Option<String>) -> bool {
    let set = |name: &str| {
        get(name).is_some_and(|v| {
            let v = v.trim().to_ascii_lowercase();
            !v.is_empty() && v != "0" && v != "false"
        })
    };
    CI_VARS.iter().any(|name| set(name)) || set("PLAYBOOK_HEADLESS")
}

/// The facts about this run that the plan needs.
#[derive(Debug, Clone)]
pub struct Probe {
    pub current: String,
    pub now: u64,
    pub stderr_tty: bool,
    pub homebrew: bool,
}

/// What this run should do.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Plan {
    /// Start a detached refresh and record the start time.
    pub refresh: bool,
    /// Print this line on stderr and record it as shown today.
    pub notice: Option<String>,
    /// The version the notice names.
    pub notice_version: Option<String>,
}

/// The one line the notice prints.
pub fn notice_text(latest: &str, current: &str, homebrew: bool) -> String {
    let fix = if homebrew {
        "brew upgrade playbook"
    } else {
        "playbook update"
    };
    format!("playbook {latest} is available (you have {current}). Run: {fix}")
}

fn notice_key(version: &str, now: u64) -> String {
    format!("{version}@{}", now / DAY_SECS)
}

/// Decides what to do from the settings, the cached row and this run. A
/// run without a terminal on stderr neither refreshes nor prints, since
/// nobody would see the notice. `auto` behaves as `notify` here.
pub fn plan(settings: Settings, cache: &Cache, probe: &Probe) -> Plan {
    if settings.mode == Mode::Off || !probe.stderr_tty {
        return Plan::default();
    }
    let age = probe.now.saturating_sub(cache.checked_at);
    let refresh = cache.checked_at > probe.now || age >= settings.interval_secs;
    let notice_version = cache
        .latest
        .clone()
        .filter(|latest| compare(latest, &probe.current).is_gt())
        .filter(|latest| cache.notified.as_deref() != Some(&notice_key(latest, probe.now)));
    Plan {
        refresh,
        notice: notice_version
            .as_deref()
            .map(|latest| notice_text(latest, &probe.current, probe.homebrew)),
        notice_version,
    }
}

/// Whether `tag` is a plain `vX.Y.Z` binary release tag.
fn is_plain_tag(tag: &str) -> bool {
    let Some(rest) = tag.strip_prefix('v') else {
        return false;
    };
    let parts: Vec<&str> = rest.split('.').collect();
    parts.len() == 3
        && parts
            .iter()
            .all(|p| !p.is_empty() && p.bytes().all(|b| b.is_ascii_digit()))
}

/// The newest published plain release in a `GET /releases` body, with its
/// publish time. Drafts, pre-releases and plugin or telemetry tags are
/// skipped.
pub fn newest_plain(body: &str) -> Result<Option<(String, Option<String>)>, String> {
    let value: Value =
        serde_json::from_str(body).map_err(|e| format!("unreadable release data: {e}"))?;
    let items = value.as_array().ok_or("unexpected release data")?;
    Ok(items
        .iter()
        .filter(|item| item["draft"].as_bool() != Some(true))
        .filter(|item| item["prerelease"].as_bool() != Some(true))
        .filter_map(|item| {
            let tag = item["tag_name"].as_str().filter(|t| is_plain_tag(t))?;
            let published = item["published_at"].as_str().map(str::to_string);
            Some((tag[1..].to_string(), published))
        })
        .max_by(|a, b| compare(&a.0, &b.0)))
}

/// Fetches the release list and stores the newest plain release. On an
/// error nothing is stored; the start time the caller recorded makes the
/// next try wait one interval.
pub fn refresh(root: &Path, fetcher: &dyn Fetcher, api_base: &str) -> Result<(), String> {
    let body = fetcher.get(&format!("{api_base}/releases?per_page=100"))?;
    let text = String::from_utf8(body).map_err(|_| "release data is not text".to_string())?;
    let Some((latest, published)) = newest_plain(&text)? else {
        return Ok(());
    };
    Cache::update(root, |cache| {
        cache.latest = Some(latest);
        cache.published_at = published;
    })
}

/// Applies `plan`: records the refresh start and the notice before acting,
/// so a store that cannot be written stops both. `spawn` starts the refresh
/// and `print` writes the notice.
pub fn execute(root: &Path, plan: &Plan, now: u64, spawn: &dyn Fn(), print: &mut dyn FnMut(&str)) {
    if !plan.refresh && plan.notice.is_none() {
        return;
    }
    let recorded = Cache::update(root, |cache| {
        if plan.refresh {
            cache.checked_at = now;
        }
        if let Some(version) = &plan.notice_version {
            cache.notified = Some(notice_key(version, now));
        }
    });
    if recorded.is_err() {
        return;
    }
    if plan.refresh {
        spawn();
    }
    if let Some(line) = &plan.notice {
        print(line);
    }
}

/// Starts `<exe> update --refresh-check` detached, in its own process group
/// so a Ctrl-C in the terminal does not reach it, with no stdio.
fn spawn_refresh() {
    let Ok(exe) = std::env::current_exe() else {
        return;
    };
    let mut cmd = std::process::Command::new(exe);
    cmd.args(["update", "--refresh-check"])
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null());
    #[cfg(unix)]
    {
        use std::os::unix::process::CommandExt;
        cmd.process_group(0);
    }
    let _ = cmd.spawn();
}

/// The entry point `main` calls before it dispatches a command. It never
/// fails and never waits on the network.
pub fn on_start(args: &[OsString]) {
    use std::io::IsTerminal;
    if !applies(args) || quiet_env(|name| std::env::var(name).ok()) {
        return;
    }
    let home = crate::common::home_dir();
    let settings = Settings::read(&home);
    if settings.mode == Mode::Off {
        return;
    }
    let stderr_tty = std::io::stderr().is_terminal();
    if !stderr_tty {
        return;
    }
    let root = crate::common::paths::playbook_root_from(&home);
    let Ok(cache) = Cache::read(&root) else {
        return;
    };
    let exe = std::env::current_exe().unwrap_or_default();
    let probe = Probe {
        current: env!("CARGO_PKG_VERSION").to_string(),
        now: crate::common::time::now_epoch_secs(),
        stderr_tty,
        homebrew: super::is_homebrew(&std::fs::canonicalize(&exe).unwrap_or(exe)),
    };
    let plan = plan(settings, &cache, &probe);
    execute(&root, &plan, probe.now, &spawn_refresh, &mut |line| {
        eprintln!("{line}")
    });
}

/// The body of the hidden `playbook update --refresh-check`.
pub fn run_refresh() {
    let home = crate::common::home_dir();
    let root = crate::common::paths::playbook_root_from(&home);
    let api_base = format!("https://api.github.com/repos/{}", super::verify::REPO);
    let fetcher = super::download::HttpFetcher {
        local_http: false,
        timeout: FETCH_TIMEOUT,
    };
    let _ = refresh(&root, &fetcher, &api_base);
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::common::test_support::scratch_dir;
    use crate::update::download::fake::FakeFetcher;
    use std::cell::Cell;

    const API: &str = "https://api.test/repos/x";
    const NOW: u64 = 1_800_000_000;
    const HOUR: u64 = 3600;

    const LIST: &str = r#"[
        {"tag_name":"playbook-plugin-v0.31.1","prerelease":false,"published_at":"2026-10-15T04:32:18Z"},
        {"tag_name":"telemetry-v0.9.0","prerelease":false},
        {"tag_name":"v0.28.0-rc.1","prerelease":true},
        {"tag_name":"v0.29.0","draft":true},
        {"tag_name":"v0.27.0","prerelease":false,"published_at":"2026-10-15T00:37:06Z"},
        {"tag_name":"v0.26.0","prerelease":false}
    ]"#;

    fn root(tag: &str) -> std::path::PathBuf {
        let dir = scratch_dir(tag);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn notify() -> Settings {
        Settings {
            mode: Mode::Notify,
            interval_secs: 24 * HOUR,
        }
    }

    fn probe(now: u64) -> Probe {
        Probe {
            current: "0.26.0".into(),
            now,
            stderr_tty: true,
            homebrew: false,
        }
    }

    fn argv(parts: &[&str]) -> Vec<OsString> {
        parts.iter().map(OsString::from).collect()
    }

    /// Runs one command start against `root` and returns the printed lines
    /// and how many refreshes it started.
    fn start(root: &Path, settings: Settings, probe: &Probe) -> (Vec<String>, u32) {
        let cache = Cache::read(root).unwrap();
        let spawned = Cell::new(0);
        let mut printed = Vec::new();
        execute(
            root,
            &plan(settings, &cache, probe),
            probe.now,
            &|| spawned.set(spawned.get() + 1),
            &mut |line| printed.push(line.to_string()),
        );
        (printed, spawned.get())
    }

    fn fetched(root: &Path) {
        let mut f = FakeFetcher::default();
        f.0.insert(format!("{API}/releases?per_page=100"), LIST.into());
        refresh(root, &f, API).unwrap();
    }

    #[test]
    fn only_plain_stable_tags_count_as_the_latest_release() {
        let (version, published) = newest_plain(LIST).unwrap().unwrap();
        assert_eq!(version, "0.27.0");
        assert_eq!(published.as_deref(), Some("2026-10-15T00:37:06Z"));
        assert_eq!(newest_plain("[]").unwrap(), None);
        assert!(newest_plain("{}").is_err());
    }

    #[test]
    fn the_notice_prints_once_a_day_for_each_version() {
        // Arrange
        let root = root("check-once");
        fetched(&root);
        Cache::update(&root, |c| c.checked_at = NOW).unwrap();

        // Act
        let (first, _) = start(&root, notify(), &probe(NOW));
        let (same_day, _) = start(&root, notify(), &probe(NOW + HOUR));
        let (next_day, _) = start(&root, notify(), &probe(NOW + DAY_SECS));

        // Assert
        assert_eq!(
            first,
            vec!["playbook 0.27.0 is available (you have 0.26.0). Run: playbook update"]
        );
        assert!(same_day.is_empty(), "{same_day:?}");
        assert_eq!(next_day.len(), 1);
    }

    #[test]
    fn a_fresh_cache_starts_no_refresh_until_the_interval_passes() {
        let root = root("check-interval");
        let (_, first) = start(&root, notify(), &probe(NOW));
        let (_, soon) = start(&root, notify(), &probe(NOW + 23 * HOUR));
        let (_, later) = start(&root, notify(), &probe(NOW + 24 * HOUR));
        assert_eq!((first, soon, later), (1, 0, 1));
        assert_eq!(Cache::read(&root).unwrap().checked_at, NOW + 24 * HOUR);
    }

    #[test]
    fn a_clock_moved_back_starts_a_refresh() {
        let root = root("check-skew");
        Cache::update(&root, |c| c.checked_at = NOW + DAY_SECS).unwrap();
        assert!(plan(notify(), &Cache::read(&root).unwrap(), &probe(NOW)).refresh);
    }

    #[test]
    fn an_offline_check_is_silent_and_waits_for_the_next_interval() {
        // Arrange
        let root = root("check-offline");
        let offline = FakeFetcher::default();

        // Act
        let (printed, spawned) = start(&root, notify(), &probe(NOW));
        let failed = refresh(&root, &offline, API);
        let (again, respawned) = start(&root, notify(), &probe(NOW + HOUR));

        // Assert
        assert!(failed.is_err());
        assert!(printed.is_empty() && again.is_empty());
        assert_eq!((spawned, respawned), (1, 0));
        assert_eq!(Cache::read(&root).unwrap().latest, None);
    }

    #[test]
    fn a_homebrew_install_is_told_to_use_brew() {
        let root = root("check-brew");
        fetched(&root);
        let mut p = probe(NOW);
        p.homebrew = true;
        let (printed, _) = start(&root, notify(), &p);
        assert_eq!(
            printed,
            vec!["playbook 0.27.0 is available (you have 0.26.0). Run: brew upgrade playbook"]
        );
    }

    #[test]
    fn off_mode_and_a_non_terminal_stderr_do_nothing() {
        let root = root("check-off");
        fetched(&root);
        let off = Settings {
            mode: Mode::Off,
            ..notify()
        };
        let mut piped = probe(NOW);
        piped.stderr_tty = false;
        assert_eq!(start(&root, off, &probe(NOW)), (vec![], 0));
        assert_eq!(start(&root, notify(), &piped), (vec![], 0));
        assert_eq!(Cache::read(&root).unwrap().notified, None);
    }

    #[test]
    fn an_up_to_date_binary_sees_no_notice() {
        let root = root("check-current");
        fetched(&root);
        let mut p = probe(NOW);
        p.current = "0.27.0".into();
        assert_eq!(start(&root, notify(), &p).0, Vec::<String>::new());
    }

    #[test]
    fn hooks_statusline_json_and_update_never_run_the_check() {
        for parts in [
            &["playbook", "hook", "session-init"][..],
            &["playbook", "statusline"],
            &["playbook", "json", "memory-context"],
            &["playbook", "mode", "status", "--json"],
            &["playbook", "doctor", "--json=true"],
            &["playbook", "update", "--check"],
            &["playbook", "--version"],
            &["playbook"],
        ] {
            assert!(!applies(&argv(parts)), "{parts:?}");
        }
        assert!(applies(&argv(&["playbook", "doctor"])));
        assert!(applies(&argv(&["playbook", "config", "get", "mode"])));
    }

    #[test]
    fn ci_variables_and_headless_runs_turn_the_check_off() {
        let env = |pairs: &'static [(&'static str, &'static str)]| {
            move |name: &str| {
                pairs
                    .iter()
                    .find(|(k, _)| *k == name)
                    .map(|(_, v)| v.to_string())
            }
        };
        assert!(quiet_env(env(&[("CI", "true")])));
        assert!(quiet_env(env(&[("GITHUB_ACTIONS", "true")])));
        assert!(quiet_env(env(&[("PLAYBOOK_HEADLESS", "1")])));
        assert!(!quiet_env(env(&[("CI", "false")])));
        assert!(!quiet_env(env(&[("CI", "")])));
        assert!(!quiet_env(env(&[])));
    }

    #[test]
    fn mode_values_parse_with_notify_as_the_fallback() {
        assert_eq!(Mode::parse("off"), Mode::Off);
        assert_eq!(Mode::parse("auto"), Mode::Auto);
        assert_eq!(Mode::parse("notify"), Mode::Notify);
        assert_eq!(Mode::parse("bogus"), Mode::Notify);
    }

    #[test]
    fn the_interval_comes_from_config() {
        let home = root("check-settings");
        assert_eq!(Settings::read(&home), notify());
        let root = crate::common::paths::playbook_root_from(&home);
        std::fs::create_dir_all(&root).unwrap();
        let global = crate::config::write::Tier::Global;
        crate::config::write::set(global, "update.checkIntervalHours", 6.into(), &home, None)
            .unwrap();
        crate::config::write::set(global, "update.mode", "off".into(), &home, None).unwrap();
        assert_eq!(
            Settings::read(&home),
            Settings {
                mode: Mode::Off,
                interval_secs: 6 * HOUR
            }
        );
    }
}
