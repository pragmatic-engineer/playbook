// SPDX-FileCopyrightText: 2026 Igor Santos
// SPDX-License-Identifier: Apache-2.0

//! One plain answer to "is telemetry on?", shown by `playbook usage status`
//! and in the usage view's footer. It only reads: the `usage.telemetry`
//! config key, the Claude Code settings files and environment, and the
//! collector state. Totals always come from transcripts, so an `off` here
//! changes what the cost-per-agent and latency views can show, never a total.

use super::collector::{read_state, running, state_path, State};
use super::run::Paths;
use super::telemetry_detect::{configured, Configured};
use serde_json::Value;
use std::path::Path;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Mode {
    /// Configured to reach the local collector, and it is running.
    OnRunning,
    /// Configured to reach the local collector, which is not running.
    OnCollectorDown,
    /// `usage.telemetry` is `off`.
    OffByConfig,
    /// Your Claude Code settings turn telemetry off.
    OffDisabled,
    /// Your Claude Code settings send telemetry somewhere else.
    OffOwnSetup,
    /// Nothing sets telemetry for plain `claude`. `cc` still does for its sessions.
    OffNotConfigured,
}

impl Mode {
    /// The status text, without the fix.
    pub fn text(self) -> &'static str {
        match self {
            Mode::OnRunning => "telemetry: on, collector running",
            Mode::OnCollectorDown => "telemetry: on, collector not running",
            Mode::OffByConfig => "telemetry: off (usage.telemetry is off, using transcripts)",
            Mode::OffDisabled => {
                "telemetry: off (disabled in your Claude Code settings, using transcripts)"
            }
            Mode::OffOwnSetup => {
                "telemetry: off (set up by you for another endpoint, using transcripts)"
            }
            Mode::OffNotConfigured => "telemetry: off (not configured)",
        }
    }

    /// The one-line fix, when there is one.
    pub fn fix(self) -> Option<&'static str> {
        match self {
            Mode::OnRunning | Mode::OffOwnSetup => None,
            Mode::OnCollectorDown => Some("start it with `playbook usage collect`; `cc` starts it for you"),
            Mode::OffByConfig => Some("run `playbook config set usage.telemetry auto`"),
            Mode::OffDisabled => Some(
                "to turn it on, remove CLAUDE_CODE_ENABLE_TELEMETRY from the env block of your Claude Code settings, then run `playbook init --telemetry`",
            ),
            Mode::OffNotConfigured => Some("run `playbook init --telemetry`, or start sessions with `cc`"),
        }
    }

    /// The short form for the footer.
    pub fn footer(self) -> &'static str {
        match self {
            Mode::OnRunning => "telemetry: on, collector running",
            Mode::OnCollectorDown => "telemetry: on, collector down",
            Mode::OffByConfig | Mode::OffDisabled | Mode::OffOwnSetup => {
                "telemetry: off, using transcripts"
            }
            Mode::OffNotConfigured => "telemetry: off (not configured)",
        }
    }
}

/// The decision with its inputs given, so it can be tested.
pub fn decide(config_auto: bool, kind: Configured, collector_running: bool) -> Mode {
    if !config_auto {
        return Mode::OffByConfig;
    }
    match kind {
        Configured::Disabled => Mode::OffDisabled,
        Configured::Other => Mode::OffOwnSetup,
        Configured::Nothing => Mode::OffNotConfigured,
        Configured::Playbook if collector_running => Mode::OnRunning,
        Configured::Playbook => Mode::OnCollectorDown,
    }
}

/// Reads the real inputs for `home` and `cwd`.
pub fn detect(home: &Path, cwd: &Path, paths: &Paths) -> Mode {
    let config_auto = matches!(
        super::host::config_value("usage.telemetry", home),
        Some(Value::String(v)) if v == "auto"
    );
    let state: Option<State> = read_state(&state_path(paths));
    let kind = configured(home, cwd, state.as_ref().map(|s| s.port));
    decide(config_auto, kind, state.as_ref().is_some_and(running))
}

/// `detect` for the real home and the current directory.
pub fn detect_here(paths: &Paths) -> Mode {
    let cwd = std::env::current_dir().unwrap_or_default();
    detect(&super::host::home_dir(), &cwd, paths)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_config_key_off_wins_over_everything() {
        for kind in [
            Configured::Nothing,
            Configured::Playbook,
            Configured::Disabled,
        ] {
            assert_eq!(decide(false, kind, true), Mode::OffByConfig);
        }
    }

    #[test]
    fn user_settings_that_disable_or_redirect_are_reported_and_never_overridden() {
        assert_eq!(decide(true, Configured::Disabled, true), Mode::OffDisabled);
        assert_eq!(decide(true, Configured::Other, true), Mode::OffOwnSetup);
    }

    #[test]
    fn nothing_configured_says_off_not_configured_with_the_fix() {
        let mode = decide(true, Configured::Nothing, false);
        assert_eq!(mode.text(), "telemetry: off (not configured)");
        assert!(mode.fix().unwrap().contains("playbook init --telemetry"));
    }

    #[test]
    fn the_collector_state_separates_the_two_on_modes() {
        assert_eq!(decide(true, Configured::Playbook, true), Mode::OnRunning);
        assert_eq!(
            decide(true, Configured::Playbook, false),
            Mode::OnCollectorDown
        );
        assert_eq!(Mode::OnRunning.text(), "telemetry: on, collector running");
    }

    #[test]
    fn the_disabled_text_names_your_settings_and_transcripts() {
        assert_eq!(
            Mode::OffDisabled.text(),
            "telemetry: off (disabled in your Claude Code settings, using transcripts)"
        );
        assert!(Mode::OffDisabled.fix().is_some());
    }
}
