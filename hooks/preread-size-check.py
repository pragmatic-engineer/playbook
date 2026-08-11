#!/usr/bin/env python3
# SPDX-FileCopyrightText: 2026 Igor Santos
# SPDX-License-Identifier: MIT
# PreToolUse hook on Read: deny a full-file Read when the file is large and no
# offset/limit was provided. Pushes Claude toward Grep-first, then targeted Read.
#
# Allowlist a small set of config files commonly needed in full.

import fnmatch
import os
import sys

sys.path.insert(0, os.path.join(os.path.dirname(os.path.abspath(__file__)), "lib"))
import common as c  # noqa: E402

LINE_LIMIT = 1000
BYTE_LIMIT = 204800  # 200 KB

# Common small config / docs files that are usually needed whole.
ALLOWLIST = (
    "package.json", "tsconfig.json", "tsconfig.*.json", "pyproject.toml",
    "go.mod", "go.sum", "Cargo.toml", "Cargo.lock", "Gemfile", "Gemfile.lock",
    "requirements.txt", "CLAUDE.md", "README.md", "README", "CHANGELOG.md",
    "LICENSE", ".gitignore", ".dockerignore", "Dockerfile",
    "docker-compose.yml", "docker-compose.yaml", "Makefile", ".env.example",
    "settings.json", "settings.local.json",
)


def main() -> int:
    path = c.field(".tool_input.file_path")
    if not path:
        return 0
    if not os.path.isfile(path):
        return 0

    # Honour explicit offset/limit: caller already knows what they're doing.
    if c.field(".tool_input.offset") or c.field(".tool_input.limit"):
        return 0

    base = os.path.basename(path)
    for pat in ALLOWLIST:
        if fnmatch.fnmatchcase(base, pat):
            return 0

    # Line count = newline count (matches `wc -l`); byte size from stat.
    try:
        with open(path, "rb") as f:
            data = f.read()
        lines = data.count(b"\n")
    except Exception:
        lines = 0
    try:
        num_bytes = os.path.getsize(path)
    except Exception:
        num_bytes = 0

    # Files at or below either threshold pass.
    if lines <= LINE_LIMIT and num_bytes <= BYTE_LIMIT:
        return 0

    reason = (
        "This file is %d lines / %d bytes, too large to Read in full.\n"
        "\n"
        "Cheaper approaches:\n"
        "  1. Grep the file first to find the relevant line ranges.\n"
        "  2. Re-call Read with offset:<line> and limit:<rows> for the section you need.\n"
        "  3. If you really need the whole file (e.g. a small minified bundle), re-issue\n"
        "     with explicit offset:0, limit:9999 to override this guard.\n"
        "\n"
        "Why this matters: full Reads on large files burn input tokens that almost never\n"
        "pay back. Most callers only use 10-20%% of the content." % (lines, num_bytes)
    )
    c.emit_pre_deny(reason)
    return 0


if __name__ == "__main__":
    sys.exit(main())
