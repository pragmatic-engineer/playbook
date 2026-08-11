#!/usr/bin/env python3
# SPDX-FileCopyrightText: 2026 Igor Santos
# SPDX-License-Identifier: MIT
# PostToolUse hook on Edit/Write/NotebookEdit: record edited absolute path + ts
# to per-session edits.jsonl. Consumed by preread-edit-check.py + statusline.

import json
import os
import sys
import time

sys.path.insert(0, os.path.join(os.path.dirname(os.path.abspath(__file__)), "lib"))
import common as c  # noqa: E402


def main() -> int:
    dir_ = c.session_dir()
    if not dir_:
        return 0

    tool = c.field(".tool_name")
    if tool not in ("Edit", "Write", "NotebookEdit"):
        return 0

    # Different tools use different field names; try both common ones.
    path = c.field(".tool_input.file_path")
    if not path:
        path = c.field(".tool_input.notebook_path")
    if not path:
        return 0

    abs_path = c.abspath(path)
    ts = int(time.time())

    line = json.dumps({"path": abs_path, "ts": ts}, separators=(",", ":"), ensure_ascii=False)
    c.atomic_append(os.path.join(dir_, "edits.jsonl"), line)

    # Bump human-readable edit count (used by statusline).
    c.incr_counter(os.path.join(dir_, "edit-count"))
    return 0


if __name__ == "__main__":
    sys.exit(main())
