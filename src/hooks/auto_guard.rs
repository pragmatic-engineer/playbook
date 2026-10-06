// SPDX-FileCopyrightText: 2026 Igor Santos
// SPDX-License-Identifier: MIT

//! Denies `AskUserQuestion` while the resolved mode is `auto`.

use crate::common::emit_pre_deny;
use crate::common::mode::{resolve_for_hook, Mode};
use crate::common::payload::Payload;

const DENY_REASON: &str = "AUTO MODE: no questions. Choose the option marked recommended, or the first option when none is marked, record it as an assumption, and continue.";

pub fn run(payload: &Payload) {
    if payload.field(".hook_event_name") != "PreToolUse"
        || payload.field(".tool_name") != "AskUserQuestion"
    {
        return;
    }
    if resolve_for_hook().mode == Mode::Auto {
        emit_pre_deny(DENY_REASON);
    }
}
