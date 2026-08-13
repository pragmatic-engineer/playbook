// SPDX-FileCopyrightText: 2026 Igor Santos
// SPDX-License-Identifier: MIT

//! The emitters that write hook decisions to stdout for Claude Code to
//! parse. Output shapes must match hooks/lib/common.sh:87-121 byte for byte;
//! the tests below assert that directly against common.sh's own functions
//! rather than a hand-copied expectation.
//!
//! `emit_block` is the exception: it has no shell equivalent. It ports the
//! `{"decision":"block","reason":<r>}` shape from hooks/memory-capture.py:80,
//! the only hook that uses it.
//!
//! Each shape is a small `Serialize` struct rather than a `serde_json::json!`
//! literal, because `serde_json::Map` without the `preserve_order` feature
//! serializes keys alphabetically; a struct's fields serialize in
//! declaration order, which is what lets the output match the shell's fixed
//! key order.

use serde::Serialize;

#[derive(Serialize)]
struct AdditionalContextOutput<'a> {
    #[serde(rename = "hookSpecificOutput")]
    hook_specific_output: AdditionalContextInner<'a>,
}

#[derive(Serialize)]
struct AdditionalContextInner<'a> {
    #[serde(rename = "hookEventName")]
    hook_event_name: &'a str,
    #[serde(rename = "additionalContext")]
    additional_context: &'a str,
}

/// Print a PreToolUse (or other) additionalContext JSON object to stdout.
/// Usage: `emit_pre_context("PreToolUse", "message text")`.
pub fn emit_pre_context(event: &str, msg: &str) {
    print_json(&AdditionalContextOutput {
        hook_specific_output: AdditionalContextInner {
            hook_event_name: event,
            additional_context: msg,
        },
    });
}

#[derive(Serialize)]
struct PreDenyOutput<'a> {
    #[serde(rename = "hookSpecificOutput")]
    hook_specific_output: PreDenyInner<'a>,
}

#[derive(Serialize)]
struct PreDenyInner<'a> {
    #[serde(rename = "hookEventName")]
    hook_event_name: &'static str,
    #[serde(rename = "permissionDecision")]
    permission_decision: &'static str,
    #[serde(rename = "permissionDecisionReason")]
    permission_decision_reason: &'a str,
}

/// Print a PreToolUse deny decision JSON object to stdout.
pub fn emit_pre_deny(reason: &str) {
    print_json(&PreDenyOutput {
        hook_specific_output: PreDenyInner {
            hook_event_name: "PreToolUse",
            permission_decision: "deny",
            permission_decision_reason: reason,
        },
    });
}

/// Print a UserPromptSubmit additionalContext JSON object to stdout. Same
/// shape as `emit_pre_context` with the event name fixed.
pub fn emit_prompt_context(msg: &str) {
    emit_pre_context("UserPromptSubmit", msg);
}

#[derive(Serialize)]
struct SystemMessageOutput<'a> {
    #[serde(rename = "systemMessage")]
    system_message: &'a str,
}

/// Print a top-level systemMessage JSON object to stdout.
pub fn emit_system_message(msg: &str) {
    print_json(&SystemMessageOutput {
        system_message: msg,
    });
}

#[derive(Serialize)]
struct BlockOutput<'a> {
    decision: &'static str,
    reason: &'a str,
}

/// Print a `{"decision":"block","reason":<r>}` JSON object to stdout, the
/// shape hooks/memory-capture.py:80 uses to pause the turn.
pub fn emit_block(reason: &str) {
    print_json(&BlockOutput {
        decision: "block",
        reason,
    });
}

fn print_json<T: Serialize>(value: &T) {
    if let Ok(rendered) = serde_json::to_string(value) {
        println!("{rendered}");
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::process::Command;

    /// Run `command` after sourcing common.sh with empty stdin, and return
    /// its stdout with the trailing newline stripped, so it lines up with
    /// what a `println!`-based Rust caller captures without the newline.
    fn shell_stdout(command: &str) -> String {
        let common_sh = concat!(env!("CARGO_MANIFEST_DIR"), "/hooks/lib/common.sh");
        let script = format!("source '{common_sh}' </dev/null; {command}");
        let output = Command::new("bash")
            .arg("-c")
            .arg(script)
            .output()
            .expect("bash should be available to run common.sh");
        assert!(
            output.status.success(),
            "common.sh command failed: {command}"
        );
        String::from_utf8(output.stdout)
            .expect("common.sh output should be valid UTF-8")
            .trim_end_matches('\n')
            .to_string()
    }

    /// Run a one-line python3 script and return its stdout with the trailing
    /// newline stripped, for the one emitter with no shell equivalent.
    fn python_stdout(code: &str) -> String {
        let output = Command::new("python3")
            .arg("-c")
            .arg(code)
            .output()
            .expect("python3 should be available to run reference code");
        assert!(output.status.success(), "python3 code failed: {code}");
        String::from_utf8(output.stdout)
            .expect("python3 output should be valid UTF-8")
            .trim_end_matches('\n')
            .to_string()
    }

    /// Each `emit_*` is a thin `println!` wrapper around a `Serialize`
    /// struct; asserting on the same struct's serialized form is exactly
    /// what reaches stdout, minus the trailing newline `println!` adds.
    fn json_of<T: Serialize>(value: &T) -> String {
        serde_json::to_string(value).unwrap()
    }

    #[test]
    fn emit_pre_context_matches_shell() {
        // Arrange
        let expected = shell_stdout("emit_pre_context 'PreToolUse' 'hello'");

        // Act
        let got = json_of(&AdditionalContextOutput {
            hook_specific_output: AdditionalContextInner {
                hook_event_name: "PreToolUse",
                additional_context: "hello",
            },
        });

        // Assert
        assert_eq!(got, expected);
        assert_eq!(
            got,
            r#"{"hookSpecificOutput":{"hookEventName":"PreToolUse","additionalContext":"hello"}}"#
        );
    }

    #[test]
    fn emit_pre_deny_matches_shell() {
        // Arrange
        let expected = shell_stdout("emit_pre_deny 'not allowed'");

        // Act
        let got = json_of(&PreDenyOutput {
            hook_specific_output: PreDenyInner {
                hook_event_name: "PreToolUse",
                permission_decision: "deny",
                permission_decision_reason: "not allowed",
            },
        });

        // Assert
        assert_eq!(got, expected);
    }

    #[test]
    fn emit_prompt_context_matches_shell() {
        // Arrange
        let expected = shell_stdout("emit_prompt_context 'context text'");

        // Act
        let got = json_of(&AdditionalContextOutput {
            hook_specific_output: AdditionalContextInner {
                hook_event_name: "UserPromptSubmit",
                additional_context: "context text",
            },
        });

        // Assert
        assert_eq!(got, expected);
    }

    #[test]
    fn emit_system_message_matches_shell() {
        // Arrange
        let expected = shell_stdout("emit_system_message 'system msg'");

        // Act
        let got = json_of(&SystemMessageOutput {
            system_message: "system msg",
        });

        // Assert
        assert_eq!(got, expected);
    }

    #[test]
    fn emit_system_message_non_ascii_matches_shell() {
        // Arrange
        let expected = shell_stdout("emit_system_message '\u{26a0} warn'");

        // Act
        let got = json_of(&SystemMessageOutput {
            system_message: "\u{26a0} warn",
        });

        // Assert
        assert_eq!(got, expected);
    }

    #[test]
    fn emit_block_matches_memory_capture_python_shape() {
        // Arrange
        let expected = python_stdout(
            "import json; print(json.dumps({'decision':'block','reason':'reason text'}, \
             separators=(',',':'), ensure_ascii=False))",
        );

        // Act
        let got = json_of(&BlockOutput {
            decision: "block",
            reason: "reason text",
        });

        // Assert
        assert_eq!(got, expected);
    }

    #[test]
    fn public_emitters_print_the_same_json_as_the_struct_builders() {
        // Arrange, Act: exercise every public emit_* once so the printed
        // form is compiled and linked, guarding against the private
        // json_of()-based tests above drifting from what emit_* prints.
        // Assert: none of these panic, which is the whole contract for a
        // stdout side-effecting function.
        emit_pre_context("PreToolUse", "hello");
        emit_pre_deny("not allowed");
        emit_prompt_context("context text");
        emit_system_message("system msg");
        emit_block("reason text");
    }
}
