// SPDX-FileCopyrightText: 2026 Igor Santos
// SPDX-License-Identifier: Apache-2.0

//! In `auto`, denies `AskUserQuestion`. On the first prompt of a session in the default
//! permission mode it also advises a trusted mode, once. The standing auto-mode rule is
//! injected by `session-init` at SessionStart, not repeated on every prompt.

use crate::common::mode::{resolve_for_hook, Mode};
use crate::common::payload::Payload;
use crate::common::{emit_pre_deny, emit_prompt_context, session_dir};
use std::path::Path;

const DENY_REASON: &str = "AUTO MODE: no questions. Choose the option marked recommended, or the first option when none is marked, record it as an assumption, and continue.";

const TRUSTED_MODE_ADVICE: &str =
    "AUTO MODE: permission prompts will stall an unattended run. Launch with a trusted permission mode.";

/// Session marker: the advice was already given.
const ADVISED_MARKER: &str = "auto-advice-given";

pub fn run(payload: &Payload) {
    let event = payload.field(".hook_event_name");
    let guards_question = event == "PreToolUse" && payload.field(".tool_name") == "AskUserQuestion";
    if !guards_question && event != "UserPromptSubmit" {
        return;
    }
    if resolve_for_hook().mode != Mode::Auto {
        return;
    }
    if guards_question {
        emit_pre_deny(DENY_REASON);
    } else if payload.field(".permission_mode") == "default" && first_advice(payload) {
        emit_prompt_context(TRUSTED_MODE_ADVICE);
    }
}

/// True the first time it is called in a session, and records that. A session
/// with no directory has nowhere to record it and always advises.
fn first_advice(payload: &Payload) -> bool {
    let dir = session_dir(payload);
    if dir.is_empty() {
        return true;
    }
    let marker = Path::new(&dir).join(ADVISED_MARKER);
    if marker.exists() {
        return false;
    }
    let _ = std::fs::write(marker, "1");
    true
}
