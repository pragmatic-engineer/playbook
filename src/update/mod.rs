// SPDX-FileCopyrightText: 2026 Igor Santos
// SPDX-License-Identifier: MIT

//! `playbook update`: fetch a release binary, verify it (SHA256SUMS, then the
//! build attestation), swap it in with a backup, then check PATH and the
//! Claude Code plugin. Install.sh and this share one policy; see `verify`.

pub mod download;
pub mod pathcheck;
pub mod resolve;
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
    pub home: PathBuf,
    pub path_var: OsString,
    pub auto_mode: bool,
    pub strict: bool,
    pub windows: bool,
    pub os: String,
    pub arch: String,
    pub current_version: String,
    pub api_base: String,
    pub download_base: String,
}

impl Env {
    /// The real machine. The running binary is canonicalised so a symlink is
    /// replaced at its target, not turned into a regular file.
    pub fn real() -> Result<Env, String> {
        let exe = std::env::current_exe()
            .map_err(|e| format!("cannot locate the running binary: {e}"))?;
        Ok(Env {
            exe: std::fs::canonicalize(&exe).unwrap_or(exe),
            home: crate::common::home_dir(),
            path_var: std::env::var_os("PATH").unwrap_or_default(),
            auto_mode: crate::common::mode::resolve_for_hook().mode
                == crate::common::mode::Mode::Auto,
            strict: std::env::var("PLAYBOOK_REQUIRE_ATTESTATION").is_ok_and(|v| v == "1"),
            windows: cfg!(windows),
            os: std::env::consts::OS.to_string(),
            arch: std::env::consts::ARCH.to_string(),
            current_version: env!("CARGO_PKG_VERSION").to_string(),
            api_base: format!("https://api.github.com/repos/{REPO}"),
            download_base: format!("https://github.com/{REPO}/releases/download"),
        })
    }
}

/// Lines to print: `out` to stdout, `warnings` to stderr.
#[derive(Debug, Default)]
pub struct Outcome {
    pub out: Vec<String>,
    pub warnings: Vec<String>,
}

fn is_homebrew(exe: &Path) -> bool {
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
        None => format!("{}/releases?per_page=30", env.api_base),
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

    refusals(env)?;
    if env.auto_mode && !opts.yes {
        return Err("auto mode is on and an update replaces the running binary; re-run with --yes to confirm".into());
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
    let stage = dir.join(format!(".playbook.update.{}", std::process::id()));
    std::fs::write(&stage, &bytes)
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

    let done = swap::install(&stage, &env.exe, current, &version, smoke)?;
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
    o.warnings
        .extend(pathcheck::plugin_hint(&env.home, &version));
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
            home: dir.clone(),
            path_var: dir.into_os_string(),
            auto_mode: false,
            strict: false,
            windows: false,
            os: "linux".into(),
            arch: "x86_64".into(),
            current_version: "0.16.0".into(),
            api_base: API.into(),
            download_base: DL.into(),
        }
    }

    fn fixture(binary: &[u8], sums_hash_of: &[u8]) -> FakeFetcher {
        let asset = "playbook-0.17.0-x86_64-unknown-linux-musl";
        let list = r#"[{"tag_name":"v0.18.0-rc.1","prerelease":true},{"tag_name":"v0.17.0","prerelease":false}]"#;
        let mut f = FakeFetcher::default();
        f.0.insert(format!("{API}/releases?per_page=30"), list.into());
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
        FakeAttest(Ok(None))
    }

    #[test]
    fn list_shows_tags_with_markers_and_changes_nothing() {
        let env = env_in("up-list");
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
        assert_eq!(out.out, vec!["v0.18.0-rc.1 (pre-release)", "v0.17.0"]);
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
        let bad = FakeAttest(Err("attestation verification failed".into()));
        assert!(run(
            &Options::default(),
            &env,
            &fixture(&bin, &bin),
            &bad,
            &swap::run_version
        )
        .is_err());
        assert_eq!(swap::run_version(&env.exe).unwrap(), "playbook 0.16.0");
        let gap = FakeAttest(Ok(Some("checksum only".into())));
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
        env.exe = PathBuf::from("/opt/homebrew/Cellar/playbook/0.14.0/bin/playbook");
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
    fn a_shadowing_binary_and_a_stale_plugin_are_warned_about() {
        let mut env = env_in("up-shadow");
        let other = scratch_dir("up-shadow-first");
        std::fs::create_dir_all(&other).unwrap();
        std::fs::write(other.join("playbook"), script_bytes("0.14.0")).unwrap();
        swap::make_executable(&other.join("playbook")).unwrap();
        let own = env.exe.parent().unwrap().to_path_buf();
        env.path_var = std::env::join_paths([&other, &own]).unwrap();
        std::fs::create_dir_all(env.home.join(".claude/plugins")).unwrap();
        std::fs::write(
            env.home.join(".claude/plugins/installed_plugins.json"),
            r#"{"plugins":{"playbook@pragmatic-engineer":[{"version":"0.15.0"}]}}"#,
        )
        .unwrap();
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
        assert!(out
            .warnings
            .iter()
            .any(|w| w.contains("claude plugin update playbook@pragmatic-engineer")));
    }
}
