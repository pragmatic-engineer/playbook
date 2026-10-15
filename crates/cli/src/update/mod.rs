// SPDX-FileCopyrightText: 2026 Igor Santos
// SPDX-License-Identifier: Apache-2.0

//! `playbook update`: fetch a release binary, verify it (SHA256SUMS, then the
//! build attestation), show the breaking changes it brings, swap it in with a
//! backup, then check PATH.
//! Install.sh and this share one policy; see `verify`.

pub mod auto;
pub mod check;
pub mod download;
pub mod pathcheck;
pub mod resolve;
pub mod style;
pub mod swap;
pub mod verify;

use download::Fetcher;
use std::ffi::OsString;
use std::path::{Path, PathBuf};
use verify::{Attest, REPO};

/// What the user asked for on the command line.
#[derive(Debug, Default, Clone)]
pub struct Options {
    pub version: Option<String>,
    pub check: bool,
    pub list: bool,
    pub pre: bool,
    pub yes: bool,
}

/// Everything the update reads from the machine, so tests can fake it.
pub struct Env {
    pub exe: PathBuf,
    pub path_var: OsString,
    pub auto_mode: bool,
    pub strict: bool,
    pub windows: bool,
    pub os: String,
    pub arch: String,
    pub current_version: String,
    pub api_base: String,
    pub download_base: String,
    /// Shows text to the user at once, before the swap: the breaking
    /// changes block.
    pub announce: fn(&str),
}

impl Env {
    /// The real machine. The running binary is canonicalised so a symlink is
    /// replaced at its target, not turned into a regular file.
    pub fn real() -> Result<Env, String> {
        let exe = std::env::current_exe()
            .map_err(|e| format!("cannot locate the running binary: {e}"))?;
        Ok(Env {
            exe: std::fs::canonicalize(&exe).unwrap_or(exe),
            path_var: std::env::var_os("PATH").unwrap_or_default(),
            auto_mode: crate::common::mode::resolve_for_hook().mode
                == crate::common::mode::Mode::Auto,
            strict: std::env::var("PLAYBOOK_REQUIRE_ATTESTATION").is_ok_and(|v| v == "1"),
            windows: cfg!(windows),
            os: std::env::consts::OS.to_string(),
            arch: std::env::consts::ARCH.to_string(),
            current_version: env!("CARGO_PKG_VERSION").to_string(),
            api_base: api_base(),
            download_base: format!("https://github.com/{REPO}/releases/download"),
            announce: |text| eprint!("{text}"),
        })
    }
}

/// The GitHub API base for the release repo.
pub fn api_base() -> String {
    format!("https://api.github.com/repos/{REPO}")
}

/// Lines to print: `out` to stdout, `warnings` to stderr.
#[derive(Debug, Default)]
pub struct Outcome {
    pub out: Vec<String>,
    pub warnings: Vec<String>,
    /// The version swapped in, when the update replaced the binary.
    pub installed: Option<String>,
    /// The breaking changes block of the new binary, shown again after the
    /// swap and the heal. Empty when the upgrade crosses none.
    pub breaking: String,
    /// The version the shown block started from, set where the block is
    /// announced. The heal gets it so it does not print the block again.
    pub breaking_shown_since: Option<String>,
}

/// The first release whose binary has `doctor --fix`; `migrate` comes in the
/// same or a later one. The `-0` sorts below every 0.27.0 pre-release, so a
/// release candidate counts too.
pub const FIRST_FIX_VERSION: &str = "0.27.0-0";

/// How long `update` waits for the new binary's breaking change list.
const BREAKING_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(10);

/// Asks the binary at `exe` for the playbook entries newer than `since`
/// (ADR 0035), as JSON, and renders them with this binary. A binary without
/// the `breaking` command is older than this one, so a failed exit or output
/// that is not the JSON list means there is nothing to show. A binary that
/// did not start or did not answer in time is an error, so the update can say
/// the check did not run.
pub fn breaking_notice(exe: &Path, since: &str) -> Result<String, String> {
    let out = run_fresh(exe, &["breaking", "--since", since, "--json"], &[], BREAKING_TIMEOUT)
        .ok_or_else(|| {
            format!(
                "could not read the breaking changes of the new binary within {} s; run `playbook breaking --since {since}` after the update",
                BREAKING_TIMEOUT.as_secs()
            )
        })?;
    if !out.status.success() {
        return Ok(String::new());
    }
    let Ok(values) = serde_json::from_slice::<Vec<serde_json::Value>>(&out.stdout) else {
        return Ok(String::new());
    };
    Ok(crate::breaking::render(&crate::breaking::lenient(&values)))
}

/// How long `update` waits for the new binary's `migrate`.
const HEAL_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(120);

/// After a swap, runs `<exe> migrate` so the new binary runs its own
/// migrations (ADR 0034). A release that has `doctor --fix` but not
/// `migrate` answers with a usage error (exit 2), and gets `doctor --fix`
/// instead. Returns the command that ran and what it printed. Best effort: a
/// release older than `FIRST_FIX_VERSION` is skipped, and any failure is a
/// warning that names the command that ran and carries the lines it printed.
/// `shown_since` is the version the block the update printed started from.
/// The heal gets it in `breaking::SHOWN_ENV`, so it records the notice
/// without printing those entries again (ADR 0035 decision 4.2).
pub fn heal_after_swap(
    exe: &Path,
    installed: &str,
    shown_since: Option<&str>,
) -> Result<Option<(&'static str, String)>, String> {
    if resolve::compare(installed, FIRST_FIX_VERSION).is_lt() {
        return Ok(None);
    }
    let late = |name: &str| {
        format!(
            "the new binary's {name} did not run or took over {} s; run `playbook {name}` by hand",
            HEAL_TIMEOUT.as_secs()
        )
    };
    let mut name = "migrate";
    let mut out = run_heal(exe, &[name], shown_since).ok_or_else(|| late(name))?;
    if out.status.code() == Some(2) {
        name = "doctor --fix";
        out = run_heal(exe, &["doctor", "--fix"], shown_since).ok_or_else(|| late(name))?;
    }
    // The new binary prints the breaking changes block (ADR 0035) on stderr
    // and records it as shown, so it is passed on with the report.
    let text = format!(
        "{}{}",
        String::from_utf8_lossy(&out.stderr),
        String::from_utf8_lossy(&out.stdout)
    );
    if !out.status.success() {
        let detail: Vec<&str> = text
            .lines()
            .map(str::trim)
            .filter(|l| !l.is_empty())
            .collect();
        return Err(format!(
            "{name} exited with {}{}; run `playbook {name}` by hand",
            out.status,
            if detail.is_empty() {
                String::new()
            } else {
                format!(": {}", detail.join("; "))
            }
        ));
    }
    Ok(Some((name, text)))
}

/// Runs `<exe> <args>` with the heal deadline, telling it where the block
/// the update showed started. An empty value, rather than none, overrides a
/// value inherited from a shell that an older install.sh exported it into.
fn run_heal(exe: &Path, args: &[&str], shown_since: Option<&str>) -> Option<std::process::Output> {
    let shown = [(crate::breaking::SHOWN_ENV, shown_since.unwrap_or(""))];
    run_fresh(exe, args, &shown, HEAL_TIMEOUT)
}

/// Runs a freshly written binary with `envs` set and a deadline. A spawn that
/// fails at once (a fresh binary can be refused while another process still
/// holds it open) is tried again twice.
fn run_fresh(
    exe: &Path,
    args: &[&str],
    envs: &[(&str, &str)],
    timeout: std::time::Duration,
) -> Option<std::process::Output> {
    for _ in 0..3 {
        let started = std::time::Instant::now();
        let mut cmd = std::process::Command::new(exe);
        cmd.args(args)
            .envs(envs.iter().copied())
            .stdin(std::process::Stdio::null());
        if let Some(out) = crate::common::proc::run_with_timeout(&mut cmd, timeout) {
            return Some(out);
        }
        if started.elapsed() > std::time::Duration::from_secs(1) {
            return None;
        }
        std::thread::sleep(std::time::Duration::from_millis(100));
    }
    None
}

/// Runs the heal after a swap, when `outcome` names an installed version,
/// and adds its lines to `outcome`.
pub fn finish_with_heal(outcome: &mut Outcome, exe: &Path) {
    let Some(version) = outcome.installed.clone() else {
        return;
    };
    match heal_after_swap(exe, &version, outcome.breaking_shown_since.as_deref()) {
        Ok(Some((name, text))) => outcome
            .out
            .extend(text.lines().map(|l| format!("{name}: {l}"))),
        Ok(None) => {}
        Err(warning) => outcome.warnings.push(warning),
    }
}

pub fn is_homebrew(exe: &Path) -> bool {
    let text = exe.to_string_lossy();
    ["/Cellar/", "/opt/homebrew/", "/.linuxbrew/"]
        .iter()
        .any(|m| text.contains(m))
}

const MANUAL_STEPS: &str = "To update by hand, download the asset for your platform from \
https://github.com/pragmatic-engineer/playbook/releases/latest, check it against SHA256SUMS, \
and replace the binary, or re-run install.sh.";

fn refusals(env: &Env) -> Result<(), String> {
    if env.windows {
        return Err(format!(
            "playbook update does not support Windows yet. {MANUAL_STEPS}"
        ));
    }
    if is_homebrew(&env.exe) {
        return Err(format!(
            "{} is managed by Homebrew; update it there: brew upgrade playbook. Or install a \
             standalone copy with install.sh and put it first on PATH.",
            env.exe.display()
        ));
    }
    Ok(())
}

fn fetch_text(fetcher: &dyn Fetcher, url: &str) -> Result<String, String> {
    String::from_utf8(fetcher.get(url)?).map_err(|_| format!("{url}: response is not text"))
}

fn releases(
    env: &Env,
    fetcher: &dyn Fetcher,
    requested: Option<&str>,
) -> Result<Vec<resolve::Release>, String> {
    let url = match requested {
        Some(v) => format!(
            "{}/releases/tags/v{}",
            env.api_base,
            v.strip_prefix('v').unwrap_or(v)
        ),
        None => format!("{}{}", env.api_base, resolve::RELEASES_PAGE),
    };
    resolve::parse_releases(
        &fetch_text(fetcher, &url).map_err(|e| format!("could not read the release list: {e}"))?,
    )
}

/// Runs the update. Read-only for `--list` and `--check`.
pub fn run(
    opts: &Options,
    env: &Env,
    fetcher: &dyn Fetcher,
    attest: &dyn Attest,
    smoke: &dyn Fn(&Path) -> Result<String, String>,
) -> Result<Outcome, String> {
    let mut o = Outcome::default();
    let current = env.current_version.as_str();

    if opts.list {
        for r in releases(env, fetcher, None)? {
            let mut line = r.tag.clone();
            if r.prerelease {
                line.push_str(" (pre-release)");
            }
            if r.version() == current {
                line.push_str(" (installed)");
            }
            o.out.push(line);
        }
        return Ok(o);
    }

    let found = releases(env, fetcher, opts.version.as_deref())?;
    let release = resolve::select(&found, opts.version.as_deref(), opts.pre)?;
    let version = release.version().to_string();
    let order = resolve::compare(&version, current);

    if opts.check {
        o.out.push(match order {
            std::cmp::Ordering::Greater => {
                format!("update available: {current} -> {version} (run `playbook update`)")
            }
            _ => format!("playbook {current} is up to date (latest is {version})"),
        });
        return Ok(o);
    }

    if order.is_eq() {
        o.out
            .push(format!("playbook {current} is already installed"));
        return Ok(o);
    }
    if order.is_lt() && opts.version.is_none() {
        o.out.push(format!(
            "playbook {current} is newer than the latest release {version}; nothing to do"
        ));
        return Ok(o);
    }

    refusals(env)?;
    if env.auto_mode && !opts.yes {
        return Err("auto mode is on and an update replaces the running binary; re-run with --yes to confirm".into());
    }

    let asset = resolve::asset_name(&version, &env.os, &env.arch)?;
    let base = format!("{}/{}", env.download_base, release.tag);
    let bytes = fetcher
        .get(&format!("{base}/{asset}"))
        .map_err(|e| format!("could not download {asset}: {e}"))?;
    let sums = fetch_text(fetcher, &format!("{base}/SHA256SUMS"))
        .map_err(|e| format!("could not download SHA256SUMS: {e}"))?;
    verify::verify_checksum(&bytes, &sums, &asset)?;

    let dir = env
        .exe
        .parent()
        .ok_or("the running binary has no parent directory")?;
    let stage = swap::write_stage(dir, &bytes)
        .map_err(|e| format!("cannot write to {}: {e}", dir.display()))?;
    match attest.verify(&stage, env.strict) {
        Ok(None) => o
            .out
            .push(format!("verified the build attestation for {asset}")),
        Ok(Some(warning)) => o.warnings.push(warning),
        Err(err) => {
            let _ = std::fs::remove_file(&stage);
            return Err(err);
        }
    }

    if order.is_gt() {
        match breaking_notice(&stage, current) {
            Ok(text) => o.breaking = text,
            Err(warning) => o.warnings.push(warning),
        }
        if !o.breaking.is_empty() {
            (env.announce)(&o.breaking);
            o.breaking_shown_since = Some(current.to_string());
        }
    }

    let done = swap::install(&stage, &env.exe, current, &version, smoke)?;
    o.installed = Some(version.clone());
    o.warnings.extend(done.warnings);
    o.out.push(format!(
        "updated playbook {current} -> {version} at {}",
        env.exe.display()
    ));
    if let Some(bak) = done.backup {
        o.out.push(format!(
            "kept the previous binary as {}; to roll back: mv -f {} {}",
            bak.display(),
            bak.display(),
            env.exe.display()
        ));
    }
    o.warnings
        .extend(pathcheck::shadow_warning(&env.path_var, &env.exe));
    Ok(o)
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;
    use crate::common::test_support::scratch_dir;
    use download::fake::FakeFetcher;
    use verify::fake::FakeAttest;

    const API: &str = "https://api.test/repos/x";
    const DL: &str = "https://dl.test";

    fn script_bytes(version: &str) -> Vec<u8> {
        format!("#!/bin/sh\necho 'playbook {version}'\n").into_bytes()
    }

    fn env_in(tag: &str) -> Env {
        let dir = scratch_dir(tag);
        std::fs::create_dir_all(&dir).unwrap();
        let exe = dir.join("playbook");
        std::fs::write(&exe, script_bytes("0.16.0")).unwrap();
        swap::make_executable(&exe).unwrap();
        Env {
            exe,
            path_var: dir.into_os_string(),
            auto_mode: false,
            strict: false,
            windows: false,
            os: "linux".into(),
            arch: "x86_64".into(),
            current_version: "0.16.0".into(),
            api_base: API.into(),
            download_base: DL.into(),
            announce: announce_to_test,
        }
    }

    thread_local! {
        static ANNOUNCED: std::cell::RefCell<Vec<String>> = const { std::cell::RefCell::new(Vec::new()) };
    }

    fn announce_to_test(text: &str) {
        ANNOUNCED.with(|a| a.borrow_mut().push(text.to_string()));
    }

    fn announced() -> Vec<String> {
        ANNOUNCED.with(|a| a.borrow_mut().drain(..).collect())
    }

    /// A 0.17.0 binary whose `breaking --since <v> --json` lists one entry.
    fn breaking_script() -> Vec<u8> {
        concat!(
            "#!/bin/sh\n",
            "if [ \"$*\" = 'breaking --since 0.16.0 --json' ]; then\n",
            "  echo '[{\"version\":\"0.17.0\",\"component\":\"playbook\",",
            "\"summary\":\"The old flag is gone.\",\"action\":\"playbook setup\",",
            "\"migration\":null}]'\n",
            "elif [ \"$1\" = breaking ]; then exit 2\n",
            "else echo 'playbook 0.17.0'; fi\n"
        )
        .as_bytes()
        .to_vec()
    }

    fn fixture(binary: &[u8], sums_hash_of: &[u8]) -> FakeFetcher {
        let asset = "playbook-0.17.0-x86_64-unknown-linux-musl";
        let list = r#"[{"tag_name":"v0.18.0-rc.1","prerelease":true},{"tag_name":"v0.17.0","prerelease":false}]"#;
        let mut f = FakeFetcher::default();
        f.0.insert(format!("{API}{}", resolve::RELEASES_PAGE), list.into());
        f.0.insert(
            format!("{API}/releases/tags/v0.17.0"),
            r#"{"tag_name":"v0.17.0"}"#.into(),
        );
        f.0.insert(format!("{DL}/v0.17.0/{asset}"), binary.to_vec());
        let sums = format!("{}  {asset}\n", verify::sha256_hex(sums_hash_of));
        f.0.insert(format!("{DL}/v0.17.0/SHA256SUMS"), sums.into_bytes());
        f
    }

    fn ok_attest() -> FakeAttest {
        FakeAttest::new(Ok(None))
    }

    #[test]
    fn list_shows_tags_with_markers_and_changes_nothing() {
        let mut env = env_in("up-list");
        env.current_version = "0.17.0".into();
        let opts = Options {
            list: true,
            ..Options::default()
        };
        let out = run(
            &opts,
            &env,
            &fixture(b"", b""),
            &ok_attest(),
            &swap::run_version,
        )
        .unwrap();
        assert_eq!(
            out.out,
            vec!["v0.18.0-rc.1 (pre-release)", "v0.17.0 (installed)"]
        );
    }

    #[test]
    fn check_reports_the_latest_stable_without_writing() {
        let env = env_in("up-check");
        let opts = Options {
            check: true,
            ..Options::default()
        };
        let out = run(
            &opts,
            &env,
            &fixture(b"", b""),
            &ok_attest(),
            &swap::run_version,
        )
        .unwrap();
        assert!(out.out[0].contains("0.16.0 -> 0.17.0"), "{:?}", out.out);
        assert_eq!(out.installed, None);
        assert_eq!(swap::run_version(&env.exe).unwrap(), "playbook 0.16.0");
    }

    #[test]
    fn check_with_pre_sees_the_release_candidate() {
        let env = env_in("up-check-pre");
        let opts = Options {
            check: true,
            pre: true,
            ..Options::default()
        };
        let out = run(
            &opts,
            &env,
            &fixture(b"", b""),
            &ok_attest(),
            &swap::run_version,
        )
        .unwrap();
        assert!(out.out[0].contains("0.18.0-rc.1"), "{:?}", out.out);
    }

    #[test]
    fn an_update_replaces_the_binary_and_keeps_a_backup() {
        let env = env_in("up-ok");
        let bin = script_bytes("0.17.0");
        let out = run(
            &Options::default(),
            &env,
            &fixture(&bin, &bin),
            &ok_attest(),
            &swap::run_version,
        )
        .unwrap();
        assert_eq!(swap::run_version(&env.exe).unwrap(), "playbook 0.17.0");
        assert!(env.exe.with_file_name("playbook.0.16.0.bak").exists());
        assert!(out.out.iter().any(|l| l.contains("0.16.0 -> 0.17.0")));
        assert_eq!(out.installed.as_deref(), Some("0.17.0"));
    }

    #[test]
    fn an_update_shows_the_new_binary_s_breaking_changes_before_the_swap() {
        // Arrange
        let env = env_in("up-breaking");
        let bin = breaking_script();
        announced();

        // Act
        let out = run(
            &Options::default(),
            &env,
            &fixture(&bin, &bin),
            &ok_attest(),
            &swap::run_version,
        )
        .unwrap();

        // Assert
        assert_eq!(
            out.breaking,
            "Breaking changes in this upgrade:\n  playbook 0.17.0: The old flag is gone.\n    Action: playbook setup\n"
        );
        assert_eq!(announced(), vec![out.breaking.clone()]);
        assert_eq!(out.installed.as_deref(), Some("0.17.0"));
    }

    #[test]
    fn the_block_is_shown_before_a_swap_that_fails() {
        let env = env_in("up-breaking-fail");
        let bin = breaking_script();
        announced();

        let err = run(
            &Options::default(),
            &env,
            &fixture(&bin, &bin),
            &ok_attest(),
            &|_: &Path| Err("smoke test failed".to_string()),
        )
        .unwrap_err();

        assert!(err.contains("smoke test"), "{err}");
        assert_eq!(announced().len(), 1);
        assert_eq!(swap::run_version(&env.exe).unwrap(), "playbook 0.16.0");
    }

    #[test]
    fn a_new_binary_without_the_breaking_command_shows_nothing() {
        let env = env_in("up-no-breaking");
        let bin = script_bytes("0.17.0");
        announced();

        let out = run(
            &Options::default(),
            &env,
            &fixture(&bin, &bin),
            &ok_attest(),
            &swap::run_version,
        )
        .unwrap();

        assert_eq!(out.breaking, "");
        assert!(announced().is_empty());
    }

    #[test]
    fn a_downgrade_shows_no_breaking_changes() {
        let mut env = env_in("up-down-breaking");
        env.current_version = "0.19.0".into();
        let bin = breaking_script();
        let opts = Options {
            version: Some("0.17.0".into()),
            ..Options::default()
        };
        announced();

        let out = run(
            &opts,
            &env,
            &fixture(&bin, &bin),
            &ok_attest(),
            &swap::run_version,
        )
        .unwrap();

        assert_eq!(out.breaking, "");
        assert!(announced().is_empty());
    }

    #[test]
    fn the_heal_is_told_when_the_update_already_showed_the_block() {
        // Arrange
        let env = env_in("heal-shown");
        std::fs::write(
            &env.exe,
            "#!/bin/sh\necho \"shown=${PLAYBOOK_BREAKING_SHOWN-unset}\"\n",
        )
        .unwrap();
        swap::make_executable(&env.exe).unwrap();
        let mut shown = Outcome {
            installed: Some("0.27.0".into()),
            breaking: "Breaking changes in this upgrade:\n".into(),
            breaking_shown_since: Some("0.26.0".into()),
            ..Outcome::default()
        };
        let mut quiet = Outcome {
            installed: Some("0.27.0".into()),
            ..Outcome::default()
        };

        // Act
        finish_with_heal(&mut shown, &env.exe);
        finish_with_heal(&mut quiet, &env.exe);

        // Assert
        assert_eq!(shown.out, vec!["migrate: shown=0.26.0"]);
        assert_eq!(quiet.out, vec!["migrate: shown="]);
    }

    #[test]
    fn the_heal_after_a_swap_skips_releases_without_doctor_fix() {
        let env = env_in("heal-old");
        assert_eq!(heal_after_swap(&env.exe, "0.26.0", None), Ok(None));
    }

    #[test]
    fn the_heal_after_a_swap_runs_the_new_binary_and_reports_its_failure() {
        // Arrange
        let env = env_in("heal-run");
        std::fs::write(
            &env.exe,
            "#!/bin/sh\necho 'Breaking changes in this upgrade:' >&2\necho \"ran: $*\"\n",
        )
        .unwrap();
        swap::make_executable(&env.exe).unwrap();
        let mut done = Outcome {
            installed: Some("0.27.0-rc.1".into()),
            ..Outcome::default()
        };

        // Act
        finish_with_heal(&mut done, &env.exe);
        let mut untouched = Outcome::default();
        finish_with_heal(&mut untouched, &env.exe);
        std::fs::write(
            &env.exe,
            "#!/bin/sh\necho 'failed: state: locked'\nexit 1\n",
        )
        .unwrap();
        let failed = heal_after_swap(&env.exe, "1.0.0", None).unwrap_err();
        std::fs::write(
            &env.exe,
            "#!/bin/sh\n[ \"$1\" = migrate ] && exit 2\necho \"ran: $*\"\n",
        )
        .unwrap();
        let older = heal_after_swap(&env.exe, "0.27.0", None).unwrap();
        std::fs::write(
            &env.exe,
            "#!/bin/sh\n[ \"$1\" = migrate ] && exit 2\necho 'failed: x'\nexit 1\n",
        )
        .unwrap();
        let older_failed = heal_after_swap(&env.exe, "0.27.0", None).unwrap_err();

        // Assert
        assert_eq!(
            done.out,
            vec![
                "migrate: Breaking changes in this upgrade:",
                "migrate: ran: migrate"
            ]
        );
        assert_eq!(older, Some(("doctor --fix", "ran: doctor --fix\n".into())));
        assert!(
            older_failed.starts_with("doctor --fix exited with")
                && older_failed.ends_with("run `playbook doctor --fix` by hand"),
            "{older_failed}"
        );
        assert!(done.warnings.is_empty());
        assert!(untouched.out.is_empty() && untouched.warnings.is_empty());
        assert!(failed.contains(": failed: state: locked;"), "{failed}");
        assert!(
            failed.contains("run `playbook migrate` by hand"),
            "{failed}"
        );
    }

    #[test]
    fn an_explicit_version_uses_the_tag_endpoint() {
        let env = env_in("up-explicit");
        let bin = script_bytes("0.17.0");
        let opts = Options {
            version: Some("0.17.0".into()),
            ..Options::default()
        };
        run(
            &opts,
            &env,
            &fixture(&bin, &bin),
            &ok_attest(),
            &swap::run_version,
        )
        .unwrap();
        assert_eq!(swap::run_version(&env.exe).unwrap(), "playbook 0.17.0");
    }

    #[test]
    fn a_checksum_mismatch_aborts_before_anything_is_written() {
        let env = env_in("up-badsum");
        let err = run(
            &Options::default(),
            &env,
            &fixture(&script_bytes("0.17.0"), b"other"),
            &ok_attest(),
            &swap::run_version,
        )
        .unwrap_err();
        assert!(err.contains("checksum mismatch"), "{err}");
        assert_eq!(swap::run_version(&env.exe).unwrap(), "playbook 0.16.0");
        assert!(!env.exe.with_file_name("playbook.0.16.0.bak").exists());
    }

    #[test]
    fn a_failed_attestation_aborts_and_a_lenient_gap_only_warns() {
        let env = env_in("up-attest");
        let bin = script_bytes("0.17.0");
        let bad = FakeAttest::new(Err("attestation verification failed".into()));
        assert!(run(
            &Options::default(),
            &env,
            &fixture(&bin, &bin),
            &bad,
            &swap::run_version
        )
        .is_err());
        assert_eq!(swap::run_version(&env.exe).unwrap(), "playbook 0.16.0");
        let gap = FakeAttest::new(Ok(Some("checksum only".into())));
        let out = run(
            &Options::default(),
            &env,
            &fixture(&bin, &bin),
            &gap,
            &swap::run_version,
        )
        .unwrap();
        assert!(out.warnings.iter().any(|w| w.contains("checksum only")));
    }

    #[test]
    fn a_binary_that_does_not_run_is_rejected_and_the_original_stays() {
        let env = env_in("up-badbin");
        let bin = b"#!/bin/sh\nexit 1\n".to_vec();
        let err = run(
            &Options::default(),
            &env,
            &fixture(&bin, &bin),
            &ok_attest(),
            &swap::run_version,
        )
        .unwrap_err();
        assert!(err.contains("smoke test"), "{err}");
        assert_eq!(swap::run_version(&env.exe).unwrap(), "playbook 0.16.0");
    }

    #[test]
    fn a_homebrew_binary_is_refused_with_the_brew_command() {
        let mut env = env_in("up-brew");
        env.exe = scratch_dir("up-brew-cellar").join("Cellar/playbook/0.14.0/bin/playbook");
        let err = run(
            &Options::default(),
            &env,
            &fixture(b"", b""),
            &ok_attest(),
            &swap::run_version,
        )
        .unwrap_err();
        assert!(err.contains("brew upgrade playbook"), "{err}");
    }

    #[test]
    fn windows_is_refused_with_manual_steps() {
        let mut env = env_in("up-win");
        env.windows = true;
        let err = run(
            &Options::default(),
            &env,
            &fixture(b"", b""),
            &ok_attest(),
            &swap::run_version,
        )
        .unwrap_err();
        assert!(err.contains("releases/latest"), "{err}");
    }

    #[test]
    fn auto_mode_needs_yes() {
        let mut env = env_in("up-auto");
        env.auto_mode = true;
        let bin = script_bytes("0.17.0");
        let err = run(
            &Options::default(),
            &env,
            &fixture(&bin, &bin),
            &ok_attest(),
            &swap::run_version,
        )
        .unwrap_err();
        assert!(err.contains("--yes"), "{err}");
        assert_eq!(swap::run_version(&env.exe).unwrap(), "playbook 0.16.0");
        let opts = Options {
            yes: true,
            ..Options::default()
        };
        run(
            &opts,
            &env,
            &fixture(&bin, &bin),
            &ok_attest(),
            &swap::run_version,
        )
        .unwrap();
    }

    #[test]
    fn a_shadowing_binary_is_warned_about() {
        let mut env = env_in("up-shadow");
        let other = scratch_dir("up-shadow-first");
        std::fs::create_dir_all(&other).unwrap();
        std::fs::write(other.join("playbook"), script_bytes("0.14.0")).unwrap();
        swap::make_executable(&other.join("playbook")).unwrap();
        let own = env.exe.parent().unwrap().to_path_buf();
        env.path_var = std::env::join_paths([&other, &own]).unwrap();
        let bin = script_bytes("0.17.0");
        let out = run(
            &Options::default(),
            &env,
            &fixture(&bin, &bin),
            &ok_attest(),
            &swap::run_version,
        )
        .unwrap();
        assert!(
            out.warnings
                .iter()
                .any(|w| w.contains("runs before the updated")),
            "{:?}",
            out.warnings
        );
    }

    #[test]
    fn strict_mode_reaches_the_attestation_check() {
        let mut env = env_in("up-strict");
        env.strict = true;
        let bin = script_bytes("0.17.0");
        let attest = ok_attest();
        run(
            &Options::default(),
            &env,
            &fixture(&bin, &bin),
            &attest,
            &swap::run_version,
        )
        .unwrap();
        assert_eq!(*attest.1.borrow(), vec![true]);
    }

    #[test]
    fn the_same_version_is_a_no_op_even_in_auto_mode_without_yes() {
        let mut env = env_in("up-same");
        env.current_version = "0.17.0".into();
        env.auto_mode = true;
        let out = run(
            &Options::default(),
            &env,
            &fixture(b"", b""),
            &ok_attest(),
            &swap::run_version,
        )
        .unwrap();
        assert!(out.out[0].contains("already installed"), "{:?}", out.out);
        assert_eq!(swap::run_version(&env.exe).unwrap(), "playbook 0.16.0");
    }

    #[test]
    fn a_binary_newer_than_latest_is_not_downgraded_without_a_version() {
        let mut env = env_in("up-newer");
        env.current_version = "0.19.0".into();
        let out = run(
            &Options::default(),
            &env,
            &fixture(b"", b""),
            &ok_attest(),
            &swap::run_version,
        )
        .unwrap();
        assert!(out.out[0].contains("nothing to do"), "{:?}", out.out);
    }

    #[test]
    fn an_explicit_older_version_is_installed_as_a_downgrade() {
        let mut env = env_in("up-down");
        env.current_version = "0.19.0".into();
        let bin = script_bytes("0.17.0");
        let opts = Options {
            version: Some("0.17.0".into()),
            ..Options::default()
        };
        run(
            &opts,
            &env,
            &fixture(&bin, &bin),
            &ok_attest(),
            &swap::run_version,
        )
        .unwrap();
        assert_eq!(swap::run_version(&env.exe).unwrap(), "playbook 0.17.0");
    }

    #[test]
    fn check_works_in_auto_mode_without_yes() {
        let mut env = env_in("up-check-auto");
        env.auto_mode = true;
        let opts = Options {
            check: true,
            ..Options::default()
        };
        assert!(run(
            &opts,
            &env,
            &fixture(b"", b""),
            &ok_attest(),
            &swap::run_version
        )
        .is_ok());
    }
}
