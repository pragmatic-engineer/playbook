// SPDX-FileCopyrightText: 2026 Igor Santos
// SPDX-License-Identifier: Apache-2.0

//! `playbook disk` and the doctor's disk row, run as
//! the built binary in a scratch home. Free space comes from
//! `PLAYBOOK_TEST_DISK_FREE_GB`, which debug builds read in place of the
//! real value.

#[path = "support/isolated.rs"]
mod isolated;

use std::process::Output;

const FREE_VAR: &str = "PLAYBOOK_TEST_DISK_FREE_GB";

fn run(home: &std::path::Path, free_gb: &str, args: &[&str]) -> Output {
    isolated::playbook_in(home)
        .args(args)
        .env(FREE_VAR, free_gb)
        .current_dir(home)
        .output()
        .expect("run playbook")
}

fn stdout(out: &Output) -> String {
    String::from_utf8_lossy(&out.stdout).into_owned()
}

#[test]
fn status_json_reports_the_level_for_each_reading() {
    let cases = [
        ("0", "critical"),
        ("1", "critical"),
        ("5.9", "critical"),
        ("6.0", "low"),
        ("14.9", "low"),
        ("15", "ok"),
    ];
    for (free, want) in cases {
        let home = isolated::fresh_home();
        let out = run(&home, free, &["disk", "status", "--json"]);
        assert!(
            out.status.success(),
            "{free}: {}",
            String::from_utf8_lossy(&out.stderr)
        );
        let value: serde_json::Value = serde_json::from_str(&stdout(&out)).expect("json");
        assert_eq!(value["level"], want, "{free} GB free");
        assert_eq!(value["thresholds"]["warnFreeGb"], 15);
        assert_eq!(value["thresholds"]["blockFreeGb"], 6);
    }
}

#[test]
fn status_text_follows_configured_thresholds() {
    let home = isolated::fresh_home();
    let set = isolated::playbook_in(&home)
        .args(["config", "set", "disk.blockFreeGb", "20"])
        .output()
        .unwrap();
    assert!(
        set.status.success(),
        "{}",
        String::from_utf8_lossy(&set.stderr)
    );
    let out = run(&home, "19", &["disk", "status"]);
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    let text = stdout(&out);
    assert!(text.contains("block below 20 GB free"), "{text}");
    assert!(text.contains("critical"), "{text}");
}

#[test]
fn doctor_shows_a_disk_row_by_level() {
    for (free, want) in [("3", "FAIL"), ("10", "WARN"), ("50", "PASS")] {
        let home = isolated::fresh_home();
        let out = run(&home, free, &["doctor", "check", "--json"]);
        let text = stdout(&out);
        let value: serde_json::Value = serde_json::from_str(&text).expect("doctor json");
        let rows = value.to_string();
        assert!(rows.contains("disk: "), "{rows}");
        let row = value
            .as_array()
            .or_else(|| value["rows"].as_array())
            .expect("rows")
            .iter()
            .find(|r| r.to_string().contains("disk: "))
            .expect("disk row")
            .to_string();
        assert!(row.contains(want), "{free} GB: {row}");
    }
}
