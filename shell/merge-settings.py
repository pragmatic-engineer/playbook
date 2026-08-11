#!/usr/bin/env python3
# SPDX-FileCopyrightText: 2026 Igor Santos
# SPDX-License-Identifier: MIT
#
# merge-settings.py BASE TEMPLATE USER NEWBASE_OUT [SKIP_OUT]
#
# 3-way merge for Claude Code settings.json. Merged JSON -> stdout.
# Refreshed baseline -> NEWBASE_OUT path.
#
# Merge policy (per top-level key, over UNION of template+user keys):
#   user lacks key         -> template value
#   user[k] == base[k]    -> template value (update applied; dropped if template dropped it)
#   user[k] != base[k]    -> user value (customization preserved)
#   BASE absent/invalid   -> treat as {} (additive fallback) + warn stderr
#
# NEWBASE_OUT partial base refresh (C2 fix):
#   contested (user != base) -> freeze OLD base value as sentinel (NEVER template value)
#   otherwise               -> template value
#
# Validation:
#   N2: TEMPLATE and USER must be JSON objects; non-object or parse error -> exit 1, no stdout
#   N4: BASE absent or invalid -> {} + stderr warning (never hard-fail on bad base)
#   N3: SKIP_OUT (optional 5th arg): JSON array of {key,template_had,yours} for each
#       contested key where the template had a different value than the user.
#       Writes [] when zero keys are withheld. When SKIP_OUT is omitted the
#       skip info is discarded.
import json
import os
import sys
import tempfile


def warn(msg):
    print("warning: " + msg, file=sys.stderr)


def die(msg):
    print("error: " + msg, file=sys.stderr)
    sys.exit(1)


def load_object(path, soft=False, label=""):
    """Load a JSON object from path.

    When soft=True, return an empty dict with a warning on any failure.
    When soft=False, call die() on any failure (no stdout).
    """
    if not os.path.isfile(path):
        if soft:
            warn(label + " not found; treating as {}: " + path)
            return {}
        die(label + " not found: " + path)
    try:
        with open(path, "r", encoding="utf-8") as fh:
            data = json.load(fh)
    except (json.JSONDecodeError, OSError):
        if soft:
            warn(label + " is not a valid JSON object; treating as {}: " + path)
            return {}
        die(label + " is not a JSON object: " + path)
    if not isinstance(data, dict):
        if soft:
            warn(label + " is not a valid JSON object; treating as {}: " + path)
            return {}
        die(label + " is not a JSON object: " + path)
    return data


def three_way_merge(base, template, user):
    """Compute merged, newbase, and skipped from the 3-way merge.

    Keys are the sorted union of template and user keys, matching jq's
    unique() which sorts its input.
    """
    keys = sorted(set(list(template.keys()) + list(user.keys())))

    merged = {}
    for k in keys:
        if k not in user:
            if k in template:
                merged[k] = template[k]
        elif user[k] == base.get(k):
            if k in template:
                merged[k] = template[k]
        else:
            merged[k] = user[k]

    newbase = {}
    for k in keys:
        if k not in user:
            if k in template:
                newbase[k] = template[k]
        elif user[k] != base.get(k):
            if k in base:
                newbase[k] = base[k]
        else:
            if k in template:
                newbase[k] = template[k]

    skipped = []
    for k in keys:
        if (
            k in user
            and user[k] != base.get(k)
            and k in template
            and template[k] != user[k]
        ):
            skipped.append({"key": k, "template_had": template[k], "yours": user[k]})

    return merged, newbase, skipped


def atomic_write(path, content):
    """Write content to path atomically via a sibling temp file."""
    dirpath = os.path.dirname(os.path.abspath(path))
    os.makedirs(dirpath, exist_ok=True)
    fd, tmp_path = tempfile.mkstemp(dir=dirpath)
    try:
        with os.fdopen(fd, "w", encoding="utf-8") as fh:
            fh.write(content)
        os.replace(tmp_path, path)
    except Exception:
        try:
            os.unlink(tmp_path)
        except OSError:
            pass
        raise


def main():
    if len(sys.argv) < 5:
        die("usage: merge-settings.py BASE TEMPLATE USER NEWBASE_OUT [SKIP_OUT]")

    base_path = sys.argv[1]
    template_path = sys.argv[2]
    user_path = sys.argv[3]
    newbase_out = sys.argv[4]
    skip_out = sys.argv[5] if len(sys.argv) >= 6 else ""

    # N2: Validate TEMPLATE (hard fail on non-object or parse error)
    template = load_object(template_path, soft=False, label="TEMPLATE")

    # N2: Validate USER (hard fail on non-object or parse error)
    user = load_object(user_path, soft=False, label="USER")

    # N4: Load BASE (soft fail -> {} with stderr warning)
    base = load_object(base_path, soft=True, label="BASE")

    merged, newbase, skipped = three_way_merge(base, template, user)

    atomic_write(newbase_out, json.dumps(newbase, indent=2) + "\n")

    if skip_out:
        atomic_write(skip_out, json.dumps(skipped, indent=2) + "\n")

    print(json.dumps(merged, indent=2))


if __name__ == "__main__":
    main()
