#!/usr/bin/env python3
# SPDX-FileCopyrightText: 2026 Igor Santos
# SPDX-License-Identifier: MIT
# PreToolUse hook on Read: if the target was edited by this session within the
# last N minutes, inject a system reminder so Claude doesn't waste tokens
# re-reading content already in context.
#
# Emits additionalContext (info-only). Never blocks.

import json
import os
import sys
import time

sys.path.insert(0, os.path.join(os.path.dirname(os.path.abspath(__file__)), "lib"))
import common as c  # noqa: E402

WINDOW = 1800  # 30 minutes


def main() -> int:
    dir_ = c.session_dir()
    if not dir_:
        return 0

    edits = os.path.join(dir_, "edits.jsonl")
    try:
        if os.path.getsize(edits) <= 0:
            return 0
    except OSError:
        return 0

    path = c.field(".tool_input.file_path")
    if not path:
        return 0

    abs_path = c.abspath(path)
    now = int(time.time())

    # Find the most recent edit of this exact path within the window.
    match_ts = None
    try:
        with open(edits) as f:
            for raw in f:
                raw = raw.strip()
                if not raw:
                    continue
                try:
                    rec = json.loads(raw)
                except Exception:
                    continue
                if rec.get("path") == abs_path and (now - rec.get("ts", 0)) < WINDOW:
                    match_ts = rec.get("ts")
    except Exception:
        return 0

    if match_ts is None:
        return 0

    delta = now - match_ts
    if delta < 60:
        ago = "%ds ago" % delta
    elif delta < 3600:
        ago = "%dm ago" % (delta // 60)
    else:
        ago = "%dh ago" % (delta // 3600)

    msg = (
        "You edited this file %s via Edit/Write. Your context already reflects the "
        "post-edit state. Re-reading it now is wasted tokens unless you suspect "
        "external modifications. Skip the Read and proceed." % ago
    )
    c.emit_pre_context("PreToolUse", msg)
    return 0


if __name__ == "__main__":
    sys.exit(main())
