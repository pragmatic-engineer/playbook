// SPDX-FileCopyrightText: 2026 Igor Santos
// SPDX-License-Identifier: MIT

//! `auto-guard` as seen from the real binary: in auto mode it denies
//! `AskUserQuestion` with a take-the-recommended-option reason, and in every
//! other case it prints nothing. Payloads are real Claude Code captures under
//! `tests/fixtures/hooks/`.

#[path = "support/auto_env.rs"]
mod auto_env;

use auto_env::{hook_command, run_hook, scratch, CLEARED_VARS};
use serde_json::Value;
use std::ffi::OsStr;

const HOOK: &str = "auto-guard";
const ASK_USER_QUESTION: &str = include_str!("fixtures/hooks/pre-askuserquestion.json");
const USER_PROMPT_SUBMIT: &str = include_str!("fixtures/hooks/user-prompt-submit.json");

/// The captured PreToolUse payload with its tool swapped to `Bash`.
fn bash_payload() -> String {
    ASK_USER_QUESTION.replace(r#""tool_name":"AskUserQuestion""#, r#""tool_name":"Bash""#)
}

fn parse(out: &str) -> Value {
    serde_json::from_str(out.trim()).unwrap_or_else(|e| panic!("stdout is not JSON ({e}): {out}"))
}

const STANDING_LINE: &str = "AUTO MODE is on: wherever the command allows it, take the recommended answer and log it as an assumption instead of asking.";
const TRUSTED_MODE_ADVICE: &str = "launch with a trusted permission mode";

/// The captured UserPromptSubmit payload with its `permission_mode` swapped
/// for `mode`, or dropped entirely when `mode` is `None`.
fn prompt_payload(mode: Option<&str>) -> String {
    let captured = r#""permission_mode":"default","#;
    assert!(
        USER_PROMPT_SUBMIT.contains(captured),
        "the fixture must carry permission_mode default"
    );
    let replacement = mode.map_or(String::new(), |m| format!(r#""permission_mode":"{m}","#));
    USER_PROMPT_SUBMIT.replace(captured, &replacement)
}

/// `hookSpecificOutput.additionalContext` of a UserPromptSubmit reply, or an
/// empty string when the hook printed nothing.
fn prompt_context(out: &str) -> String {
    if out.trim().is_empty() {
        return String::new();
    }
    let value = parse(out);
    let decision = &value["hookSpecificOutput"];
    assert_eq!(decision["hookEventName"], "UserPromptSubmit");
    decision["additionalContext"]
        .as_str()
        .unwrap_or("")
        .to_string()
}

#[test]
fn auto_mode_denies_ask_user_question_and_names_the_recommended_option() {
    // Arrange
    let s = scratch("deny");

    // Act
    let (out, code) = run_hook(&s, HOOK, ASK_USER_QUESTION, &[("PLAYBOOK_MODE", "auto")]);

    // Assert
    assert_eq!(code, 0);
    let decision = &parse(&out)["hookSpecificOutput"];
    assert_eq!(decision["hookEventName"], "PreToolUse");
    assert_eq!(decision["permissionDecision"], "deny");
    let reason = decision["permissionDecisionReason"].as_str().unwrap_or("");
    assert!(
        reason.contains("Choose the option marked recommended"),
        "reason missing the recommended-option instruction: {reason}"
    );
}

#[test]
fn auto_mode_never_denies_another_tool() {
    // Arrange
    let s = scratch("bash");

    // Act
    let (out, code) = run_hook(&s, HOOK, &bash_payload(), &[("PLAYBOOK_MODE", "auto")]);

    // Assert
    assert_eq!(code, 0);
    assert_eq!(out, "", "a Bash call must produce no output");
}

#[test]
fn auto_mode_user_prompt_submit_carries_no_deny() {
    // Arrange
    let s = scratch("prompt");

    // Act
    let (out, code) = run_hook(&s, HOOK, USER_PROMPT_SUBMIT, &[("PLAYBOOK_MODE", "auto")]);

    // Assert
    assert_eq!(code, 0);
    assert!(
        !out.contains("permissionDecision"),
        "a prompt event must never carry a deny: {out}"
    );
}

#[test]
fn ask_mode_prints_nothing_for_ask_user_question() {
    // Arrange
    let s = scratch("ask");

    // Act
    let (out, code) = run_hook(&s, HOOK, ASK_USER_QUESTION, &[("PLAYBOOK_MODE", "ask")]);

    // Assert
    assert_eq!(code, 0);
    assert_eq!(out, "");
}

#[test]
fn env_ask_overrides_config_auto() {
    // Arrange
    let s = scratch("env-over-config");
    s.seed_mode_config("auto");

    // Act
    let (out, code) = run_hook(&s, HOOK, ASK_USER_QUESTION, &[("PLAYBOOK_MODE", "ask")]);

    // Assert
    assert_eq!(code, 0);
    assert_eq!(out, "", "env ask must beat config auto");
}

#[test]
fn config_auto_alone_denies_ask_user_question() {
    // Arrange
    let s = scratch("config-auto");
    s.seed_mode_config("auto");

    // Act
    let (out, code) = run_hook(&s, HOOK, ASK_USER_QUESTION, &[]);

    // Assert
    assert_eq!(code, 0);
    assert_eq!(
        parse(&out)["hookSpecificOutput"]["permissionDecision"],
        "deny"
    );
}

#[test]
fn malformed_stdin_exits_zero_with_no_output() {
    // Arrange
    let s = scratch("malformed");

    // Act
    let (out, code) = run_hook(
        &s,
        HOOK,
        r#"{"tool_name":"AskUserQuestion","tool_input":{"#,
        &[("PLAYBOOK_MODE", "auto")],
    );

    // Assert
    assert_eq!(code, 0);
    assert_eq!(out, "");
}

#[test]
fn auto_mode_user_prompt_submit_carries_the_standing_line() {
    // Arrange
    let s = scratch("standing-line");

    // Act
    let (out, code) = run_hook(&s, HOOK, USER_PROMPT_SUBMIT, &[("PLAYBOOK_MODE", "auto")]);

    // Assert
    assert_eq!(code, 0);
    let context = prompt_context(&out);
    assert!(
        context.contains(STANDING_LINE),
        "additionalContext missing the standing line: {out}"
    );
}

#[test]
fn auto_mode_repeats_the_standing_line_on_every_prompt() {
    // Arrange
    let s = scratch("not-deduped");

    // Act
    let first = run_hook(&s, HOOK, USER_PROMPT_SUBMIT, &[("PLAYBOOK_MODE", "auto")]);
    let second = run_hook(&s, HOOK, USER_PROMPT_SUBMIT, &[("PLAYBOOK_MODE", "auto")]);

    // Assert
    assert!(prompt_context(&first.0).contains(STANDING_LINE));
    assert!(
        prompt_context(&second.0).contains(STANDING_LINE),
        "the second identical prompt must carry the line again: {}",
        second.0
    );
}

#[test]
fn auto_mode_emits_the_standing_line_for_a_slash_prompt() {
    // Arrange
    let s = scratch("slash");
    let payload = USER_PROMPT_SUBMIT.replace(r#""prompt":"Use"#, r#""prompt":"/playbook:plan Use"#);
    assert!(payload.contains(r#""prompt":"/playbook:plan"#));

    // Act
    let (out, code) = run_hook(&s, HOOK, &payload, &[("PLAYBOOK_MODE", "auto")]);

    // Assert
    assert_eq!(code, 0);
    assert!(
        prompt_context(&out).contains(STANDING_LINE),
        "a slash prompt must still carry the line: {out}"
    );
}

#[test]
fn ask_mode_user_prompt_submit_prints_nothing() {
    // Arrange
    let s = scratch("ask-prompt");

    // Act
    let (out, code) = run_hook(&s, HOOK, USER_PROMPT_SUBMIT, &[("PLAYBOOK_MODE", "ask")]);

    // Assert
    assert_eq!(code, 0);
    assert_eq!(out, "", "ask mode must add no context");
}

#[test]
fn auto_mode_with_default_permission_mode_advises_a_trusted_mode() {
    // Arrange
    let s = scratch("perm-default");

    // Act
    let (out, code) = run_hook(
        &s,
        HOOK,
        &prompt_payload(Some("default")),
        &[("PLAYBOOK_MODE", "auto")],
    );

    // Assert
    assert_eq!(code, 0);
    let context = prompt_context(&out);
    assert!(context.contains(STANDING_LINE), "standing line lost: {out}");
    assert!(
        context.contains(TRUSTED_MODE_ADVICE),
        "default permission mode must add the advice: {out}"
    );
}

#[test]
fn auto_mode_gives_no_trusted_mode_advice_for_other_permission_modes() {
    for mode in [
        Some("acceptEdits"),
        Some("plan"),
        Some("bypassPermissions"),
        Some("dontAsk"),
        Some("auto"),
        Some("someFutureMode"),
        None,
    ] {
        // Arrange
        let s = scratch("perm-other");

        // Act
        let (out, code) = run_hook(
            &s,
            HOOK,
            &prompt_payload(mode),
            &[("PLAYBOOK_MODE", "auto")],
        );

        // Assert
        assert_eq!(code, 0, "permission_mode {mode:?}");
        let context = prompt_context(&out);
        assert!(
            context.contains(STANDING_LINE),
            "permission_mode {mode:?} must still carry the standing line: {out}"
        );
        assert!(
            !context.contains(TRUSTED_MODE_ADVICE),
            "permission_mode {mode:?} must not carry the advice: {out}"
        );
    }
}

#[test]
fn helper_isolates_the_child_env_without_touching_the_parent() {
    // Arrange
    let s = scratch("isolation");
    let parent_before: Vec<_> = std::env::vars_os().collect();

    // Act
    let command = hook_command(&s, HOOK);
    let envs: Vec<_> = command.get_envs().collect();

    // Assert
    let removed = |key: &str| {
        envs.iter()
            .any(|(k, v)| *k == OsStr::new(key) && v.is_none())
    };
    for var in CLEARED_VARS {
        assert!(removed(var), "{var} must be removed from the child env");
    }
    // Variables a hook reads that a developer's shell may well export.
    for var in [
        "CLAUDE_PLUGIN_ROOT",
        "AUTO_LEARN_NUDGE",
        "SKILLS_PRIMER",
        "ASYNC_DISCIPLINE",
        "STATUSLINE_CACHE_DIR",
        "PLAYBOOK_AUTO_COST_INTERVAL_MS",
    ] {
        assert!(CLEARED_VARS.contains(&var), "{var} must be in CLEARED_VARS");
        assert!(removed(var), "{var} must be removed from the child env");
    }
    let home = envs
        .iter()
        .find(|(k, _)| *k == OsStr::new("HOME"))
        .and_then(|(_, v)| *v)
        .expect("HOME must be set on the child");
    assert_eq!(home, s.home.as_os_str());
    assert!(home.to_string_lossy().contains("auto-env-"));
    assert_eq!(
        std::env::vars_os().collect::<Vec<_>>(),
        parent_before,
        "the parent env must not change"
    );
}
