#!/usr/bin/env python3
# SPDX-FileCopyrightText: 2026 Igor Santos
# SPDX-License-Identifier: MIT
#
# gen-shared-settings.py: derive the tracked, conservative settings.shared.json
# template from a live settings.json. Replaces .permissions with a canned
# permissions object, forces skipAutoPermissionPrompt:false, strips any pinned
# model (the harness or the user's own settings.json chooses it), drops the
# owner's personal keys, and reduces .hooks to the always-on safety guards only
# (rm-workspace-guard, bg-await-guard, no-dash-guard, precommit-check). Keep this
# list in step with SAFETY_RE below: a guard missing from it is silently dropped
# from the seed on the next regeneration. The functional hooks ship
# with the plugin instead, so the seed wires only the guards to avoid
# double-firing. Other product config (env, statusLine, worktree, plugins, ...)
# passes through unchanged. Merged JSON goes to stdout.
#
# Usage: gen-shared-settings.py SRC [PERMS]
#   SRC    path to the live settings.json (required)
#   PERMS  path to the canned permissions object
#          (default: <repo>/permissions.shared.json)
#
# Exit: 0 on success (merged JSON on stdout); non-zero on any guard failure
#       (diagnostic on stderr, nothing on stdout).

import json
import re
import sys
from pathlib import Path

SCRIPT_DIR = Path(__file__).resolve().parent
REPO_ROOT = SCRIPT_DIR.parent

PERSONAL_KEYS = frozenset(
    {"model", "effortLevel", "theme", "preferredNotifChannel", "prefersReducedMotion"}
)
SAFETY_RE = re.compile(
    r"(rm-workspace-guard|bg-await-guard|no-dash-guard|precommit-check)\.sh"
)


def die(msg: str, code: int = 1) -> None:
    print(f"gen-shared-settings: {msg}", file=sys.stderr)
    sys.exit(code)


def load_json(path: Path, label: str):
    try:
        text = path.read_text(encoding="utf-8")
    except OSError:
        die(f"{label} not readable: {path}", 2)
    try:
        return json.loads(text)
    except json.JSONDecodeError:
        die(f"{label} is not valid JSON: {path}", 2)


def filter_hooks(hooks: dict) -> dict:
    result = {}
    for event, groups in hooks.items():
        new_groups = []
        for group in groups:
            safe = [
                h for h in group.get("hooks", [])
                if SAFETY_RE.search(h.get("command", ""))
            ]
            if safe:
                new_group = dict(group)
                new_group["hooks"] = safe
                new_groups.append(new_group)
        if new_groups:
            result[event] = new_groups
    return result


def main() -> None:
    if len(sys.argv) < 2:
        die("usage: gen-shared-settings.py SRC [PERMS]", 2)

    src_path = Path(sys.argv[1])
    perms_path = (
        Path(sys.argv[2]) if len(sys.argv) >= 3 else REPO_ROOT / "permissions.shared.json"
    )

    src = load_json(src_path, "source settings")
    perms = load_json(perms_path, "permissions file")

    if (
        not isinstance(perms, dict)
        or not isinstance(perms.get("allow"), list)
        or len(perms["allow"]) == 0
    ):
        die(
            f"permissions file must be an object with a non-empty allow array: {perms_path}",
            2,
        )

    result = dict(src)
    result["permissions"] = perms
    result["skipAutoPermissionPrompt"] = False
    for key in PERSONAL_KEYS:
        result.pop(key, None)

    if "hooks" in result:
        result["hooks"] = filter_hooks(result["hooks"])

    print(json.dumps(result, indent=2))


if __name__ == "__main__":
    main()
