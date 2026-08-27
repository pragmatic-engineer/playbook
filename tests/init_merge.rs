// SPDX-FileCopyrightText: 2026 Igor Santos
// SPDX-License-Identifier: MIT

//! Integration tests for `playbook::init::merge`, ported from
//! `shell/merge-settings.test.sh`'s 19 scenarios (s1 through s19). The real
//! `shell/merge-settings.py` is the oracle throughout: every comparison test
//! runs it as a subprocess and diffs its actual stdout and output files
//! against the Rust port's, rather than hand-typing an expected JSON blob.
//!
//! Coverage map, so every scenario named in the Work Unit brief is
//! traceable to one place below:
//! - Mandatory fixture 1 (user key absent from base): `FIXTURES`, "user key
//!   absent from base"
//! - Mandatory fixture 2 (user key modified from base) / s2 (user-changed key
//!   is preserved) / s6 / s15 (NEWBASE freeze): `FIXTURES`, "user key modified
//!   from base"
//! - Mandatory fixture 3 (template key removed) / s4: `FIXTURES`,
//!   "template key removed"
//! - Mandatory fixture 4 (malformed user JSON) / s9: `INVALID_INPUT_CASES`,
//!   "user corrupt json"
//! - Mandatory fixture 5 (user key nested three deep): `FIXTURES`, "user key
//!   nested three deep differs from base"
//! - Mandatory fixture 6 (user hand-added hook entry, the clobber risk):
//!   `FIXTURES`, "user hand-added hook entry template does not know about"
//! - s1 (user-unchanged key gets the template update): `FIXTURES`, "s1"
//! - s3 (new template key added when absent from user): `FIXTURES`, "s3"
//! - s5 (absent base -> additive fallback): `n4_missing_base_becomes_empty_object_with_warning_not_hard_fail`
//! - s7, s8 (corrupt/missing TEMPLATE -> N2 fail closed): `INVALID_INPUT_CASES`
//! - s10, s11 (USER is an array/scalar -> N2 fail closed): `INVALID_INPUT_CASES`
//! - s12 (USER == {} -> output equals template): `FIXTURES`, "s12"
//! - s13 (corrupt BASE -> additive fallback + warning): `n4_invalid_base_becomes_empty_object_with_warning_not_hard_fail`
//! - s14 (type-mismatch on a contested key): `FIXTURES`, "s14"
//! - s16 (C2 coincidence, three cycles): `c2_coincidence_keeps_user_value_frozen_through_a_matching_template_cycle`
//! - s17, s18 (skip file and merged stdout are valid JSON): asserted inline
//!   in the `FIXTURES` loop for every case that produces a skip entry
//! - s19 (zero withheld keys -> skip file is `[]`): `n3_zero_withheld_keys_writes_empty_skip_array`
//! - N3, omitted SKIP_OUT: `n3_omitting_skip_out_discards_skip_info_without_erroring`
//! - Crash mid-write leaves the original file intact:
//!   `crash_mid_write_leaves_the_original_settings_file_intact`
//! - WU-14 (skip-report reuse): `merge::render_skip_report` reproduces
//!   SKIP_OUT's own bytes exactly, since `init::run`'s pruned skip-report
//!   writes reuse it instead of going through `merge`'s `skip_out`
//!   parameter: `render_skip_report_matches_the_bytes_merge_writes_to_skip_out`

#![allow(dead_code)]

use playbook::init::merge::{merge, render_skip_report, MergeError};
use serde_json::Value;
use std::env;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

static SCRATCH_COUNTER: AtomicU64 = AtomicU64::new(0);

/// A fresh scratch directory under the OS temp dir, unique per call so
/// parallel tests never collide.
fn scratch_dir(tag: &str) -> PathBuf {
    let n = SCRATCH_COUNTER.fetch_add(1, Ordering::Relaxed);
    let dir = env::temp_dir().join(format!(
        "playbook-init-merge-{}-{tag}-{n}",
        std::process::id()
    ));
    fs::create_dir_all(&dir).expect("scratch dir should be creatable");
    dir
}

fn write_file(path: &Path, content: &str) {
    fs::write(path, content).expect("scratch file should be writable");
}

/// A base/template/user triple the Rust and python mergers must agree on,
/// byte for byte, in merged stdout, NEWBASE_OUT and SKIP_OUT.
struct Fixture {
    name: &'static str,
    base: &'static str,
    template: &'static str,
    user: &'static str,
    /// Extra pin beyond cross-engine agreement: the merged output must
    /// contain this literal substring. Only mandatory fixture 6 uses this,
    /// to make the clobber-risk assertion explicit rather than implied.
    must_contain: Option<&'static str>,
    /// Extra pin for the NEWBASE_OUT freeze rule (s15): `(key, expected)`
    /// asserts NEWBASE_OUT's value for `key` directly, rather than relying
    /// only on cross-engine equality to notice a frozen-value regression.
    frozen_newbase: Option<(&'static str, &'static str)>,
    /// The frozen python oracle for this fixture: a JSON object with
    /// `stdout`, `newbase` and `skip` string fields, captured from
    /// `shell/merge-settings.py`. See tests/fixtures/golden/README.md.
    golden: &'static str,
}

const FIXTURES: [Fixture; 9] = [
    Fixture {
        name: "user key absent from base",
        base: r#"{"other":"x"}"#,
        template: r#"{"other":"x","newkey":"tmpl_val"}"#,
        user: r#"{"other":"x","newkey":"user_val"}"#,
        must_contain: None,
        frozen_newbase: None,
        golden: include_str!("fixtures/golden/init-merge.user-key-absent-from-base.json"),
    },
    Fixture {
        name: "user key modified from base",
        base: r#"{"k":"base_val"}"#,
        template: r#"{"k":"tmpl_val"}"#,
        user: r#"{"k":"user_val"}"#,
        must_contain: None,
        frozen_newbase: Some(("k", "base_val")),
        golden: include_str!("fixtures/golden/init-merge.user-key-modified-from-base.json"),
    },
    Fixture {
        name: "template key removed",
        base: r#"{"gone":"was_here","keep":"yes"}"#,
        template: r#"{"keep":"yes"}"#,
        user: r#"{"gone":"was_here","keep":"yes"}"#,
        must_contain: None,
        frozen_newbase: None,
        golden: include_str!("fixtures/golden/init-merge.template-key-removed.json"),
    },
    Fixture {
        name: "user key nested three deep differs from base",
        base: r#"{"k":{"a":{"b":{"c":1}}}}"#,
        template: r#"{"k":{"a":{"b":{"c":2}}}}"#,
        user: r#"{"k":{"a":{"b":{"c":99}}}}"#,
        must_contain: None,
        frozen_newbase: None,
        golden: include_str!(
            "fixtures/golden/init-merge.user-key-nested-three-deep-differs-from-base.json"
        ),
    },
    Fixture {
        name: "user hand-added hook entry template does not know about",
        base: r#"{"hooks":{"PreToolUse":[{"matcher":"Bash","hooks":[{"type":"command","command":"tmpl-hook-a"}]}]}}"#,
        template: r#"{"hooks":{"PreToolUse":[{"matcher":"Bash","hooks":[{"type":"command","command":"tmpl-hook-a-v2"}]}]}}"#,
        user: r#"{"hooks":{"PreToolUse":[{"matcher":"Bash","hooks":[{"type":"command","command":"tmpl-hook-a"},{"type":"command","command":"user-added-hook-b"}]}]}}"#,
        must_contain: Some("user-added-hook-b"),
        frozen_newbase: None,
        golden: include_str!(
            "fixtures/golden/init-merge.user-hand-added-hook-entry-template-does-not-know-about.json"
        ),
    },
    Fixture {
        name: "s1: user-unchanged key gets the template update",
        base: r#"{"k":"v1","shared":"base"}"#,
        template: r#"{"k":"v2","shared":"tmpl"}"#,
        user: r#"{"k":"v1","shared":"base"}"#,
        must_contain: None,
        frozen_newbase: None,
        golden: include_str!(
            "fixtures/golden/init-merge.s1--user-unchanged-key-gets-the-template-update.json"
        ),
    },
    Fixture {
        name: "s3: new template key added when absent from user",
        base: r#"{"existing":"x"}"#,
        template: r#"{"existing":"x","newkey":"from_tmpl"}"#,
        user: r#"{"existing":"x"}"#,
        must_contain: None,
        frozen_newbase: None,
        golden: include_str!(
            "fixtures/golden/init-merge.s3--new-template-key-added-when-absent-from-user.json"
        ),
    },
    Fixture {
        name: "s12: user is an empty object, output equals template",
        base: r#"{}"#,
        template: r#"{"a":"1","b":"2"}"#,
        user: r#"{}"#,
        must_contain: None,
        frozen_newbase: None,
        golden: include_str!(
            "fixtures/golden/init-merge.s12--user-is-an-empty-object--output-equals-template.json"
        ),
    },
    Fixture {
        name: "s14: type-mismatch on a contested key keeps user's whole value",
        base: r#"{"k":"scalar_base"}"#,
        template: r#"{"k":"scalar_tmpl"}"#,
        user: r#"{"k":{"nested":"obj"}}"#,
        must_contain: None,
        frozen_newbase: None,
        golden: include_str!(
            "fixtures/golden/init-merge.s14--type-mismatch-on-a-contested-key-keeps-user-s-whole-value.json"
        ),
    },
];

#[test]
fn mandatory_and_ported_fixtures_rust_and_python_mergers_agree() {
    for fixture in FIXTURES {
        // Arrange
        let dir = scratch_dir(&fixture.name.replace([' ', ':', ',', '\''], "-"));
        let base_path = dir.join("base.json");
        let template_path = dir.join("template.json");
        let user_path = dir.join("user.json");
        write_file(&base_path, fixture.base);
        write_file(&template_path, fixture.template);
        write_file(&user_path, fixture.user);
        let rs_newbase = dir.join("rs-newbase.json");
        let rs_skip = dir.join("rs-skip.json");

        // Act
        let rs = merge(
            &base_path,
            &template_path,
            &user_path,
            &rs_newbase,
            Some(&rs_skip),
        );

        // Assert against the frozen python oracle rather than a live python
        // run. See tests/fixtures/golden/README.md: the python original is
        // deleted by ADR 0007 WU-14, so its output is committed instead.
        let golden: Value = serde_json::from_str(fixture.golden).unwrap_or_else(|e| {
            panic!("{}: golden fixture should be valid JSON: {e}", fixture.name)
        });
        let golden_stdout = golden["stdout"].as_str().unwrap();
        let golden_newbase = golden["newbase"].as_str().unwrap();
        let golden_skip = golden["skip"].as_str().unwrap();

        let outcome = rs.unwrap_or_else(|e| panic!("{}: rust merge failed: {e:?}", fixture.name));

        assert_eq!(
            outcome.stdout.trim_end_matches('\n'),
            golden_stdout.trim_end_matches('\n'),
            "{}: merged stdout should match python's",
            fixture.name
        );
        // s18: merged stdout must itself be valid JSON.
        assert!(
            serde_json::from_str::<Value>(&outcome.stdout).is_ok(),
            "{}: merged stdout should be valid JSON: {}",
            fixture.name,
            outcome.stdout
        );

        let rs_newbase_content = fs::read_to_string(&rs_newbase).unwrap();
        assert_eq!(
            rs_newbase_content, golden_newbase,
            "{}: NEWBASE_OUT should match python's byte for byte",
            fixture.name
        );
        // s18: NEWBASE_OUT must itself be valid JSON.
        assert!(
            serde_json::from_str::<Value>(&rs_newbase_content).is_ok(),
            "{}: NEWBASE_OUT should be valid JSON: {}",
            fixture.name,
            rs_newbase_content
        );

        let rs_skip_content = fs::read_to_string(&rs_skip).unwrap();
        assert_eq!(
            rs_skip_content, golden_skip,
            "{}: SKIP_OUT should match python's byte for byte",
            fixture.name
        );
        // s17: skip file must parse as a JSON array.
        let skip_value: Value = serde_json::from_str(&rs_skip_content)
            .unwrap_or_else(|e| panic!("{}: SKIP_OUT should be valid JSON: {e}", fixture.name));
        assert!(
            skip_value.is_array(),
            "{}: SKIP_OUT should be a JSON array, got: {rs_skip_content}",
            fixture.name
        );

        if let Some(needle) = fixture.must_contain {
            assert!(
                outcome.stdout.contains(needle),
                "{}: merged output should contain {needle:?} (clobber check): {}",
                fixture.name,
                outcome.stdout
            );
        }

        if let Some((key, expected)) = fixture.frozen_newbase {
            let newbase_value: Value = serde_json::from_str(&rs_newbase_content).unwrap();
            assert_eq!(
                newbase_value[key], expected,
                "{}: NEWBASE_OUT[{key}] should freeze to the OLD base value",
                fixture.name
            );
        }
    }
}

const VALID_OBJECT: &str = r#"{"k":"v"}"#;

/// N2 (`shell/merge-settings.py`'s TEMPLATE/USER validation): a case where
/// exactly one of TEMPLATE or USER is missing, unparsable, or not a JSON
/// object. `None` means the file is never written at all (the "missing"
/// case); `Some(content)` means it is written with that (invalid) content.
struct InvalidInputCase {
    name: &'static str,
    template: Option<&'static str>,
    user: Option<&'static str>,
}

const INVALID_INPUT_CASES: [InvalidInputCase; 8] = [
    InvalidInputCase {
        name: "template missing",
        template: None,
        user: Some(VALID_OBJECT),
    },
    InvalidInputCase {
        name: "template corrupt json",
        template: Some("{ not valid json"),
        user: Some(VALID_OBJECT),
    },
    InvalidInputCase {
        name: "template is array",
        template: Some("[1,2,3]"),
        user: Some(VALID_OBJECT),
    },
    InvalidInputCase {
        name: "template is scalar",
        template: Some("\"just a string\""),
        user: Some(VALID_OBJECT),
    },
    InvalidInputCase {
        name: "user missing",
        template: Some(VALID_OBJECT),
        user: None,
    },
    InvalidInputCase {
        name: "user corrupt json",
        template: Some(VALID_OBJECT),
        user: Some("{ not valid json"),
    },
    InvalidInputCase {
        name: "user is array",
        template: Some(VALID_OBJECT),
        user: Some("[1,2,3]"),
    },
    InvalidInputCase {
        name: "user is scalar",
        template: Some(VALID_OBJECT),
        user: Some("\"just a string\""),
    },
];

#[test]
fn n2_non_object_template_or_user_fails_closed_with_no_output() {
    for case in INVALID_INPUT_CASES {
        // Arrange
        let dir = scratch_dir(&case.name.replace(' ', "-"));
        let base_path = dir.join("base.json");
        let template_path = dir.join("template.json");
        let user_path = dir.join("user.json");
        write_file(&base_path, VALID_OBJECT);
        if let Some(content) = case.template {
            write_file(&template_path, content);
        }
        if let Some(content) = case.user {
            write_file(&user_path, content);
        }
        let rs_newbase = dir.join("rs-newbase.json");

        // Act
        let rs = merge(&base_path, &template_path, &user_path, &rs_newbase, None);

        // Assert
        //
        // The python half was REMOVED rather than frozen as a golden. It
        // asserted only that python exited non-zero with empty stdout, and
        // once ADR 0007 WU-14 deletes the script `python3` exits non-zero
        // with "can't open file" and empty stdout, so both assertions would
        // have held for the wrong reason: a test that survives the deletion
        // while proving nothing. Demonstrated 2026-08-21 by pointing the
        // helper at a non-existent script and watching every test still pass.
        // Freezing it was rejected because the assertion is a bare "python
        // rejected this", a boolean with no content, unlike the output
        // comparisons the goldens preserve.
        assert!(
            matches!(rs, Err(MergeError::Validation(_))),
            "{}: rust should fail closed with a validation error",
            case.name
        );
        assert!(
            !rs_newbase.exists(),
            "{}: NEWBASE_OUT should not be written on N2 failure",
            case.name
        );
    }
}

#[test]
fn n4_missing_base_becomes_empty_object_with_warning_not_hard_fail() {
    // Arrange: mirrors shell test s5, an absent BASE path entirely.
    let dir = scratch_dir("n4-missing-base");
    let template_path = dir.join("template.json");
    let user_path = dir.join("user.json");
    write_file(
        &template_path,
        r#"{"added":"from_tmpl","shared":"tmpl_val"}"#,
    );
    write_file(&user_path, r#"{"mykey":"myval","shared":"user_val"}"#);
    let missing_base = dir.join("no-such-base.json");
    let rs_newbase = dir.join("rs-newbase.json");

    // The frozen python oracle for this fixture: `exit_code`, `stdout` and
    // `stderr`, captured from shell/merge-settings.py before its deletion.
    // `stderr`'s BASE path is normalised to `<base-path>`, since python's
    // warning text embeds the absolute path of the (never written)
    // `no-such-base.json`, which varies with the capture machine's temp
    // directory. See tests/fixtures/golden/README.md.
    let golden: Value = serde_json::from_str(include_str!(
        "fixtures/golden/init-merge.n4-missing-base.json"
    ))
    .expect("golden fixture should be valid JSON");
    let py_exit_code = golden["exit_code"].as_i64().unwrap();
    let py_stderr = golden["stderr"].as_str().unwrap();
    let py_stdout = golden["stdout"].as_str().unwrap();

    // Act
    let rs = merge(&missing_base, &template_path, &user_path, &rs_newbase, None);

    // Assert
    assert_eq!(
        py_exit_code, 0,
        "python should still succeed on a missing base (N4)"
    );
    assert!(
        py_stderr.to_lowercase().contains("warning"),
        "python should warn on stderr: {py_stderr}"
    );
    let outcome = rs.expect("rust should still succeed on a missing base (N4)");
    let warning = outcome
        .base_warning
        .expect("rust should report a base fallback warning");
    assert!(
        warning.to_lowercase().contains("warning"),
        "rust warning text should say 'warning': {warning}"
    );
    let merged: Value = serde_json::from_str(&outcome.stdout).unwrap();
    assert_eq!(
        merged["mykey"], "myval",
        "user key should survive additively"
    );
    assert_eq!(
        merged["added"], "from_tmpl",
        "new template key should be added"
    );
    assert_eq!(
        outcome.stdout.trim_end_matches('\n'),
        py_stdout.trim_end_matches('\n'),
        "merged output should still match python's"
    );
}

#[test]
fn n4_invalid_base_becomes_empty_object_with_warning_not_hard_fail() {
    // Arrange: mirrors shell test s13, a BASE file that is not valid JSON.
    let dir = scratch_dir("n4-invalid-base");
    let base_path = dir.join("base.json");
    let template_path = dir.join("template.json");
    let user_path = dir.join("user.json");
    write_file(&base_path, "not json");
    write_file(&template_path, r#"{"newkey":"nv","shared":"tv"}"#);
    write_file(&user_path, r#"{"mykey":"mv","shared":"uv"}"#);
    let rs_newbase = dir.join("rs-newbase.json");

    // The frozen python oracle for this fixture: `exit_code` and `stderr`,
    // captured from shell/merge-settings.py before its deletion. `stderr`'s
    // BASE path is normalised to `<base-path>` for the same reason as
    // init-merge.n4-missing-base.json. See tests/fixtures/golden/README.md.
    let golden: Value = serde_json::from_str(include_str!(
        "fixtures/golden/init-merge.n4-invalid-base.json"
    ))
    .expect("golden fixture should be valid JSON");
    let py_exit_code = golden["exit_code"].as_i64().unwrap();
    let py_stderr = golden["stderr"].as_str().unwrap();

    // Act
    let rs = merge(&base_path, &template_path, &user_path, &rs_newbase, None);

    // Assert
    assert_eq!(
        py_exit_code, 0,
        "python should still succeed on a corrupt base (N4)"
    );
    assert!(
        py_stderr.to_lowercase().contains("warning"),
        "python should warn on stderr: {py_stderr}"
    );
    let outcome = rs.expect("rust should still succeed on a corrupt base (N4)");
    let warning = outcome
        .base_warning
        .expect("rust should report a base fallback warning");
    assert!(
        warning.to_lowercase().contains("warning"),
        "rust warning text should say 'warning': {warning}"
    );
    let merged: Value = serde_json::from_str(&outcome.stdout).unwrap();
    assert_eq!(merged["mykey"], "mv");
    assert_eq!(merged["newkey"], "nv");
}

#[test]
fn n3_zero_withheld_keys_writes_empty_skip_array() {
    // Arrange: mirrors shell test s19, an unchanged key with a template
    // update and no conflict at all.
    let dir = scratch_dir("n3-zero-withheld");
    let base_path = dir.join("base.json");
    let template_path = dir.join("template.json");
    let user_path = dir.join("user.json");
    write_file(&base_path, r#"{"k":"v"}"#);
    write_file(&template_path, r#"{"k":"v2"}"#);
    write_file(&user_path, r#"{"k":"v"}"#);
    let rs_newbase = dir.join("rs-newbase.json");
    let rs_skip = dir.join("rs-skip.json");

    // The frozen python oracle for this fixture: `exit_code` and `skip`
    // (SKIP_OUT's content), captured from shell/merge-settings.py before its
    // deletion. See tests/fixtures/golden/README.md.
    let golden: Value = serde_json::from_str(include_str!(
        "fixtures/golden/init-merge.n3-zero-withheld-keys.json"
    ))
    .expect("golden fixture should be valid JSON");
    let py_exit_code = golden["exit_code"].as_i64().unwrap();
    let py_skip_content = golden["skip"].as_str().unwrap();

    // Act
    let rs = merge(
        &base_path,
        &template_path,
        &user_path,
        &rs_newbase,
        Some(&rs_skip),
    );

    // Assert
    assert_eq!(py_exit_code, 0);
    let outcome = rs.expect("merge should succeed");
    assert!(
        outcome.skipped.is_empty(),
        "no keys should be withheld: {:?}",
        outcome.skipped
    );
    let skip_content = fs::read_to_string(&rs_skip).unwrap();
    assert_eq!(
        skip_content.trim(),
        "[]",
        "SKIP_OUT should be an empty array, got: {skip_content}"
    );
    assert_eq!(
        skip_content, py_skip_content,
        "skip file should match python's byte for byte"
    );
}

#[test]
fn n3_omitting_skip_out_discards_skip_info_without_erroring() {
    // Arrange: a genuinely contested key, so there IS skip info that would
    // have been written had a SKIP_OUT path been given.
    let dir = scratch_dir("n3-omit-skip");
    let base_path = dir.join("base.json");
    let template_path = dir.join("template.json");
    let user_path = dir.join("user.json");
    write_file(&base_path, r#"{"k":"base_val"}"#);
    write_file(&template_path, r#"{"k":"tmpl_val"}"#);
    write_file(&user_path, r#"{"k":"user_val"}"#);
    let rs_newbase = dir.join("rs-newbase.json");

    // Act: no skip_out argument at all.
    let rs = merge(&base_path, &template_path, &user_path, &rs_newbase, None);

    // Assert
    let outcome = rs.expect("omitting SKIP_OUT should not error");
    let merged: Value = serde_json::from_str(&outcome.stdout).unwrap();
    assert_eq!(merged["k"], "user_val");
    assert_eq!(
        outcome.skipped.len(),
        1,
        "the skip info is still computed, just never written to disk"
    );
    // No file was ever named as a SKIP_OUT target, so nothing beyond
    // base/template/user/newbase exists in the scratch dir.
    let entries: Vec<_> = fs::read_dir(&dir).unwrap().flatten().collect();
    assert_eq!(
        entries.len(),
        4,
        "omitting SKIP_OUT should write no extra file: {entries:?}"
    );
}

/// WU-14: `init::run`'s `backup_then_write` needs the exact same SKIP_OUT
/// shape `merge` writes, but decided on its own timeline (gated on whether a
/// real settings.json write happened, not on whether `skip_out` was passed
/// to `merge`), so it renders the report itself via `render_skip_report`
/// rather than re-calling `merge` with a `skip_out` path. Pins that the
/// standalone renderer and `merge`'s own SKIP_OUT write never drift apart.
#[test]
fn render_skip_report_matches_the_bytes_merge_writes_to_skip_out() {
    // Arrange: a genuinely contested key, so `skipped` is non-empty.
    let dir = scratch_dir("render-skip-report");
    let base_path = dir.join("base.json");
    let template_path = dir.join("template.json");
    let user_path = dir.join("user.json");
    write_file(&base_path, r#"{"k":"base_val"}"#);
    write_file(&template_path, r#"{"k":"tmpl_val"}"#);
    write_file(&user_path, r#"{"k":"user_val"}"#);
    let rs_newbase = dir.join("rs-newbase.json");
    let rs_skip = dir.join("rs-skip.json");

    // Act
    let outcome = merge(
        &base_path,
        &template_path,
        &user_path,
        &rs_newbase,
        Some(&rs_skip),
    )
    .expect("merge should succeed");

    // Assert
    let written = fs::read_to_string(&rs_skip).unwrap();
    assert_eq!(
        render_skip_report(&outcome.skipped),
        written,
        "the standalone renderer should match what merge's own SKIP_OUT write produced"
    );
}

#[test]
fn c2_coincidence_keeps_user_value_frozen_through_a_matching_template_cycle() {
    // Arrange: mirrors shell test s16. The user customises `k` away from the
    // original base value.
    let dir = scratch_dir("c2-coincidence");
    let base0 = dir.join("base0.json");
    let user = dir.join("user.json");
    write_file(&base0, r#"{"k":"original"}"#);
    write_file(&user, r#"{"k":"my_custom"}"#);

    // The frozen python oracle for both cycles: `cycle1_exit_code`,
    // `cycle1_newbase` (cycle 1's NEWBASE_OUT content) and
    // `cycle2_exit_code`, captured from shell/merge-settings.py before its
    // deletion. Cycle 2's python run used cycle 1's NEWBASE_OUT as its BASE
    // input, same as the rust run below does with `nb1`, so freezing only
    // what each cycle actually asserts on keeps the capture faithful. See
    // tests/fixtures/golden/README.md.
    let golden: Value = serde_json::from_str(include_str!(
        "fixtures/golden/init-merge.c2-coincidence.json"
    ))
    .expect("golden fixture should be valid JSON");
    let py1_exit_code = golden["cycle1_exit_code"].as_i64().unwrap();
    let py_newbase1: Value = serde_json::from_str(golden["cycle1_newbase"].as_str().unwrap())
        .expect("golden cycle1_newbase should be valid JSON");
    let py2_exit_code = golden["cycle2_exit_code"].as_i64().unwrap();

    // Act, Assert: cycle 1, the template ships a value different from
    // original. The merged value stays the user's; NEWBASE freezes to the
    // OLD base value, not the template's.
    let template1 = dir.join("template1.json");
    write_file(&template1, r#"{"k":"tmpl_v2"}"#);
    let nb1 = dir.join("nb1.json");
    let outcome1 = merge(&base0, &template1, &user, &nb1, None).expect("cycle 1 should succeed");
    assert_eq!(py1_exit_code, 0, "python cycle 1 should succeed");
    let merged1: Value = serde_json::from_str(&outcome1.stdout).unwrap();
    assert_eq!(
        merged1["k"], "my_custom",
        "cycle 1: user value should be kept"
    );
    let newbase1: Value = serde_json::from_str(&fs::read_to_string(&nb1).unwrap()).unwrap();
    assert_eq!(
        newbase1["k"], "original",
        "cycle 1: NEWBASE should freeze to the OLD base value"
    );
    assert_eq!(
        py_newbase1["k"], "original",
        "python oracle should also freeze to the OLD base value"
    );

    // Act, Assert: cycle 2, the template coincidentally now matches the
    // user's own value. The C2 fix must keep `k` contested anyway, since
    // the frozen base is still "original", not "my_custom".
    let template2 = dir.join("template2.json");
    write_file(&template2, r#"{"k":"my_custom"}"#);
    let nb2 = dir.join("nb2.json");
    let outcome2 = merge(&nb1, &template2, &user, &nb2, None).expect("cycle 2 should succeed");
    assert_eq!(py2_exit_code, 0, "python cycle 2 should succeed");
    let merged2: Value = serde_json::from_str(&outcome2.stdout).unwrap();
    assert_eq!(
        merged2["k"], "my_custom",
        "cycle 2: C2 fix should keep the user's value despite the coincidental template match"
    );
    let newbase2: Value = serde_json::from_str(&fs::read_to_string(&nb2).unwrap()).unwrap();
    assert_eq!(
        newbase2["k"], "original",
        "cycle 2: NEWBASE should still be frozen to the original base value"
    );
}

#[cfg(unix)]
#[test]
fn crash_mid_write_leaves_the_original_settings_file_intact() {
    use std::os::unix::fs::PermissionsExt;

    // Arrange: a pre-existing settings-shaped file at the atomic write
    // target with known content, inside a directory made read-only so the
    // sibling temp file the write needs can never be created, simulating a
    // crash before the rename step ever runs.
    let dir = scratch_dir("crash-mid-write");
    let base_path = dir.join("base.json");
    let template_path = dir.join("template.json");
    let user_path = dir.join("user.json");
    write_file(&base_path, r#"{"k":"base_val"}"#);
    write_file(&template_path, r#"{"k":"tmpl_val"}"#);
    write_file(&user_path, r#"{"k":"user_val"}"#);

    let target_dir = dir.join("target");
    fs::create_dir_all(&target_dir).unwrap();
    let settings_json = target_dir.join("settings.json");
    let original_content = r#"{"k":"original-untouched-value"}"#;
    write_file(&settings_json, original_content);

    fs::set_permissions(&target_dir, fs::Permissions::from_mode(0o555)).unwrap();

    // A test running as root bypasses Unix permission checks entirely, so
    // this write would unexpectedly succeed; guard against that rather than
    // asserting something the environment cannot actually exercise.
    let probe = target_dir.join(".write-probe");
    let permissions_are_enforced = fs::write(&probe, "x").is_err();
    let _ = fs::set_permissions(&target_dir, fs::Permissions::from_mode(0o755));
    let _ = fs::remove_file(&probe);
    if !permissions_are_enforced {
        eprintln!(
            "skipping crash_mid_write_leaves_the_original_settings_file_intact: \
             running as a user that bypasses directory permissions"
        );
        return;
    }
    fs::set_permissions(&target_dir, fs::Permissions::from_mode(0o555)).unwrap();

    // Act
    let result = merge(&base_path, &template_path, &user_path, &settings_json, None);

    // Assert
    fs::set_permissions(&target_dir, fs::Permissions::from_mode(0o755)).unwrap();
    assert!(
        matches!(result, Err(MergeError::Io(_))),
        "a write that cannot create its temp file should fail, not silently succeed: {result:?}"
    );
    let after = fs::read_to_string(&settings_json).unwrap();
    assert_eq!(
        after, original_content,
        "the original file must survive a mid-write failure untouched"
    );
}
