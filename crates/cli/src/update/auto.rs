// SPDX-FileCopyrightText: 2026 Igor Santos
// SPDX-License-Identifier: Apache-2.0

//! `update.mode = auto`: the detached `update --refresh-check` installs a
//! newer release through the same verified path as `playbook update`, then
//! leaves a report in the cache for the next command to print. The command
//! that started it keeps running on the old binary. Any reason not to update
//! falls back to the notice.

use super::check::{stamp, Cache, Held, Settings, UpdateMode};
use super::download::Fetcher;
use super::verify::Attest;
use super::{Env, Options, Outcome};
use crate::breaking::{self, Component, Entry};
use std::cell::RefCell;
use std::path::{Path, PathBuf};
use std::time::Duration;

/// The lock both `playbook update` and the auto update hold while they run.
pub const LOCK_DIR: &str = "update.lock";

/// A lock this old belongs to a holder that died. A download, the checks
/// and the heal stay well under this.
const LOCK_STALE: Duration = Duration::from_secs(30 * 60);

/// A release must be this old before auto installs it, so a release that is
/// pulled right after it ships never reaches anyone automatically.
pub const MIN_RELEASE_AGE_SECS: u64 = 3600;

/// How long a busy lock holds auto back before it tries again.
const BUSY_RETRY_SECS: u64 = 3600;

/// How long a hold lasts when its cause can go away on its own, such as a
/// network error or a missing `gh`.
const DAY_HOLD_SECS: u64 = 24 * 3600;

/// The release asset that lists breaking changes (ADR 0035).
const BREAKING_ASSET: &str = "breaking-changes.json";

/// The facts about this install that decide whether auto may run.
#[derive(Debug, Clone)]
pub struct Install {
    pub homebrew: bool,
    pub dev_build: bool,
    pub dir_writable: bool,
    /// `playbook update` can replace a binary on this platform.
    pub supported: bool,
}

impl Install {
    /// The running binary. A debug build or a version with a pre-release
    /// part counts as a development build.
    pub fn real(exe: &Path, current: &str) -> Install {
        let os = std::env::consts::OS;
        Install {
            homebrew: super::is_homebrew(exe),
            dev_build: Install::is_dev_build(current),
            dir_writable: exe.parent().is_some_and(crate::common::paths::is_writable),
            supported: !cfg!(windows)
                && super::resolve::asset_name(current, os, std::env::consts::ARCH).is_ok(),
        }
    }

    /// Whether a binary of version `current` is a development build, which
    /// never updates itself.
    pub fn is_dev_build(current: &str) -> bool {
        cfg!(debug_assertions) || current.contains('-')
    }
}

/// Why this install can never update itself, whatever the release.
pub fn install_reason(install: &Install) -> Option<&'static str> {
    if install.homebrew {
        Some("Homebrew manages this install")
    } else if !install.supported {
        Some("playbook update does not support this platform")
    } else if install.dev_build {
        Some("this is a development or pre-release build")
    } else if !install.dir_writable {
        Some("the install directory is not writable")
    } else {
        None
    }
}

/// Why auto falls back to the notice, from what any process can see: the
/// install, the release time and a hold the detached process recorded.
pub fn hold_reason(install: &Install, cache: &Cache, latest: &str, now: u64) -> Option<String> {
    if let Some(reason) = install_reason(install) {
        return Some(reason.into());
    }
    let published = cache
        .published_at
        .as_deref()
        .and_then(crate::common::time::parse_iso8601_utc_epoch)
        .and_then(|at| u64::try_from(at).ok());
    match published {
        None => return Some("the release time is unknown".into()),
        Some(at) if now < at.saturating_add(MIN_RELEASE_AGE_SECS) => {
            return Some("the release is less than an hour old".into())
        }
        Some(_) => {}
    }
    cache.held_reason(latest, now).map(str::to_string)
}

/// Holds `update.lock` under the playbook root until dropped.
pub struct UpdateLock(PathBuf);

impl Drop for UpdateLock {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir(&self.0);
    }
}

/// Why the update lock was not taken.
#[derive(Debug, PartialEq, Eq)]
pub enum LockError {
    /// Another process holds it.
    Busy,
    /// The playbook root could not be prepared.
    Io(String),
}

impl std::fmt::Display for LockError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            LockError::Busy => write!(
                f,
                "another playbook process is updating; try again in a few minutes"
            ),
            LockError::Io(err) => write!(f, "cannot take the update lock: {err}"),
        }
    }
}

/// Takes the update lock without waiting.
pub fn try_lock(root: &Path) -> Result<UpdateLock, LockError> {
    crate::common::atomic::ensure_private_dir(root)
        .map_err(|e| LockError::Io(format!("{}: {e}", root.display())))?;
    let path = root.join(LOCK_DIR);
    if crate::common::atomic::acquire_dir_lock_aged(&path, 1, Duration::ZERO, LOCK_STALE) {
        Ok(UpdateLock(path))
    } else {
        Err(LockError::Busy)
    }
}

/// Whether a fetch error is a plain not found, which for this asset means
/// a release from before ADR 0035 attached it.
fn is_not_found(err: &str) -> bool {
    err.contains("404")
}

/// The playbook entries between `from` and `to` from the release's own
/// `breaking-changes.json`, checked against its SHA256SUMS. The running
/// binary cannot know about later releases, so the record comes from the
/// target release. Only a release without the asset has no known entries;
/// any other failure, a playbook entry this binary cannot read included, is
/// an error so auto holds the release.
pub fn breaking_entries(
    fetcher: &dyn Fetcher,
    download_base: &str,
    from: &str,
    to: &str,
) -> Result<Vec<Entry>, String> {
    let base = format!("{download_base}/v{to}");
    let body = match fetcher.get(&format!("{base}/{BREAKING_ASSET}")) {
        Ok(body) => body,
        Err(err) if is_not_found(&err) => return Ok(Vec::new()),
        Err(err) => return Err(err),
    };
    let sums = fetcher.get(&format!("{base}/SHA256SUMS"))?;
    super::verify::verify_checksum(&body, &String::from_utf8_lossy(&sums), BREAKING_ASSET)?;
    let doc: serde_json::Value =
        serde_json::from_slice(&body).map_err(|e| format!("{BREAKING_ASSET}: {e}"))?;
    let raw = doc["entries"]
        .as_array()
        .ok_or_else(|| format!("{BREAKING_ASSET} has no entries list"))?;
    let entries = breaking::lenient(raw);
    let playbook = |c: &serde_json::Value| c.as_str() == Some(Component::Playbook.name());
    let raw_playbook = raw.iter().filter(|v| playbook(&v["component"])).count();
    let read_playbook = entries
        .iter()
        .filter(|e| e.component == Component::Playbook)
        .count();
    if read_playbook != raw_playbook {
        return Err(format!(
            "{BREAKING_ASSET} has a playbook entry this version cannot read"
        ));
    }
    Ok(breaking::select(&entries, Component::Playbook, from, to))
}

/// What the detached process uses to install. Tests pass fakes.
pub struct Deps<'a> {
    pub env: &'a Env,
    pub fetcher: &'a dyn Fetcher,
    pub attest: &'a dyn Attest,
    pub smoke: &'a dyn Fn(&Path) -> Result<String, String>,
    pub heal: &'a dyn Fn(&mut Outcome, &Path),
    pub install: Install,
    pub now: u64,
}

/// An attestation check with no person present: a check that could not run
/// counts as a failure, and the gap is kept so `apply` can hold the release.
struct Unattended<'a> {
    inner: &'a dyn Attest,
    gap: RefCell<Option<String>>,
}

impl Attest for Unattended<'_> {
    fn verify(&self, file: &Path, strict: bool) -> Result<Option<String>, String> {
        match self.inner.verify(file, strict)? {
            None => Ok(None),
            Some(gap) => {
                *self.gap.borrow_mut() = Some(gap.clone());
                Err(gap)
            }
        }
    }
}

/// Runs the auto update when `update.mode` is `auto`, the cache names a
/// newer release and nothing holds it back. One version is tried at most
/// once a day. The result goes into the cache for the next command to
/// print; nothing is printed here.
pub fn apply(root: &Path, settings: Settings, deps: &Deps) {
    let current = deps.env.current_version.as_str();
    let Ok(cache) = Cache::read(root) else {
        return;
    };
    let Some(latest) = cache.newer_than(current).map(str::to_string) else {
        return;
    };
    if settings.mode != UpdateMode::Auto
        || hold_reason(&deps.install, &cache, &latest, deps.now).is_some()
    {
        return;
    }
    let hold = |reason: &str, until: u64| {
        let _ = Cache::update(root, |c| {
            c.held = Some(Held {
                version: latest.clone(),
                reason: reason.to_string(),
                until,
            })
        });
    };
    let _lock = match try_lock(root) {
        Ok(lock) => lock,
        // An auto update already working on this version holds the lock;
        // only a manual `playbook update` is worth a hold.
        Err(LockError::Busy) if cache.tried_recently(&latest, deps.now) => return,
        Err(LockError::Busy) => {
            hold(
                "another playbook process is updating",
                deps.now + BUSY_RETRY_SECS,
            );
            return;
        }
        Err(LockError::Io(_)) => return,
    };
    let claimed = Cache::update(root, |c| {
        let free = !c.tried_recently(&latest, deps.now);
        if free {
            c.tried = Some(stamp(&latest, deps.now));
        }
        free
    });
    if claimed != Ok(true) {
        return;
    }
    let entries = match breaking_entries(deps.fetcher, &deps.env.download_base, current, &latest) {
        Ok(entries) => entries,
        Err(err) => {
            hold(
                &format!("its breaking changes could not be read ({err})"),
                deps.now + DAY_HOLD_SECS,
            );
            return;
        }
    };
    if entries.iter().any(Entry::needs_user) {
        hold("this release has breaking changes that need you", u64::MAX);
        return;
    }
    let opts = Options {
        version: Some(latest.clone()),
        yes: true,
        ..Options::default()
    };
    let attest = Unattended {
        inner: deps.attest,
        gap: RefCell::new(None),
    };
    let report = match super::run(&opts, deps.env, deps.fetcher, &attest, deps.smoke) {
        Ok(mut outcome) if outcome.installed.is_some() => {
            (deps.heal)(&mut outcome, &deps.env.exe);
            success_report(current, &latest, &entries, &outcome)
        }
        Ok(_) => return,
        Err(_) if attest.gap.borrow().is_some() => {
            hold(
                "the build attestation could not be checked; install gh and run gh auth login",
                deps.now + DAY_HOLD_SECS,
            );
            return;
        }
        Err(err) => vec![failure_line(&latest, &err)],
    };
    // A warning from an earlier failed try at this version is replaced, so
    // runs without a terminal do not pile up one warning a day.
    let earlier = failure_prefix(&latest);
    let _ = Cache::update(root, |c| {
        c.report.retain(|line| !line.starts_with(&earlier));
        c.report.extend(report);
    });
}

/// The real dependencies, for the detached process.
pub fn run_detached(root: &Path, settings: Settings) {
    let Ok(env) = Env::real() else {
        return;
    };
    let fetcher = super::download::HttpFetcher {
        local_http: false,
        timeout: super::download::DOWNLOAD_TIMEOUT,
    };
    let deps = Deps {
        env: &env,
        fetcher: &fetcher,
        attest: &super::verify::GhAttest,
        smoke: &super::swap::run_version,
        heal: &super::finish_with_heal,
        install: Install::real(&env.exe, &env.current_version),
        now: crate::common::time::now_epoch_secs(),
    };
    apply(root, settings, &deps);
}

fn success_report(from: &str, to: &str, entries: &[Entry], outcome: &Outcome) -> Vec<String> {
    let mut lines = vec![format!("playbook updated from {from} to {to}")];
    lines.extend(breaking::render(entries).lines().map(str::to_string));
    lines.extend(outcome.warnings.iter().map(|w| format!("warning: {w}")));
    lines
}

/// How every failure line for `version` starts.
fn failure_prefix(version: &str) -> String {
    format!("warning: playbook could not update itself to {version}:")
}

/// The one warning a failed auto update leaves, with the fix.
pub fn failure_line(version: &str, err: &str) -> String {
    format!(
        "{} {}. Run: playbook update, or set update.mode to notify",
        failure_prefix(version),
        err.trim_end_matches('.')
    )
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;
    use crate::common::test_support::scratch_dir;
    use crate::update::download::fake::FakeFetcher;
    use crate::update::swap;
    use crate::update::verify::{self, fake::FakeAttest};

    const API: &str = "https://api.test/repos/x";
    const DL: &str = "https://dl.test";
    const ASSET: &str = "playbook-0.17.0-x86_64-unknown-linux-musl";
    /// 2026-10-15T00:37:06Z
    const PUBLISHED: u64 = 1_792_024_626;
    const LATER: u64 = PUBLISHED + 2 * 3600;

    fn ok_install() -> Install {
        Install {
            homebrew: false,
            dev_build: false,
            dir_writable: true,
            supported: true,
        }
    }

    fn auto() -> Settings {
        Settings {
            mode: UpdateMode::Auto,
            interval_secs: 24 * 3600,
        }
    }

    fn script(version: &str) -> Vec<u8> {
        format!("#!/bin/sh\necho 'playbook {version}'\n").into_bytes()
    }

    /// A home with an installed 0.16.0 script and a cache that names 0.17.0.
    fn setup(tag: &str) -> (PathBuf, Env, FakeFetcher) {
        let dir = scratch_dir(tag);
        std::fs::create_dir_all(dir.join("bin")).unwrap();
        let exe = dir.join("bin/playbook");
        std::fs::write(&exe, script("0.16.0")).unwrap();
        swap::make_executable(&exe).unwrap();
        let env = Env {
            exe,
            path_var: dir.join("bin").into_os_string(),
            auto_mode: false,
            strict: false,
            windows: false,
            os: "linux".into(),
            arch: "x86_64".into(),
            current_version: "0.16.0".into(),
            api_base: API.into(),
            download_base: DL.into(),
            announce: |_| {},
        };
        let bin = script("0.17.0");
        let mut f = FakeFetcher::default();
        f.0.insert(
            format!("{API}/releases/tags/v0.17.0"),
            br#"{"tag_name":"v0.17.0"}"#.to_vec(),
        );
        f.0.insert(format!("{DL}/v0.17.0/{ASSET}"), bin.clone());
        f.0.insert(
            format!("{DL}/v0.17.0/SHA256SUMS"),
            format!("{}  {ASSET}\n", verify::sha256_hex(&bin)).into_bytes(),
        );
        let root = dir.join("root");
        Cache::update(&root, |c| {
            c.latest = Some("0.17.0".into());
            c.published_at = Some("2026-10-15T00:37:06Z".into());
        })
        .unwrap();
        (root, env, f)
    }

    /// Attaches a breaking changes record to the fake 0.17.0 release, with
    /// its line in SHA256SUMS.
    fn with_record(f: &mut FakeFetcher, entry: &str) {
        let record = format!(r#"{{"entries":[{entry}]}}"#).into_bytes();
        let bin = script("0.17.0");
        let sums = format!(
            "{}  {ASSET}\n{}  {BREAKING_ASSET}\n",
            verify::sha256_hex(&bin),
            verify::sha256_hex(&record)
        );
        f.0.insert(format!("{DL}/v0.17.0/{BREAKING_ASSET}"), record);
        f.0.insert(format!("{DL}/v0.17.0/SHA256SUMS"), sums.into_bytes());
    }

    fn run_with(root: &Path, env: &Env, fetcher: &FakeFetcher, attest: &FakeAttest, now: u64) {
        let deps = Deps {
            env,
            fetcher,
            attest,
            smoke: &swap::run_version,
            heal: &|_, _| {},
            install: ok_install(),
            now,
        };
        apply(root, auto(), &deps);
    }

    fn run_apply(root: &Path, env: &Env, fetcher: &FakeFetcher) {
        run_with(root, env, fetcher, &FakeAttest::new(Ok(None)), LATER);
    }

    fn installed(env: &Env) -> String {
        swap::run_version(&env.exe).unwrap()
    }

    #[test]
    fn each_local_reason_holds_auto_back() {
        let cache = Cache {
            latest: Some("0.17.0".into()),
            published_at: Some("2026-10-15T00:37:06Z".into()),
            ..Cache::default()
        };
        let with = |f: fn(&mut Install)| {
            let mut i = ok_install();
            f(&mut i);
            hold_reason(&i, &cache, "0.17.0", LATER).unwrap()
        };
        assert_eq!(hold_reason(&ok_install(), &cache, "0.17.0", LATER), None);
        assert!(with(|i| i.homebrew = true).contains("Homebrew"));
        assert!(with(|i| i.supported = false).contains("platform"));
        assert!(with(|i| i.dev_build = true).contains("development"));
        assert!(with(|i| i.dir_writable = false).contains("not writable"));
        let young = hold_reason(&ok_install(), &cache, "0.17.0", PUBLISHED + 59 * 60).unwrap();
        assert!(young.contains("less than an hour"), "{young}");
        let unknown = Cache {
            published_at: None,
            ..cache.clone()
        };
        assert!(hold_reason(&ok_install(), &unknown, "0.17.0", LATER).is_some());
    }

    #[test]
    fn auto_swaps_the_binary_and_leaves_a_from_to_report() {
        // Arrange
        let (root, env, fetcher) = setup("auto-swap");

        // Act
        run_apply(&root, &env, &fetcher);

        // Assert
        assert_eq!(installed(&env), "playbook 0.17.0");
        let cache = Cache::read(&root).unwrap();
        assert_eq!(cache.report, vec!["playbook updated from 0.16.0 to 0.17.0"]);
        assert!(cache.tried.unwrap().starts_with("0.17.0@"));
    }

    #[test]
    fn a_new_report_is_added_to_one_nobody_has_seen() {
        let (root, env, fetcher) = setup("auto-append");
        Cache::update(&root, |c| c.report = vec!["earlier".into()]).unwrap();
        run_apply(&root, &env, &fetcher);
        assert_eq!(
            Cache::read(&root).unwrap().report,
            vec!["earlier", "playbook updated from 0.16.0 to 0.17.0"]
        );
    }

    #[test]
    fn breaking_changes_that_need_the_user_hold_the_update() {
        // Arrange
        let (root, env, mut fetcher) = setup("auto-breaking");
        with_record(
            &mut fetcher,
            r#"{"version":"0.17.0","component":"playbook",
                "summary":"The hook layout changed","action":"playbook setup"}"#,
        );

        // Act
        run_apply(&root, &env, &fetcher);

        // Assert
        assert_eq!(installed(&env), "playbook 0.16.0");
        let cache = Cache::read(&root).unwrap();
        assert!(cache
            .held_reason("0.17.0", LATER)
            .unwrap()
            .contains("breaking changes that need you"));
        assert!(cache.report.is_empty());
    }

    #[test]
    fn automatic_breaking_changes_install_and_show_in_the_report() {
        let (root, env, mut fetcher) = setup("auto-breaking-ok");
        with_record(
            &mut fetcher,
            r#"{"version":"0.17.0","component":"playbook",
                "summary":"The state moved to config.db","action":"none, migrated automatically",
                "migration":"0099-state"}"#,
        );
        run_apply(&root, &env, &fetcher);
        let report = Cache::read(&root).unwrap().report.join("\n");
        assert!(report.contains("updated from 0.16.0 to 0.17.0"), "{report}");
        assert!(report.contains("The state moved to config.db"), "{report}");
    }

    #[test]
    fn a_record_that_cannot_be_trusted_holds_the_update() {
        for (tag, change) in [
            ("auto-record-sum", 0),
            ("auto-record-entry", 1),
            ("auto-record-net", 2),
        ] {
            // Arrange
            let (root, env, mut fetcher) = setup(tag);
            with_record(
                &mut fetcher,
                r#"{"version":"0.17.0","component":"playbook","summary":"x","action":"none, migrated automatically","migration":"m"}"#,
            );
            let url = format!("{DL}/v0.17.0/{BREAKING_ASSET}");
            match change {
                0 => {
                    fetcher.0.insert(url, br#"{"entries":[]}"#.to_vec());
                }
                1 => with_record(
                    &mut fetcher,
                    r#"{"version":"0.17.0","component":"playbook","summary":"x","action":"none, migrated automatically"}"#,
                ),
                _ => {
                    fetcher.0.remove(&format!("{DL}/v0.17.0/SHA256SUMS"));
                }
            }

            // Act
            run_apply(&root, &env, &fetcher);

            // Assert
            assert_eq!(installed(&env), "playbook 0.16.0", "{tag}");
            let reason = Cache::read(&root)
                .unwrap()
                .held_reason("0.17.0", LATER)
                .map(str::to_string);
            assert!(
                reason
                    .as_deref()
                    .is_some_and(|r| r.contains("could not be read")),
                "{tag}: {reason:?}"
            );
        }
    }

    #[test]
    fn a_missing_attestation_holds_the_update_without_a_swap() {
        let (root, env, fetcher) = setup("auto-attest");
        let gap = FakeAttest::new(Ok(Some("checksum only, provenance not verified".into())));
        run_with(&root, &env, &fetcher, &gap, LATER);
        assert_eq!(installed(&env), "playbook 0.16.0");
        let cache = Cache::read(&root).unwrap();
        assert!(cache
            .held_reason("0.17.0", LATER)
            .unwrap()
            .contains("attestation"));
        assert!(cache.held_reason("0.17.0", LATER + DAY_HOLD_SECS).is_none());
        assert!(cache.report.is_empty());
    }

    #[test]
    fn a_homebrew_binary_is_never_swapped() {
        let (root, env, fetcher) = setup("auto-brew");
        let attest = FakeAttest::new(Ok(None));
        let mut install = ok_install();
        install.homebrew = true;
        let deps = Deps {
            env: &env,
            fetcher: &fetcher,
            attest: &attest,
            smoke: &swap::run_version,
            heal: &|_, _| {},
            install,
            now: LATER,
        };
        apply(&root, auto(), &deps);
        assert_eq!(installed(&env), "playbook 0.16.0");
        assert!(Cache::read(&root).unwrap().report.is_empty());
    }

    #[test]
    fn one_version_is_tried_once_a_day() {
        let (root, env, mut fetcher) = setup("auto-tried");
        fetcher
            .0
            .insert(format!("{DL}/v0.17.0/SHA256SUMS"), b"bad".to_vec());
        run_apply(&root, &env, &fetcher);
        run_with(
            &root,
            &env,
            &fetcher,
            &FakeAttest::new(Ok(None)),
            LATER + 3600,
        );
        assert_eq!(Cache::read(&root).unwrap().report.len(), 1);
    }

    #[test]
    fn a_manual_update_holding_the_lock_holds_auto_for_an_hour() {
        let (root, env, fetcher) = setup("auto-locked");
        let held = try_lock(&root).unwrap();
        run_apply(&root, &env, &fetcher);
        assert_eq!(installed(&env), "playbook 0.16.0");
        let cache = Cache::read(&root).unwrap();
        assert!(cache.held_reason("0.17.0", LATER).is_some());
        assert!(cache
            .held_reason("0.17.0", LATER + BUSY_RETRY_SECS)
            .is_none());
        assert_eq!(cache.tried, None, "the next try may run after the hold");
        assert_eq!(try_lock(&root).err(), Some(LockError::Busy));
        drop(held);
    }

    #[test]
    fn another_auto_run_on_the_same_version_leaves_no_hold() {
        let (root, env, fetcher) = setup("auto-locked-auto");
        Cache::update(&root, |c| c.tried = Some(stamp("0.17.0", LATER))).unwrap();
        let _held = try_lock(&root).unwrap();
        run_apply(&root, &env, &fetcher);
        assert_eq!(Cache::read(&root).unwrap().held, None);
    }

    #[test]
    fn a_failed_update_leaves_one_warning_with_the_fix() {
        let (root, env, mut fetcher) = setup("auto-fail");
        fetcher
            .0
            .insert(format!("{DL}/v0.17.0/SHA256SUMS"), b"bad".to_vec());
        run_apply(&root, &env, &fetcher);
        let report = Cache::read(&root).unwrap().report;
        assert_eq!(report.len(), 1, "{report:?}");
        assert!(report[0].starts_with("warning: playbook could not update itself to 0.17.0"));
        assert!(report[0].contains("Run: playbook update"));
        assert_eq!(installed(&env), "playbook 0.16.0");
    }

    #[test]
    fn a_failure_on_the_next_day_replaces_the_earlier_warning() {
        let (root, env, mut fetcher) = setup("auto-fail-twice");
        fetcher
            .0
            .insert(format!("{DL}/v0.17.0/SHA256SUMS"), b"bad".to_vec());
        Cache::update(&root, |c| c.report = vec!["earlier".into()]).unwrap();
        run_apply(&root, &env, &fetcher);
        run_with(
            &root,
            &env,
            &fetcher,
            &FakeAttest::new(Ok(None)),
            LATER + DAY_HOLD_SECS,
        );
        let report = Cache::read(&root).unwrap().report;
        assert_eq!(report.len(), 2, "{report:?}");
        assert_eq!(report[0], "earlier");
    }

    #[test]
    fn notify_mode_never_installs() {
        let (root, env, fetcher) = setup("auto-notify");
        let attest = FakeAttest::new(Ok(None));
        let deps = Deps {
            env: &env,
            fetcher: &fetcher,
            attest: &attest,
            smoke: &swap::run_version,
            heal: &|_, _| {},
            install: ok_install(),
            now: LATER,
        };
        let notify = Settings {
            mode: UpdateMode::Notify,
            ..auto()
        };
        apply(&root, notify, &deps);
        assert_eq!(installed(&env), "playbook 0.16.0");
    }
}
