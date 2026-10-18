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

#[test]
fn auto_cleanup_does_nothing_when_space_is_fine_or_it_is_off() {
    let home = isolated::fresh_home();
    let out = run(&home, "50", &["disk", "clean", "--auto"]);
    assert!(out.status.success());
    assert!(stdout(&out).contains("nothing to do"), "{}", stdout(&out));
    let set = isolated::playbook_in(&home)
        .args(["config", "set", "disk.autoCleanup", "false"])
        .output()
        .unwrap();
    assert!(set.status.success());
    let out = run(&home, "1", &["disk", "clean", "--auto"]);
    assert!(stdout(&out).contains("nothing to do"), "{}", stdout(&out));
}

#[test]
fn clean_removes_idle_scratch_in_a_test_tmp_folder_only() {
    let home = isolated::fresh_home();
    let tmp = home.join("tmp");
    let old = tmp.join("pb-old");
    std::fs::create_dir_all(&old).unwrap();
    std::fs::write(old.join("file"), vec![1u8; 4096]).unwrap();
    let hour_ago = std::time::SystemTime::now() - std::time::Duration::from_secs(2 * 60 * 60);
    for path in [old.join("file"), old.clone()] {
        std::fs::File::open(&path)
            .unwrap()
            .set_modified(hour_ago)
            .unwrap();
    }
    let fresh = tmp.join("pb-fresh");
    std::fs::create_dir_all(&fresh).unwrap();
    let out = isolated::playbook_in(&home)
        .args(["disk", "clean"])
        .env("PLAYBOOK_TEST_DISK_TMP", &tmp)
        .current_dir(&home)
        .output()
        .unwrap();
    let text = stdout(&out);
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    assert!(
        text.contains("removed") && text.contains("pb-old"),
        "{text}"
    );
    assert!(!old.exists());
    assert!(fresh.exists());
}

#[test]
fn reserve_refuses_when_space_is_low_and_release_reports_nothing_to_do() {
    let home = isolated::fresh_home();
    let out = run(&home, "10", &["disk", "reserve"]);
    assert_eq!(out.status.code(), Some(1));
    assert!(String::from_utf8_lossy(&out.stderr).contains("refused"));
    assert!(!home.join(".cache/playbook/ballast/ballast.bin").exists());
    let out = run(&home, "10", &["disk", "release"]);
    assert!(out.status.success());
    assert!(stdout(&out).contains("no ballast"), "{}", stdout(&out));
}

#[test]
fn release_deletes_an_existing_ballast() {
    let home = isolated::fresh_home();
    let ballast = home.join(".cache/playbook/ballast/ballast.bin");
    std::fs::create_dir_all(ballast.parent().unwrap()).unwrap();
    std::fs::write(&ballast, vec![1u8; 64 * 1024]).unwrap();
    let status = run(&home, "50", &["disk", "status"]);
    assert!(stdout(&status).contains("ballast"), "{}", stdout(&status));
    let out = run(&home, "50", &["disk", "release"]);
    assert!(out.status.success());
    assert!(stdout(&out).contains("given back"), "{}", stdout(&out));
    assert!(!ballast.exists());
}
