// SPDX-FileCopyrightText: 2026 Igor Santos
// SPDX-License-Identifier: Apache-2.0

//! Ports the retired shell original: a PreToolUse hook on Grep/Glob/Read that
//! tracks exploration breadth and nudges Claude toward the Explore subagent
//! once the main session fans out across many files.
//!
//! Counting rules:
//!   - Grep/Glob: each call counts 1.
//!   - Read: only the first time a unique absolute path is read this session
//!     counts. Subsequent reads of the same file don't, since those are
//!     often offset follow-ups that should be encouraged, not discouraged.
//!
//! Emits one additionalContext nudge, at count 8. Every other count stays
//! silent so it doesn't become spam: by then Claude has either delegated or
//! chosen not to.
//!
//! The first Grep or Glob of the main conversation also gets the code search
//! hint, once per session. The nudge wins when both fall on the same call.

use crate::common::payload::Payload;
use crate::common::{abspath, atomic_append, emit_pre_context, incr_counter, session_dir};
use std::fs::{self, OpenOptions};

pub fn run(payload: &Payload) {
    let dir = session_dir(payload);
    if dir.is_empty() {
        return;
    }

    let tool = payload.field(".tool_name");

    let count_file = format!("{dir}/search-count");
    let seen_file = format!("{dir}/seen-reads");
    let tool_count_file = format!("{dir}/tool-count");

    // Bump global tool counter (statusline reads this).
    incr_counter(&tool_count_file);

    let is_text_search = tool == "Grep" || tool == "Glob";
    let mut bump_search = false;
    if is_text_search {
        bump_search = true;
    } else if tool == "Read" {
        let path = payload.field(".tool_input.file_path");
        if !path.is_empty() {
            let abs_path = abspath(&path);
            if !seen(&seen_file, &abs_path) {
                // the retired shell original appends to seen_file with a plain
                // unlocked `open(..., "a")`. The bytes written here are
                // identical; only the synchronisation differs, a deliberate
                // difference rather than a divergence in behaviour.
                atomic_append(&seen_file, &abs_path);
                bump_search = true;
            }
        }
    }

    if !bump_search {
        return;
    }

    let n = incr_counter(&count_file);
    if n == NUDGE_AT {
        nudge(n);
    } else if is_text_search && payload.field(".agent_id").is_empty() {
        hint_once(&dir);
    }
}

/// The one nudge toward the Explore subagent, fired when the unique reads and
/// searches reach this count. Any other count stays silent.
const NUDGE_AT: i64 = 8;

fn nudge(n: i64) {
    emit_pre_context(
        "PreToolUse",
        &format!(
            "Search/read count is {n}. If more discovery is coming, dispatch the Explore \
             subagent (Agent tool, subagent_type: \"Explore\"): its search context stays in \
             its own window and only a digest comes back. Tell it to run `playbook code status` \
             and `playbook code context` before Grep."
        ),
    );
}

/// Marker file in the session directory; its existence means the hint went out.
const HINT_MARKER: &str = "code-hint";

const CODE_HINT: &str = "For a code question, run `playbook code status` (then `playbook code index` when it says stale or missing) and `playbook code context \"<task>\"`, `symbol`, `refs`, `search`, `outline` or `read` before more Grep or Glob. Grep and Glob are the fallback when the binary or the index is unavailable. Put the same rule in any `Explore` prompt.";

/// Emits the code search hint the first time it is called in a session.
/// `create_new` makes the marker the single winner when calls race.
fn hint_once(dir: &str) {
    let created = OpenOptions::new()
        .create_new(true)
        .write(true)
        .open(format!("{dir}/{HINT_MARKER}"));
    if created.is_ok() {
        emit_pre_context("PreToolUse", CODE_HINT);
    }
}

/// Whole-line exact match against `seen_file`, mirroring `grep -qxF`. A
/// missing or unreadable file is treated as "not seen". Never panics.
fn seen(seen_file: &str, abs_path: &str) -> bool {
    let Ok(contents) = fs::read_to_string(seen_file) else {
        return false;
    };
    contents.lines().any(|line| line == abs_path)
}
