// SPDX-FileCopyrightText: 2026 Igor Santos
// SPDX-License-Identifier: Apache-2.0

//! `settings.shared.json` carries exactly the pinned set of top-level keys, so
//! a key cannot be added or dropped without updating this list.

use serde_json::Value;
use std::collections::BTreeSet;

const EXPECTED: [&str; 23] = [
    "$schema",
    "agentPushNotifEnabled",
    "autoUpdatesChannel",
    "awaySummaryEnabled",
    "cleanupPeriodDays",
    "editorMode",
    "env",
    "feedbackSurveyRate",
    "hooks",
    "includeCoAuthoredBy",
    "includeGitInstructions",
    "inputNeededNotifEnabled",
    "outputStyle",
    "permissions",
    "remoteControlAtStartup",
    "skipAutoPermissionPrompt",
    "skipDangerousModePermissionPrompt",
    "spinnerTipsEnabled",
    "statusLine",
    "teammateMode",
    "tui",
    "useAutoModeDuringPlan",
    "worktree",
];

fn keys_of(value: &Value) -> BTreeSet<String> {
    value
        .as_object()
        .expect("settings is an object")
        .keys()
        .cloned()
        .collect()
}

fn expected() -> BTreeSet<String> {
    EXPECTED.iter().map(|s| s.to_string()).collect()
}

fn seed() -> Value {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("settings.shared.json");
    serde_json::from_str(&std::fs::read_to_string(path).unwrap()).unwrap()
}

#[test]
fn the_committed_seed_matches_the_pinned_key_set() {
    let actual = keys_of(&seed());
    let exp = expected();
    let extra: Vec<_> = actual.difference(&exp).collect();
    let missing: Vec<_> = exp.difference(&actual).cloned().collect();
    assert!(
        extra.is_empty() && missing.is_empty(),
        "extra={extra:?} missing={missing:?}"
    );
}

#[test]
fn an_extra_key_is_reported_by_name() {
    let mut v = seed();
    v.as_object_mut()
        .unwrap()
        .insert("zzExtraKey".into(), Value::Null);
    let actual = keys_of(&v);
    let exp = expected();
    let extra: Vec<_> = actual.difference(&exp).collect();
    assert_eq!(extra, vec![&"zzExtraKey".to_string()]);
}

#[test]
fn a_removed_key_is_reported_by_name() {
    let mut v = seed();
    v.as_object_mut().unwrap().remove("editorMode");
    let actual = keys_of(&v);
    let exp = expected();
    let missing: Vec<_> = exp.difference(&actual).cloned().collect();
    assert_eq!(missing, vec!["editorMode".to_string()]);
}
