#!/usr/bin/env python3
# SPDX-FileCopyrightText: 2026 Igor Santos
# SPDX-License-Identifier: MIT
# Stop / SessionEnd hook: mark this session as having ended cleanly.
# Used by session-init.py on the NEXT session start to detect crashes
# (orphaned sessions with no clean-exit marker).

import json
import os
import re
import subprocess
import sys
import time

sys.path.insert(0, os.path.join(os.path.dirname(os.path.abspath(__file__)), "lib"))
import common as c  # noqa: E402


def main() -> int:
    dir_ = c.session_dir()
    if not dir_:
        return 0

    # Stop fires after every assistant turn (not session-end). We still want to
    # refresh the timestamp on every turn so a stale crash detection only fires
    # if the session is genuinely abandoned, not just paused.
    try:
        with open(os.path.join(dir_, "last-clean-ts"), "w") as f:
            f.write(str(int(time.time())))
    except Exception:
        pass

    # SessionEnd is the real "this session is done" signal. The reason field
    # distinguishes graceful (clear/resume/logout/prompt_input_exit) from crash.
    reason = c.field(".reason")
    if not reason or reason == "other":
        return 0

    # SessionEnd is side effects only: its stdout goes to the debug log and cannot
    # inject context, so emitting hookSpecificOutput.additionalContext here fails
    # output validation. Memory persistence is the model's job during the session
    # (system prompt Memory section); the auto-learn queue below nudges the next.
    try:
        with open(os.path.join(dir_, "clean-exit"), "w") as f:
            f.write(reason + "\n")
    except Exception:
        pass

    # Auto-learn queue: if this session did substantive work in a repo, drop a
    # per-repo flag so the next session there nudges a /learn-project run. This
    # writes a flag file only; nothing is written to memory here. Disable with
    # AUTO_LEARN_NUDGE=0; tune the threshold with AUTO_LEARN_MIN_EDITS (default 5).
    if os.environ.get("AUTO_LEARN_NUDGE", "1") == "0":
        return 0

    root = _git_toplevel()
    if not root:
        return 0

    edits = _read_int(os.path.join(dir_, "edit-count"))
    try:
        threshold = int(os.environ.get("AUTO_LEARN_MIN_EDITS", "5"))
    except ValueError:
        threshold = 5
    if edits < threshold:
        return 0

    qdir = os.path.join(c.RUNTIME_ROOT, "to-learn")
    try:
        os.makedirs(qdir, exist_ok=True)
        slug = re.sub(r"[^A-Za-z0-9_.-]", "_", root)
        payload = {
            "repo_root": root,
            "edits": edits,
            "session_id": c.session_id(),
            "ts": int(time.time()),
        }
        dest = os.path.join(qdir, slug + ".json")
        tmp = dest + ".tmp"
        with open(tmp, "w") as f:
            f.write(json.dumps(payload, separators=(",", ":")))
        os.replace(tmp, dest)
    except Exception:
        pass
    return 0


def _git_toplevel() -> str:
    try:
        r = subprocess.run(
            ["git", "--no-optional-locks", "rev-parse", "--show-toplevel"],
            capture_output=True, text=True, timeout=5,
        )
        return r.stdout.strip() if r.returncode == 0 else ""
    except Exception:
        return ""


def _read_int(path: str) -> int:
    try:
        with open(path) as f:
            digits = re.sub(r"[^0-9]", "", f.read())
        return int(digits) if digits else 0
    except Exception:
        return 0


if __name__ == "__main__":
    sys.exit(main())
