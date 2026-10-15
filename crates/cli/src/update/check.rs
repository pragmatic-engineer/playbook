// SPDX-FileCopyrightText: 2026 Igor Santos
// SPDX-License-Identifier: Apache-2.0

//! The release check that runs at the start of a command. `update.mode`
//! picks `notify` (the default), `auto` or `off`. The last result lives in the
//! `state` table under `update-check/cache`, so a command reads one row and
//! never waits on the network: a stale row starts a detached
//! `playbook update --refresh-check`, and the notice shows on a later run.
//! Every failure is silent and the check runs again after the next interval.

use super::download::Fetcher;
use super::resolve::{self, compare};
use crate::Command;
use serde::{Deserialize, Serialize};
use std::ffi::OsString;
use std::path::Path;
use std::time::Duration;

/// The `state` table key that holds the cached result.
pub const CACHE_KEY: &str = "update-check/cache";

/// The limit on the release list request. It runs in a detached process,
/// so no command waits for it.
pub const REFRESH_TIMEOUT: Duration = Duration::from_secs(10);

const DAY_SECS: u64 = 86_400;
const DEFAULT_INTERVAL_HOURS: u64 = 24;

/// What `update.mode` asks for. This is separate from the playbook
/// permission mode (`ask` or `auto`) that `Env::auto_mode` reads.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UpdateMode {
    Off,
    Notify,
    Auto,
}

impl UpdateMode {
    pub fn parse(value: &str) -> UpdateMode {
        match value {
            "off" => UpdateMode::Off,
            "auto" => UpdateMode::Auto,
            _ => UpdateMode::Notify,
        }
    }
}

/// The two config keys the check reads.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Settings {
    pub mode: UpdateMode,
    pub interval_secs: u64,
}

impl Settings {
    /// Reads `update.mode` and `update.checkIntervalHours` through the usual
    /// tiers. An unreadable store or a bad value falls back to the default.
    pub fn read(home: &Path, repo_slug: Option<&str>) -> Settings {
        let mode = crate::config::resolve_valid("update.mode", home, repo_slug)
            .ok()
            .and_then(|(v, _, _)| v.as_str().map(UpdateMode::parse))
            .unwrap_or(UpdateMode::Notify);
        let hours = crate::config::resolve("update.checkIntervalHours", home, repo_slug)
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
    /// `<version>@<unix seconds>` of the last notice.
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

    /// Reads, changes and writes the row in one transaction and returns what
    /// `change` returns, so two processes never both claim the same action.
    pub fn update<T>(root: &Path, change: impl FnOnce(&mut Cache) -> T) -> Result<T, String> {
        crate::state::transaction(root, |tx| {
            let mut cache: Cache = tx
                .get(CACHE_KEY)?
                .and_then(|text| serde_json::from_str(&text).ok())
                .unwrap_or_default();
            let out = change(&mut cache);
            let text = serde_json::to_string(&cache)
                .map_err(|e| crate::state::StateError(e.to_string()))?;
            tx.set(CACHE_KEY, &text)?;
            Ok(out)
        })
        .map_err(|e| e.to_string())
    }

    /// Whether the notice for `version` showed less than a day before `now`.
    fn shown_recently(&self, version: &str, now: u64) -> bool {
        self.notified
            .as_deref()
            .and_then(|n| n.rsplit_once('@'))
            .filter(|(v, _)| *v == version)
            .and_then(|(_, at)| at.parse::<u64>().ok())
            .is_some_and(|at| at <= now && now - at < DAY_SECS)
    }
}

/// Whether `command` may run the check. Hooks, the status line,
/// `shell-init` (it runs on every new shell), `version`, `json`, `update`
/// itself and any run with a `--json` argument never do.
pub fn applies(command: &Command, args: &[OsString]) -> bool {
    let skipped = matches!(
        command,
        Command::Hook { .. }
            | Command::Statusline
            | Command::ShellInit { .. }
            | Command::Version
            | Command::Json { .. }
            | Command::Update { .. }
    );
    !skipped
        && !args.iter().any(|a| {
            a.to_str()
                .is_some_and(|s| s == "--json" || s.starts_with("--json="))
        })
}

/// CI vendor variables. `CI` and `PLAYBOOK_HEADLESS` go through
/// `common::headless`, the switch every hook uses.
const CI_VARS: [&str; 6] = [
    "GITHUB_ACTIONS",
    "GITLAB_CI",
    "BUILDKITE",
    "CIRCLECI",
    "JENKINS_URL",
    "TF_BUILD",
];

/// Whether a CI vendor variable is set to anything other than empty, `0` or
/// `false`.
pub fn ci_vendor_env(get: impl Fn(&str) -> Option<String>) -> bool {
    CI_VARS.iter().any(|name| {
        get(name).is_some_and(|v| {
            let v = v.trim().to_ascii_lowercase();
            !v.is_empty() && v != "0" && v != "false"
        })
    })
}

/// The facts about this run that the plan needs.
#[derive(Debug, Clone)]
pub struct Probe {
    pub current: String,
    pub now: u64,
    pub stderr_tty: bool,
    pub homebrew: bool,
}

/// A notice to print, and the version it names.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Notice {
    pub version: String,
    pub line: String,
}

/// What this run should do.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Plan {
    /// Start a detached refresh and record the start time.
    pub refresh: bool,
    /// The `checked_at` the plan saw, so only one process claims a refresh.
    pub seen_checked_at: u64,
    /// Print this line on stderr and record it.
    pub notice: Option<Notice>,
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

/// Decides what to do from the settings, the cached row and this run. A
/// run without a terminal on stderr neither refreshes nor prints, since
/// nobody would see the notice. `auto` behaves as `notify` here.
pub fn plan(settings: Settings, cache: &Cache, probe: &Probe) -> Plan {
    if settings.mode == UpdateMode::Off || !probe.stderr_tty {
        return Plan::default();
    }
    let age = probe.now.saturating_sub(cache.checked_at);
    let notice = cache
        .latest
        .as_deref()
        .filter(|latest| compare(latest, &probe.current).is_gt())
        .filter(|latest| !cache.shown_recently(latest, probe.now))
        .map(|latest| Notice {
            version: latest.to_string(),
            line: notice_text(latest, &probe.current, probe.homebrew),
        });
    Plan {
        refresh: cache.checked_at > probe.now || age >= settings.interval_secs,
        seen_checked_at: cache.checked_at,
        notice,
    }
}

/// The newest plain release in a `GET /releases` body, with its publish
/// time. Drafts, pre-releases and plugin or telemetry tags are skipped.
pub fn newest_plain(body: &str) -> Result<Option<(String, Option<String>)>, String> {
    Ok(resolve::parse_releases(body)?
        .into_iter()
        .filter(resolve::Release::is_plain_stable)
        .max_by(|a, b| compare(a.version(), b.version()))
        .map(|r| (r.version().to_string(), r.published_at)))
}

/// Fetches the release list and stores the newest plain release. On an
/// error nothing is stored; the start time the caller recorded makes the
/// next try wait one interval.
pub fn refresh(root: &Path, fetcher: &dyn Fetcher, api_base: &str) -> Result<(), String> {
    let body = fetcher.get(&format!("{api_base}{}", resolve::RELEASES_PAGE))?;
    let text = String::from_utf8(body).map_err(|_| "release data is not text".to_string())?;
    let Some((latest, published)) = newest_plain(&text)? else {
        return Ok(());
    };
    Cache::update(root, |cache| {
        cache.latest = Some(latest);
        cache.published_at = published;
    })
}

/// Applies `plan`. Each action is claimed inside one transaction against
/// the current row, so of two commands that start together only one spawns
/// the refresh and only one prints. A store that cannot be written stops
/// both. `spawn` starts the refresh and `print` writes the notice.
pub fn execute(root: &Path, plan: &Plan, now: u64, spawn: &dyn Fn(), print: &mut dyn FnMut(&str)) {
    if !plan.refresh && plan.notice.is_none() {
        return;
    }
    let claimed = Cache::update(root, |cache| {
        let refresh = plan.refresh && cache.checked_at == plan.seen_checked_at;
        if refresh {
            cache.checked_at = now;
        }
        let notice = plan
            .notice
            .as_ref()
            .filter(|n| !cache.shown_recently(&n.version, now));
        if let Some(n) = notice {
            cache.notified = Some(format!("{}@{now}", n.version));
        }
        (refresh, notice.map(|n| n.line.clone()))
    });
    let Ok((refresh, notice)) = claimed else {
        return;
    };
    if refresh {
        spawn();
    }
    if let Some(line) = notice {
        print(&line);
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

/// The repo slug for config lookups, only when a repo or org tier exists,
/// so most runs skip reading the git config.
fn config_slug(root: &Path) -> Option<String> {
    if !crate::config::any_scoped_config(root) {
        return None;
    }
    Some(crate::common::repo_slug()).filter(|s| !s.is_empty())
}

/// The entry point `main` calls before it dispatches a command. It never
/// fails and never waits on the network.
pub fn on_start(command: &Command, args: &[OsString]) {
    use std::io::IsTerminal;
    // The cheap checks come first, so a piped run never opens config.db.
    let stderr_tty = std::io::stderr().is_terminal();
    if !stderr_tty
        || !applies(command, args)
        || crate::common::headless::is_headless()
        || ci_vendor_env(|name| std::env::var(name).ok())
    {
        return;
    }
    let home = crate::common::home_dir();
    let root = crate::common::paths::playbook_root_from(&home);
    let settings = Settings::read(&home, config_slug(&root).as_deref());
    if settings.mode == UpdateMode::Off {
        return;
    }
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
    let fetcher = super::download::HttpFetcher {
        local_http: false,
        timeout: REFRESH_TIMEOUT,
    };
    let _ = refresh(&root, &fetcher, &super::api_base());
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::common::test_support::scratch_dir;
    use crate::update::download::fake::FakeFetcher;
    use clap::Parser;
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

    fn settings(mode: UpdateMode) -> Settings {
        Settings {
            mode,
            interval_secs: 24 * HOUR,
        }
    }

    fn notify() -> Settings {
        settings(UpdateMode::Notify)
    }

    fn probe(now: u64) -> Probe {
        Probe {
            current: "0.26.0".into(),
            now,
            stderr_tty: true,
            homebrew: false,
        }
    }

    /// Runs one command start against `root` and returns the printed lines
    /// and how many refreshes it started.
    fn start(root: &Path, settings: Settings, probe: &Probe) -> (Vec<String>, u32) {
        let cache = Cache::read(root).unwrap();
        run_plan(root, &plan(settings, &cache, probe), probe.now)
    }

    fn run_plan(root: &Path, plan: &Plan, now: u64) -> (Vec<String>, u32) {
        let spawned = Cell::new(0);
        let mut printed = Vec::new();
        execute(
            root,
            plan,
            now,
            &|| spawned.set(spawned.get() + 1),
            &mut |line| printed.push(line.to_string()),
        );
        (printed, spawned.get())
    }

    fn fetched(root: &Path) {
        let mut f = FakeFetcher::default();
        f.0.insert(format!("{API}{}", resolve::RELEASES_PAGE), LIST.into());
        refresh(root, &f, API).unwrap();
    }

    fn line(version: &str) -> String {
        format!("playbook {version} is available (you have 0.26.0). Run: playbook update")
    }

    #[test]
    fn only_plain_stable_tags_count_as_the_latest_release() {
        let (version, published) = newest_plain(LIST).unwrap().unwrap();
        assert_eq!(version, "0.27.0");
        assert_eq!(published.as_deref(), Some("2026-10-15T00:37:06Z"));
        assert_eq!(newest_plain("[]").unwrap(), None);
        assert!(newest_plain("not json").is_err());
    }

    #[test]
    fn the_notice_prints_once_a_day_for_each_version() {
        // Arrange
        let root = root("check-once");
        fetched(&root);
        Cache::update(&root, |c| c.checked_at = NOW).unwrap();

        // Act
        let (first, _) = start(&root, notify(), &probe(NOW));
        let (same_day, _) = start(&root, notify(), &probe(NOW + 23 * HOUR));
        Cache::update(&root, |c| c.latest = Some("0.28.0".into())).unwrap();
        let (newer, _) = start(&root, notify(), &probe(NOW + 23 * HOUR));
        let (next_day, _) = start(&root, notify(), &probe(NOW + 23 * HOUR + DAY_SECS));

        // Assert
        assert_eq!(first, vec![line("0.27.0")]);
        assert!(same_day.is_empty(), "{same_day:?}");
        assert_eq!(newer, vec![line("0.28.0")]);
        assert_eq!(next_day, vec![line("0.28.0")]);
    }

    #[test]
    fn auto_mode_notifies_like_notify_for_now() {
        let root = root("check-auto");
        fetched(&root);
        assert_eq!(
            start(&root, settings(UpdateMode::Auto), &probe(NOW)),
            (vec![line("0.27.0")], 1)
        );
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
    fn two_commands_that_start_together_claim_each_action_once() {
        // Arrange
        let root = root("check-race");
        fetched(&root);
        let cache = Cache::read(&root).unwrap();
        let first = plan(notify(), &cache, &probe(NOW));
        let second = plan(notify(), &cache, &probe(NOW));

        // Act
        let one = run_plan(&root, &first, NOW);
        let two = run_plan(&root, &second, NOW);

        // Assert
        assert_eq!(one, (vec![line("0.27.0")], 1));
        assert_eq!(two, (vec![], 0));
    }

    #[test]
    fn a_store_that_cannot_be_written_spawns_and_prints_nothing() {
        let file = root("check-broken").join("not-a-dir");
        std::fs::write(&file, "x").unwrap();
        let plan = Plan {
            refresh: true,
            seen_checked_at: 0,
            notice: Some(Notice {
                version: "9.0.0".into(),
                line: "x".into(),
            }),
        };
        assert_eq!(run_plan(&file, &plan, NOW), (vec![], 0));
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
        let mut piped = probe(NOW);
        piped.stderr_tty = false;
        assert_eq!(
            start(&root, settings(UpdateMode::Off), &probe(NOW)),
            (vec![], 0)
        );
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

    fn applies_to(parts: &[&str]) -> bool {
        let args: Vec<OsString> = parts.iter().map(OsString::from).collect();
        let cli = crate::Cli::try_parse_from(&args).unwrap();
        applies(&cli.command, &args)
    }

    #[test]
    fn hooks_statusline_json_and_update_never_run_the_check() {
        for parts in [
            &["playbook", "hook", "session-init"][..],
            &["playbook", "statusline"],
            &["playbook", "shell-init"],
            &["playbook", "json", "memory-context", "g.json", "o/r"],
            &["playbook", "mode", "status", "--json"],
            &["playbook", "update", "--check"],
            &["playbook", "upgrade"],
            &["playbook", "version"],
        ] {
            assert!(!applies_to(parts), "{parts:?}");
        }
        assert!(applies_to(&["playbook", "doctor"]));
        assert!(applies_to(&["playbook", "mode", "status"]));
    }

    #[test]
    fn ci_vendor_variables_turn_the_check_off() {
        let env = |pairs: &'static [(&'static str, &'static str)]| {
            move |name: &str| {
                pairs
                    .iter()
                    .find(|(k, _)| *k == name)
                    .map(|(_, v)| v.to_string())
            }
        };
        assert!(ci_vendor_env(env(&[("GITHUB_ACTIONS", "true")])));
        assert!(ci_vendor_env(env(&[("JENKINS_URL", "https://ci")])));
        assert!(!ci_vendor_env(env(&[("GITLAB_CI", "false")])));
        assert!(!ci_vendor_env(env(&[("BUILDKITE", "")])));
        assert!(!ci_vendor_env(env(&[])));
    }

    #[test]
    fn mode_values_parse_with_notify_as_the_fallback() {
        assert_eq!(UpdateMode::parse("off"), UpdateMode::Off);
        assert_eq!(UpdateMode::parse("auto"), UpdateMode::Auto);
        assert_eq!(UpdateMode::parse("notify"), UpdateMode::Notify);
        assert_eq!(UpdateMode::parse("bogus"), UpdateMode::Notify);
    }

    #[test]
    fn the_settings_come_from_config_and_a_zero_interval_is_the_default() {
        // Arrange
        let home = root("check-settings");
        let before = Settings::read(&home, None);
        let root = crate::common::paths::playbook_root_from(&home);
        let global = crate::config::write::Tier::Global;
        let put = |key: &str, value: serde_json::Value| {
            crate::config::store::put(&root, global, None, key, &value).unwrap()
        };

        // Act
        put("update.checkIntervalHours", 6.into());
        put("update.mode", "off".into());
        let set = Settings::read(&home, None);
        put("update.checkIntervalHours", 0.into());
        let zero = Settings::read(&home, None);

        // Assert
        assert_eq!(before, notify());
        assert_eq!(
            set,
            Settings {
                mode: UpdateMode::Off,
                interval_secs: 6 * HOUR
            }
        );
        assert_eq!(zero.interval_secs, 24 * HOUR);
    }
}
