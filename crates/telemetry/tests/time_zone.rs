// SPDX-FileCopyrightText: 2026 Igor Santos
// SPDX-License-Identifier: Apache-2.0

//! The time zone end to end. This file is its own process, so setting the
//! process wide zone here cannot disturb any other test.

use playbook_telemetry::aggregate::{group_usage, Dimension};
use playbook_telemetry::query::Range;
use playbook_telemetry::tz::{self, Zone};
use playbook_telemetry::UsageEvent;
use std::path::PathBuf;
use std::process::{Command, Output};

const DAY: i64 = 86_400;

fn event_at(timestamp: i64) -> UsageEvent {
    UsageEvent {
        event_id: timestamp.to_string(),
        timestamp,
        model: "opus".to_string(),
        ..UsageEvent::default()
    }
}

#[test]
fn an_event_after_utc_midnight_lands_on_the_previous_day_three_hours_west() {
    // Arrange: 01:00 UTC on 2026-09-03 is 22:00 on 2026-09-02 at UTC-3.
    let utc_midnight = 1_788_393_600; // 2026-09-03 00:00:00 UTC
    tz::set(Zone::Fixed(-3 * 3600));

    // Act
    let days = group_usage(&[event_at(utc_midnight + 3600)], Dimension::Day);

    // Assert
    assert_eq!(days.len(), 1);
    assert_eq!(days[0].key, "2026-09-02");
}

#[test]
fn today_starts_at_local_midnight_in_the_chosen_zone() {
    // Arrange
    let utc_midnight = 1_788_393_600;
    tz::set(Zone::Fixed(-3 * 3600));
    let now = utc_midnight + 3600; // 22:00 local, 2026-09-02

    // Act
    let (first, past) = Range::Today.days(now).expect("today has a window");

    // Assert
    assert_eq!(tz::day_start(first), utc_midnight - DAY + 3 * 3600);
    assert_eq!(tz::day_start(past), utc_midnight + 3 * 3600);
}

fn binary(args: &[&str], zone_env: Option<&str>) -> Output {
    let home: PathBuf =
        std::env::temp_dir().join(format!("pt-tz-{}-{}", std::process::id(), args.join("")));
    std::fs::create_dir_all(&home).expect("create a home");
    let mut cmd = Command::new(env!("CARGO_BIN_EXE_playbook-telemetry"));
    cmd.args(args)
        .env("HOME", &home)
        .env_remove("CLAUDE_CONFIG_DIR")
        .env_remove("XDG_STATE_HOME")
        .env_remove("XDG_CONFIG_HOME")
        .env_remove("PLAYBOOK_TELEMETRY_TZ");
    if let Some(zone) = zone_env {
        cmd.env("PLAYBOOK_TELEMETRY_TZ", zone);
    }
    cmd.output().expect("run the binary")
}

fn json_mode(out: &Output) -> String {
    let text = String::from_utf8_lossy(&out.stdout);
    let doc: serde_json::Value = serde_json::from_str(&text).expect("json output");
    doc["time_zone"]["mode"].as_str().unwrap_or("").to_string()
}

#[test]
fn the_json_output_names_local_time_by_default() {
    let out = binary(&["--json", "--range", "today"], None);
    assert_eq!(json_mode(&out), "local");
}

#[test]
fn the_utc_flag_beats_a_local_environment_value() {
    let out = binary(&["--json", "--utc", "--range", "today"], Some("local"));
    assert_eq!(json_mode(&out), "utc");
}

#[test]
fn the_environment_value_switches_to_utc() {
    let out = binary(&["--json", "--range", "today"], Some("utc"));
    assert_eq!(json_mode(&out), "utc");
}

#[test]
fn passing_both_zone_flags_is_an_error() {
    let out = binary(&["--utc", "--local"], None);
    assert!(!out.status.success());
    assert!(String::from_utf8_lossy(&out.stderr).contains("cannot be used with"));
}

#[test]
fn the_zone_flags_work_after_a_subcommand() {
    let out = binary(&["status", "--utc"], None);
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
}
