// SPDX-FileCopyrightText: 2026 Igor Santos
// SPDX-License-Identifier: MIT

//! Ports hooks/search-counter.py: a PreToolUse hook on Grep/Glob/Read that
//! tracks exploration breadth and nudges Claude toward the Explore subagent
//! once the main session fans out across many files.
//!
//! Counting rules:
//!   - Grep/Glob: each call counts 1.
//!   - Read: only the first time a unique absolute path is read this session
//!     counts. Subsequent reads of the same file don't, since those are
//!     often offset follow-ups that should be encouraged, not discouraged.
//!
//! Emits additionalContext at thresholds 4, 8 and 12. Past 12 it stays
//! silent so it doesn't become spam: by then Claude has either delegated or
//! chosen not to.

use crate::common::payload::Payload;
use crate::common::{abspath, atomic_append, emit_pre_context, incr_counter, session_dir};
use std::fs;

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

    let mut bump_search = false;
    if tool == "Grep" || tool == "Glob" {
        bump_search = true;
    } else if tool == "Read" {
        let path = payload.field(".tool_input.file_path");
        if !path.is_empty() {
            let abs_path = abspath(&path);
            if !seen(&seen_file, &abs_path) {
                // hooks/search-counter.py appends to seen_file with a plain
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
    nudge(n);
}

/// Escalating nudge toward the Explore subagent at counts 4, 8 and 12. Any
/// other count, including everything past 12, stays silent so the nudge
/// never becomes spam.
fn nudge(n: i64) {
    match n {
        4 => emit_pre_context(
            "PreToolUse",
            &format!(
                "Search/read count for this session has reached {n}. If your remaining \
                 searches will fan across more than a couple more files, dispatch the \
                 Explore subagent now (Agent tool, subagent_type: \"Explore\"): its full \
                 search context stays in its window and only a digest comes back to \
                 yours. Keeps main context lean for the actual work."
            ),
        ),
        8 => emit_pre_context(
            "PreToolUse",
            &format!(
                "Search/read count is now {n}. You're deep in exploration, so strongly \
                 prefer dispatching the Explore subagent for the rest of this discovery \
                 work. Each additional Read here costs main-context tokens you won't \
                 recover."
            ),
        ),
        12 => emit_pre_context(
            "PreToolUse",
            &format!(
                "Search/read count is {n}. Main context is now carrying significant \
                 exploration weight. Wrap up this discovery and continue in an Explore \
                 subagent, or summarize findings to yourself and consider /clear once \
                 the task is settled."
            ),
        ),
        _ => {}
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
