// SPDX-FileCopyrightText: 2026 Igor Santos
// SPDX-License-Identifier: Apache-2.0

//! In `auto`, denies `AskUserQuestion` and adds a standing note to every user prompt.

use crate::common::mode::{resolve_for_hook, Mode};
use crate::common::payload::Payload;
use crate::common::{emit_pre_deny, emit_prompt_context};

const DENY_REASON: &str = "AUTO MODE: no questions. Choose the option marked recommended, or the first option when none is marked, record it as an assumption, and continue.";

const PROMPT_NOTE: &str = "AUTO MODE is on: wherever the command allows it, take the recommended answer and log it as an assumption instead of asking.";

const TRUSTED_MODE_ADVICE: &str =
    "Permission prompts will stall an unattended run: launch with a trusted permission mode.";

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
    } else {
        emit_prompt_context(&prompt_note(payload));
    }
}

fn prompt_note(payload: &Payload) -> String {
    if payload.field(".permission_mode") == "default" {
        format!("{PROMPT_NOTE} {TRUSTED_MODE_ADVICE}")
    } else {
        PROMPT_NOTE.to_string()
    }
}
