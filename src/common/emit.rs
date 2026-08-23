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

    /// The frozen output of `hooks/lib/common.sh`'s emitters, captured while
    /// that file still existed. See tests/fixtures/golden/README.md.
    ///
    /// These five tests were differential: they sourced common.sh and asserted
    /// the Rust port matched byte for byte. ADR 0007 WU-14 deletes common.sh,
    /// which would have removed the oracle and quietly downgraded them to
    /// "Rust agrees with itself". Reading a committed golden keeps the check,
    /// and keeps it working on a machine with no bash at all.
    fn shell_golden(key: &str) -> String {
        let raw = include_str!("../../tests/fixtures/golden/common-sh.emitters.json");
        let bundle: serde_json::Value =
            serde_json::from_str(raw).expect("golden bundle should be valid JSON");
        bundle[key]
            .as_str()
            .unwrap_or_else(|| panic!("golden bundle has no key {key}"))
            .to_string()
    }

    /// The frozen output of the python one-liner that was the oracle for the
    /// one emitter with no shell equivalent. Frozen for the same reason, and it
    /// removes the last reason `cargo test` needed python3 on the machine.
    fn python_golden() -> String {
        include_str!("../../tests/fixtures/golden/memory-capture.block.txt")
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
        let expected = shell_golden("emit_pre_context");

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
        let expected = shell_golden("emit_pre_deny");

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
        let expected = shell_golden("emit_prompt_context");

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
        let expected = shell_golden("emit_system_message");

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
        let expected = shell_golden("emit_system_message_non_ascii");

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
        let expected = python_golden();

        // Act
        let got = json_of(&BlockOutput {
            decision: "block",
            reason: "reason text",
        });

        // Assert
        assert_eq!(got, expected);
    }

    /// Re-invoke this test binary as a child process to run `emit_probe`
    /// below with `EMIT_PROBE` naming one emitter, so that emitter's own
    /// real stdout can be captured from a subprocess. This process's own
    /// stdout is
    /// shared with every other test thread, so it cannot be read directly;
    /// a fresh child process is the only way to observe just one call's
    /// output.
    fn capture_emitter_stdout(probe: &str) -> String {
        let exe = std::env::current_exe().expect("test binary path should be available");
        let output = Command::new(exe)
            .arg("common::emit::tests::emit_probe")
            .args(["--exact", "--nocapture"])
            .env("EMIT_PROBE", probe)
            .output()
            .expect("re-invoking the test binary should succeed");
        assert!(output.status.success(), "emit_probe child process failed");
        let stdout = String::from_utf8(output.stdout).expect("probe stdout should be valid UTF-8");
        stdout
            .lines()
            .find(|line| line.starts_with('{'))
            .unwrap_or_else(|| panic!("no JSON line in probe output for {probe}: {stdout}"))
            .to_string()
    }

    /// Prints exactly one emitter's real stdout when `EMIT_PROBE` names it.
    /// A no-op otherwise, so this still runs harmlessly as part of the
    /// normal suite; only `capture_emitter_stdout` above invokes it with
    /// the env var set.
    #[test]
    fn emit_probe() {
        match std::env::var("EMIT_PROBE").as_deref() {
            Ok("pre_context") => emit_pre_context("PreToolUse", "hello"),
            Ok("pre_deny") => emit_pre_deny("not allowed"),
            Ok("prompt_context") => emit_prompt_context("context text"),
            Ok("system_message") => emit_system_message("system msg"),
            Ok("block") => emit_block("reason text"),
            _ => {}
        }
    }

    #[test]
    fn public_emitters_print_the_same_json_as_the_struct_builders() {
        // Arrange: the exact JSON each emit_* call's own struct implies,
        // paired with the probe name that makes emit_probe call it for
        // real.
        let cases: [(&str, String); 5] = [
            (
                "pre_context",
                json_of(&AdditionalContextOutput {
                    hook_specific_output: AdditionalContextInner {
                        hook_event_name: "PreToolUse",
                        additional_context: "hello",
                    },
                }),
            ),
            (
                "pre_deny",
                json_of(&PreDenyOutput {
                    hook_specific_output: PreDenyInner {
                        hook_event_name: "PreToolUse",
                        permission_decision: "deny",
                        permission_decision_reason: "not allowed",
                    },
                }),
            ),
            (
                "prompt_context",
                json_of(&AdditionalContextOutput {
                    hook_specific_output: AdditionalContextInner {
                        hook_event_name: "UserPromptSubmit",
                        additional_context: "context text",
                    },
                }),
            ),
            (
                "system_message",
                json_of(&SystemMessageOutput {
                    system_message: "system msg",
                }),
            ),
            (
                "block",
                json_of(&BlockOutput {
                    decision: "block",
                    reason: "reason text",
                }),
            ),
        ];

        // Act, Assert: each emitter's real, captured stdout must equal the
        // JSON its own struct builder produces. A mutation that printed a
        // different shape, dropped a field, or wrote to stderr instead of
        // stdout now fails here instead of only being checked for "does
        // not panic".
        for (probe, expected) in cases {
            assert_eq!(
                capture_emitter_stdout(probe),
                expected,
                "{probe} emitter's real stdout should match its struct builder's JSON"
            );
        }
    }
}
