#!/usr/bin/env python3
# SPDX-FileCopyrightText: 2026 Igor Santos
# SPDX-License-Identifier: MIT
# PreToolUse hook on Grep/Glob/Read: track exploration breadth. Nudge Claude
# toward the Explore subagent when the main session is fanning out across
# many files.
#
# Counting rules:
#   - Grep/Glob: each call = 1.
#   - Read: only the *first* time a unique absolute path is read this session
#     counts. Subsequent reads of the same file don't (they're often offset
#     follow-ups, which we want to encourage, not discourage).
#
# Emits additionalContext at thresholds 4, 8, 12. Past 12 it stays silent so
# it doesn't become spam: by then Claude has either delegated or chosen not to.

import os
import sys

sys.path.insert(0, os.path.join(os.path.dirname(os.path.abspath(__file__)), "lib"))
import common as c  # noqa: E402


def main() -> int:
    dir_ = c.session_dir()
    if not dir_:
        return 0

    tool = c.field(".tool_name")

    count_file = os.path.join(dir_, "search-count")
    seen_file = os.path.join(dir_, "seen-reads")
    tool_count_file = os.path.join(dir_, "tool-count")

    # Bump global tool counter (statusline reads this).
    c.incr_counter(tool_count_file)

    bump_search = False
    if tool in ("Grep", "Glob"):
        bump_search = True
    elif tool == "Read":
        path = c.field(".tool_input.file_path")
        if path:
            abs_path = c.abspath(path)
            if not _seen(seen_file, abs_path):
                try:
                    with open(seen_file, "a") as f:
                        f.write(abs_path + "\n")
                except Exception:
                    pass
                bump_search = True

    if not bump_search:
        return 0

    n = c.incr_counter(count_file)

    # Threshold nudges. Single, escalating message at each step.
    if n == 4:
        c.emit_pre_context(
            "PreToolUse",
            'Search/read count for this session has reached %d. If your remaining '
            'searches will fan across more than a couple more files, dispatch the '
            'Explore subagent now (Agent tool, subagent_type: "Explore"): its full '
            'search context stays in its window and only a digest comes back to '
            'yours. Keeps main context lean for the actual work.' % n,
        )
    elif n == 8:
        c.emit_pre_context(
            "PreToolUse",
            "Search/read count is now %d. You're deep in exploration, so strongly "
            "prefer dispatching the Explore subagent for the rest of this discovery "
            "work. Each additional Read here costs main-context tokens you won't "
            "recover." % n,
        )
    elif n == 12:
        c.emit_pre_context(
            "PreToolUse",
            "Search/read count is %d. Main context is now carrying significant "
            "exploration weight. Wrap up this discovery and continue in an Explore "
            "subagent, or summarize findings to yourself and consider /clear once "
            "the task is settled." % n,
        )
    return 0


def _seen(seen_file: str, abs_path: str) -> bool:
    """Whole-line exact match against seen-reads (mirrors grep -qxF)."""
    try:
        with open(seen_file) as f:
            for line in f:
                if line.rstrip("\n") == abs_path:
                    return True
    except Exception:
        return False
    return False


if __name__ == "__main__":
    sys.exit(main())
