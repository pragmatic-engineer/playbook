// SPDX-FileCopyrightText: 2026 Igor Santos
// SPDX-License-Identifier: Apache-2.0

//! UserPromptSubmit, Stop and Notification hook (ADR-0024, stage 3). It keeps
//! the small per session activity file that tells `playbook sessions` whether
//! a turn is running or the session waits for a person. It reads no prompt
//! text, prints nothing and never blocks.

use crate::common::{session_dir, Payload};

pub fn run(payload: &Payload) {
    let dir = session_dir(payload);
    if dir.is_empty() {
        return;
    }
    crate::sessions::activity::record(
        std::path::Path::new(&dir),
        &payload.field(".hook_event_name"),
        &payload.field(".notification_type"),
        crate::common::time::now_epoch_secs(),
    );
}
