// SPDX-FileCopyrightText: 2026 Igor Santos
// SPDX-License-Identifier: MIT

//! Mode resolver precedence (flag > env > config > default) as a pure table,
//! the hook-side resolution, and a `mode` value written straight into the
//! config file. Every spawned binary has `PLAYBOOK_MODE` removed and `$HOME`
//! pinned to a scratch directory, so a developer's exported mode or real
//! config can never flip a result.

use playbook::common::mode::{resolve, resolve_for_hook_at, Mode, Resolved, Source};
use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};
use std::sync::atomic::{AtomicU64, Ordering};

static SCRATCH_COUNTER: AtomicU64 = AtomicU64::new(0);

fn scratch_dir(tag: &str) -> PathBuf {
    let n = SCRATCH_COUNTER.fetch_add(1, Ordering::Relaxed);
    let dir = std::env::temp_dir().join(format!(
        "playbook-mode-resolve-{}-{tag}-{n}",
        std::process::id()
    ));
    fs::create_dir_all(&dir).expect("scratch dir should be creatable");
    dir
}

/// A scratch `$HOME` whose global config file holds `contents` verbatim, so a
/// value the CLI would refuse to write can still be planted.
fn home_with_global_config(tag: &str, contents: &str) -> PathBuf {
    let home = scratch_dir(tag);
    let dir = home.join(".config").join("playbook");
    fs::create_dir_all(&dir).expect("config dir should be creatable");
    fs::write(dir.join("config.json"), contents).expect("config file should be writable");
    home
}

/// Run the real binary with the mode env removed and `$HOME` pinned.
fn run_playbook(cwd: &Path, home: &Path, args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_playbook"))
        .args(args)
        .current_dir(cwd)
        .env("HOME", home)
        .env_remove("PLAYBOOK_MODE")
        .output()
        .expect("playbook binary should spawn")
}

fn stdout_of(out: &Output) -> String {
    String::from_utf8_lossy(&out.stdout).into_owned()
}

fn stderr_of(out: &Output) -> String {
    String::from_utf8_lossy(&out.stderr).into_owned()
}

/// Each input paired with the mode it yields when valid, `None` when the
/// resolver must ignore it.
const FLAGS: [(Option<Mode>, Option<Mode>); 3] = [
    (None, None),
    (Some(Mode::Auto), Some(Mode::Auto)),
    (Some(Mode::Ask), Some(Mode::Ask)),
];
const ENVS: [(Option<&str>, Option<Mode>); 7] = [
    (None, None),
    (Some("auto"), Some(Mode::Auto)),
    (Some("ask"), Some(Mode::Ask)),
    (Some("AUTO"), Some(Mode::Auto)),
    (Some(""), None),
    (Some("   "), None),
    (Some("banana"), None),
];
const CONFIGS: [(Option<&str>, Option<Mode>); 4] = [
    (None, None),
    (Some("auto"), Some(Mode::Auto)),
    (Some("ask"), Some(Mode::Ask)),
    (Some("banana"), None),
];

#[test]
fn highest_valid_level_wins_across_every_flag_env_config_combination() {
    for (flag, flag_mode) in FLAGS {
        for (env, env_mode) in ENVS {
            for (config, config_mode) in CONFIGS {
                // Arrange
                let want = if let Some(mode) = flag_mode {
                    (mode, Source::Flag)
                } else if let Some(mode) = env_mode {
                    (mode, Source::Env)
                } else if let Some(mode) = config_mode {
                    (mode, Source::Config)
                } else {
                    (Mode::Ask, Source::Default)
                };

                // Act
                let got = resolve(flag, env, config);

                // Assert
                assert_eq!(
                    (got.mode, got.source),
                    want,
                    "flag={flag:?} env={env:?} config={config:?}"
                );
            }
        }
    }
}

#[test]
fn an_invalid_env_value_is_ignored_with_a_warning() {
    // Arrange
    let env = Some("banana");

    // Act
    let got = resolve(None, env, None);

    // Assert
    assert_eq!((got.mode, got.source), (Mode::Ask, Source::Default));
    assert!(
        got.warnings.iter().any(|w| w.contains("banana")),
        "warnings: {:?}",
        got.warnings
    );
}

#[test]
fn hook_resolution_with_env_unset_and_config_auto_is_auto_from_config() {
    // Arrange
    let home = home_with_global_config("hook-config-auto", r#"{"mode":"auto"}"#);

    // Act
    let got = resolve_for_hook_at(None, &home, None);

    // Assert
    assert_eq!((got.mode, got.source), (Mode::Auto, Source::Config));
}

#[test]
fn a_mode_banana_written_into_the_file_is_ignored_by_the_resolver_with_a_warning() {
    // Arrange
    let home = home_with_global_config("resolver-banana", r#"{"mode":"banana"}"#);

    // Act
    let got: Resolved = resolve_for_hook_at(None, &home, None);

    // Assert
    assert_eq!((got.mode, got.source), (Mode::Ask, Source::Default));
    assert!(
        got.warnings.iter().any(|w| w.contains("banana")),
        "warnings: {:?}",
        got.warnings
    );
}

#[test]
fn a_mode_banana_written_into_the_file_is_ignored_by_config_get_with_a_warning() {
    // Arrange
    let home = home_with_global_config("cli-banana", r#"{"mode":"banana"}"#);
    let cwd = scratch_dir("cli-banana-cwd");

    // Act
    let out = run_playbook(&cwd, &home, &["config", "get", "mode"]);

    // Assert
    assert!(out.status.success(), "stderr: {}", stderr_of(&out));
    let stdout = stdout_of(&out);
    assert!(
        stdout.contains("mode: ask (source: default, global value ignored)"),
        "{stdout}"
    );
    let stderr = stderr_of(&out);
    assert!(
        stderr.contains("warning") && stderr.contains("banana"),
        "{stderr}"
    );
}

#[test]
fn config_set_rejects_mode_banana_naming_the_valid_values() {
    // Arrange
    let home = scratch_dir("set-banana-home");
    let cwd = scratch_dir("set-banana-cwd");

    // Act
    let out = run_playbook(
        &cwd,
        &home,
        &["config", "set", "mode", "banana", "--global"],
    );

    // Assert
    assert!(!out.status.success());
    let stderr = stderr_of(&out);
    assert!(stderr.contains("valid values are: ask, auto"), "{stderr}");
    assert!(
        !home
            .join(".config")
            .join("playbook")
            .join("config.json")
            .exists(),
        "nothing should have been written"
    );
}
