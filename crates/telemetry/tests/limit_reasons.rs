// SPDX-FileCopyrightText: 2026 Igor Santos
// SPDX-License-Identifier: Apache-2.0

//! `pbtop --json` exits 0 and names why no limit comes from Claude, for each
//! reason word, from a home seeded with the files that cause it.

use std::path::PathBuf;
use std::process::Command;
use std::time::{SystemTime, UNIX_EPOCH};

const HOUR: i64 = 3600;
const MAX_5X: &str = r#""oauthAccount":{"organizationRateLimitTier":"default_claude_max_5x"}"#;

fn now() -> i64 {
    let secs = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("clock")
        .as_secs();
    i64::try_from(secs).expect("seconds fit")
}

fn home(tag: &str, claude_json: Option<String>, rate_limits: Option<String>) -> PathBuf {
    let home = std::env::temp_dir().join(format!("pb-reasons-{}-{tag}", std::process::id()));
    let _ = std::fs::remove_dir_all(&home);
    let data = home.join(".config/playbook/data");
    std::fs::create_dir_all(&data).expect("create the data folder");
    if let Some(body) = claude_json {
        std::fs::write(home.join(".claude.json"), body).expect("write .claude.json");
    }
    if let Some(body) = rate_limits {
        std::fs::write(data.join("rate_limits.json"), body).expect("write rate_limits.json");
    }
    home
}

fn reason_of(home: &PathBuf) -> serde_json::Value {
    let out = Command::new(env!("CARGO_BIN_EXE_pbtop"))
        .arg("--json")
        .env("HOME", home)
        .env_remove("PLAYBOOK_TELEMETRY_TZ")
        .env_remove("CLAUDE_CONFIG_DIR")
        .env_remove("XDG_STATE_HOME")
        .env_remove("XDG_CONFIG_HOME")
        .output()
        .expect("run the binary");
    assert!(out.status.success(), "{out:?}");
    let v: serde_json::Value = serde_json::from_slice(&out.stdout).expect("json output");
    v["limits"]["reason"].clone()
}

#[test]
fn each_reason_word_comes_out_of_the_binary_with_exit_zero() {
    let t = now();
    let stale_cache = format!(
        r#"{{{MAX_5X},"cachedUsageUtilization":{{"fetchedAtMs":{},"utilization":{{}}}}}}"#,
        (t - 7 * HOUR) * 1000
    );
    let reset_reading = format!(
        r#"{{"captured_at":{},"five_hour":{{"used_percentage":10,"resets_at":{}}}}}"#,
        t - HOUR,
        t - 10
    );
    let live_reading = format!(
        r#"{{"captured_at":{t},"five_hour":{{"used_percentage":10,"resets_at":{}}}}}"#,
        t + HOUR
    );
    let cases = [
        ("none", None, None, "cap-unknown"),
        ("max", Some(format!("{{{MAX_5X}}}")), None, "no-reading"),
        ("stale", Some(stale_cache), None, "stale"),
        ("reset", None, Some(reset_reading), "windows-reset"),
        ("live", None, Some(live_reading), ""),
    ];

    for (tag, claude_json, reading, want) in cases {
        let got = reason_of(&home(tag, claude_json, reading));

        if want.is_empty() {
            assert!(got.is_null(), "{tag}: {got}");
        } else {
            assert_eq!(got, want, "{tag}");
        }
    }
}
