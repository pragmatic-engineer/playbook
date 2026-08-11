#!/usr/bin/env python3
# SPDX-FileCopyrightText: 2026 Igor Santos
# SPDX-License-Identifier: MIT
# Stop hook: statusline.sh drops a capture-due marker in the session dir once
# context usage crosses CC_CAPTURE_AT (see statusline.sh). This hook reads
# that marker and, if present, pauses the turn with a block decision asking
# the model to write down durable facts from the session while it still can,
# then clears the marker so the prompt fires once per crossing rather than on
# every turn after it.
#
# hooks/precompact-warn.py records that PreCompact has no additionalContext
# channel, so it cannot instruct the model at all. Stop fires after every
# assistant turn and can feed text back via a decision block with a reason,
# which is why capture lives here instead.
#
# session-clean-exit.py is the other Stop hook on this event; both are
# registered independently in hooks.json and must keep working side by side.

import json
import os
import sys

sys.path.insert(0, os.path.join(os.path.dirname(os.path.abspath(__file__)), "lib"))
import common as c  # noqa: E402

MAX_PATHS = 5


def main() -> int:
    dir_ = c.session_dir()
    if not dir_:
        return 0

    marker = os.path.join(dir_, "capture-due")
    if not os.path.isfile(marker):
        return 0

    # Consume the marker before building the reason text. If anything below goes
    # wrong, a plain block with a shorter reason is far better than a marker that
    # survives and blocks every turn after this one.
    try:
        os.remove(marker)
    except OSError:
        pass

    listed = []
    more = 0
    edits = os.path.join(dir_, "edits.jsonl")
    unique = _unique_paths_recent_first(edits)
    if unique:
        total = len(unique)
        listed = unique[:MAX_PATHS]
        if total > MAX_PATHS:
            more = total - MAX_PATHS

    body = (
        "Context usage in this session just crossed the capture threshold. This "
        "is a good moment to pause, not a problem: write down anything from this "
        "session worth remembering next time, such as a decision made, a gotcha "
        "found, or a convention confirmed, using the memory tools or store this "
        "project keeps. Then continue with the rest of the turn."
    )

    if listed:
        path_lines = "\n".join("- " + p for p in listed)
        body = (
            body
            + "\n\nFiles edited this session, most recent first, worth checking "
            "for capture worthy facts:\n" + path_lines
        )
        if more > 0:
            body = body + "\n...and %d more not shown." % more

    body = (
        body
        + "\n\nThis prompt fires once per threshold crossing, so it will not "
        "interrupt the next turn unless usage climbs past the threshold again."
    )

    print(json.dumps({"decision": "block", "reason": body},
                     separators=(",", ":"), ensure_ascii=False))
    return 0


def _unique_paths_recent_first(edits: str):
    """Unique edited paths, most recently edited first. Mirrors the jq slurp:
    reverse the append log, keep the first occurrence of each path."""
    try:
        if os.path.getsize(edits) <= 0:
            return []
    except OSError:
        return []
    paths = []
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
                p = rec.get("path")
                if p is not None:
                    paths.append(p)
    except Exception:
        return []
    seen = set()
    out = []
    for p in reversed(paths):
        if p not in seen:
            seen.add(p)
            out.append(p)
    return out


if __name__ == "__main__":
    sys.exit(main())
