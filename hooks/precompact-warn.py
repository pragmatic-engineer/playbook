#!/usr/bin/env python3
# SPDX-FileCopyrightText: 2026 Igor Santos
# SPDX-License-Identifier: MIT
# PreCompact hook: fires when Claude Code is about to auto-compact the
# conversation. By the time this fires, the cheap-cache window is gone and the
# next turn will reload a lossy summary. Strong signal to wrap up + restart.
#
# Emits only a user-facing systemMessage. PreCompact has no additionalContext
# channel (the hook output schema defines no PreCompact variant), so the hook
# can't inject guidance to Claude here; the systemMessage prompts the user.

import os
import sys
import time

sys.path.insert(0, os.path.join(os.path.dirname(os.path.abspath(__file__)), "lib"))
import common as c  # noqa: E402


def main() -> int:
    trigger = c.field(".trigger")
    sid = c.session_id()
    ts = time.strftime("%Y-%m-%d %H:%M:%S")

    # Log to a flat file for later review. Cap at 500 lines to prevent unbounded growth.
    log = os.path.join(c.RUNTIME_ROOT, "compactions.log")
    try:
        os.makedirs(c.RUNTIME_ROOT, exist_ok=True)
        with open(log, "a") as f:
            f.write("%s\tsession=%s\ttrigger=%s\n" % (ts, sid, trigger or "unknown"))
        _cap_lines(log, 500)
    except Exception:
        pass

    user_msg = (
        "⚠ Context compaction triggered (%s). After this point, every turn "
        "replays a lossy summary instead of the original transcript, so the cache "
        "savings are gone. Strongly consider: finish the current step, ask me to "
        "wrap up (a session handoff), then /clear for a fresh session."
        % (trigger or "auto")
    )

    # PreCompact output supports only top-level fields, so emit just systemMessage.
    c.emit_system_message(user_msg)
    return 0


def _cap_lines(path: str, limit: int) -> None:
    try:
        with open(path) as f:
            lines = f.readlines()
        if len(lines) > limit:
            tmp = path + ".tmp." + str(os.getpid())
            with open(tmp, "w") as f:
                f.writelines(lines[-limit:])
            os.replace(tmp, path)
    except Exception:
        pass


if __name__ == "__main__":
    sys.exit(main())
