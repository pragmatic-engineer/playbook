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
    /// Telemetry rows arrived in the last day, whatever the settings say.
    OnReceiving,
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
    /// Telemetry is switched on but no `OTEL_EXPORTER_OTLP_ENDPOINT` is set,
    /// so plain `claude` exports nothing.
    OffNoEndpoint,
    /// Nothing sets telemetry for plain `claude`. `cc` still does for its sessions.
    OffNotConfigured,
}

impl Mode {
    /// The status text, without the fix.
    pub fn text(self) -> &'static str {
        match self {
            Mode::OnReceiving => "telemetry: on, data arriving",
            Mode::OnRunning => "telemetry: on, collector running",
            Mode::OnCollectorDown => "telemetry: on, collector not running",
            Mode::OffNoEndpoint => {
                "telemetry: off (OTEL_EXPORTER_OTLP_ENDPOINT is missing, using transcripts)"
            }
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
            Mode::OnReceiving | Mode::OnRunning | Mode::OffOwnSetup => None,
            Mode::OffNoEndpoint => Some(
                "run `playbook init --telemetry` to add the OTEL_* variables to ~/.claude/settings.json, and keep `playbook usage collect` running",
            ),
            Mode::OnCollectorDown => Some("start it with `playbook usage collect`; `cc` starts it for you"),
            Mode::OffByConfig => Some("run `playbook config set usage.telemetry auto`"),
            Mode::OffDisabled => Some(
                "to turn it on, run `playbook init --telemetry` (it replaces an off switch that stands alone in ~/.claude/settings.json), or remove CLAUDE_CODE_ENABLE_TELEMETRY from the env block of your Claude Code settings yourself",
            ),
            Mode::OffNotConfigured => Some("run `playbook init --telemetry`, or start sessions with `cc`"),
        }
    }

    /// The short form for the footer.
    pub fn footer(self) -> &'static str {
        match self {
            Mode::OnReceiving => "telemetry: on, data arriving",
            Mode::OnRunning => "telemetry: on, collector running",
            Mode::OnCollectorDown => "telemetry: on, collector down",
            Mode::OffNoEndpoint => "telemetry: off (no OTLP endpoint)",
            Mode::OffByConfig | Mode::OffDisabled | Mode::OffOwnSetup => {
                "telemetry: off, using transcripts"
            }
            Mode::OffNotConfigured => "telemetry: off (not configured)",
        }
    }
}

/// Facts beside the settings that change the answer.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Signals {
    /// A telemetry request was stored in the last day.
    pub rows_recent: bool,
    /// No `OTEL_EXPORTER_OTLP_ENDPOINT` in the process environment or settings.
    pub endpoint_missing: bool,
}

/// Whether the usage database holds a telemetry request from the last day.
fn rows_recent(paths: &Paths) -> bool {
    let Ok(conn) = rusqlite::Connection::open_with_flags(
        &paths.db,
        rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY,
    ) else {
        return false;
    };
    let since = super::util::time::now_secs() - 86_400;
    conn.query_row(
        "SELECT EXISTS(SELECT 1 FROM telemetry_requests WHERE timestamp >= ?1)",
        [since],
        |r| r.get::<_, bool>(0),
    )
    .unwrap_or(false)
}

/// The decision with its inputs given, so it can be tested.
pub fn decide(
    config_auto: bool,
    kind: Configured,
    collector_running: bool,
    signals: Signals,
) -> Mode {
    if !config_auto {
        return Mode::OffByConfig;
    }
    if signals.rows_recent {
        return Mode::OnReceiving;
    }
    match kind {
        Configured::Disabled => Mode::OffDisabled,
        Configured::Other if signals.endpoint_missing => Mode::OffNoEndpoint,
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
    let endpoint_missing = super::telemetry_detect::merged_vars(home, cwd, true)
        .is_some_and(|v| !v.contains_key("OTEL_EXPORTER_OTLP_ENDPOINT"));
    let signals = Signals {
        rows_recent: rows_recent(paths),
        endpoint_missing,
    };
    decide(
        config_auto,
        kind,
        state.as_ref().is_some_and(running),
        signals,
    )
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
            assert_eq!(
                decide(false, kind, true, Signals::default()),
                Mode::OffByConfig
            );
        }
    }

    #[test]
    fn user_settings_that_disable_or_redirect_are_reported_and_never_overridden() {
        assert_eq!(
            decide(true, Configured::Disabled, true, Signals::default()),
            Mode::OffDisabled
        );
        assert_eq!(
            decide(true, Configured::Other, true, Signals::default()),
            Mode::OffOwnSetup
        );
    }

    #[test]
    fn nothing_configured_says_off_not_configured_with_the_fix() {
        let mode = decide(true, Configured::Nothing, false, Signals::default());
        assert_eq!(mode.text(), "telemetry: off (not configured)");
        assert!(mode.fix().unwrap().contains("playbook init --telemetry"));
    }

    #[test]
    fn the_collector_state_separates_the_two_on_modes() {
        assert_eq!(
            decide(true, Configured::Playbook, true, Signals::default()),
            Mode::OnRunning
        );
        assert_eq!(
            decide(true, Configured::Playbook, false, Signals::default()),
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

    #[test]
    fn recent_telemetry_rows_mean_on_even_when_the_settings_say_off() {
        let signals = Signals {
            rows_recent: true,
            endpoint_missing: true,
        };

        let mode = decide(true, Configured::Disabled, false, signals);

        assert_eq!(mode, Mode::OnReceiving);
        assert!(mode.fix().is_none());
    }

    #[test]
    fn rows_do_not_override_the_off_config_key() {
        let signals = Signals {
            rows_recent: true,
            endpoint_missing: false,
        };

        assert_eq!(
            decide(false, Configured::Playbook, true, signals),
            Mode::OffByConfig
        );
    }

    #[test]
    fn telemetry_on_without_an_endpoint_names_the_missing_variable_and_the_fix() {
        let signals = Signals {
            rows_recent: false,
            endpoint_missing: true,
        };

        let mode = decide(true, Configured::Other, false, signals);

        assert_eq!(mode, Mode::OffNoEndpoint);
        assert!(mode.text().contains("OTEL_EXPORTER_OTLP_ENDPOINT"));
        assert!(mode.fix().unwrap().contains("playbook init --telemetry"));
    }

    #[test]
    fn a_missing_database_reports_no_recent_rows() {
        let paths = Paths::from_home(Path::new("/nonexistent-home-for-test"));

        assert!(!rows_recent(&paths));
    }
}
