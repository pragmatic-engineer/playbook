#!/usr/bin/env python3
# SPDX-FileCopyrightText: 2026 Igor Santos
# SPDX-License-Identifier: MIT
# Shared helpers for Claude Code hooks.
# Import from each hook script:
#   sys.path.insert(0, <hooks>/lib); import common as c; c.field(".tool_name")
#
# Design rules:
#   - Hooks must never break Claude Code. Any error returns empty/neutral value.
#   - Stdout from an emit_* is a single valid JSON object followed by a newline.
#   - All file writes are atomic (tmp + rename) so concurrent hooks do not tear state.
#   - Per-session state lives in ~/.claude/runtime/<session_id>/.

import json
import os
import re
import subprocess
import sys
import tempfile
import time

RUNTIME_ROOT = os.path.join(os.path.expanduser("~"), ".claude", "runtime")

# Read the entire JSON payload from stdin ONCE at module import time.
# Honour HOOK_INPUT env var if set and non-empty; else read all of stdin
# when stdin is not a tty; else treat as empty.
# This mirrors common.sh lines 18-26: HOOK_INPUT is inherited by subshells.
_raw: str = os.environ.get("HOOK_INPUT", "")
if not _raw and not sys.stdin.isatty():
    try:
        _raw = sys.stdin.read()
    except Exception:
        _raw = ""

try:
    _payload: dict = json.loads(_raw) if _raw else {}
    if not isinstance(_payload, dict):
        _payload = {}
except Exception:
    _payload = {}


def _traverse(keys: list[str]) -> object:
    """Traverse the cached payload by a list of string keys."""
    cur: object = _payload
    for k in keys:
        if isinstance(cur, dict) and k in cur:
            cur = cur[k]
        else:
            return None
    return cur


def field(path: str) -> str:
    """
    Extract a JSON field by jq-style dotted path (e.g. '.tool_input.file_path').
    Returns '' for missing/null. Strings as-is; bools as 'true'/'false';
    numbers as text; objects/arrays as compact JSON. Never raises.
    """
    try:
        stripped = path.lstrip(".")
        if not stripped:
            return ""
        keys = stripped.split(".")
        value = _traverse(keys)
        if value is None:
            return ""
        if isinstance(value, bool):
            return "true" if value else "false"
        if isinstance(value, str):
            return value
        if isinstance(value, (int, float)):
            if isinstance(value, float) and value == int(value):
                return str(int(value))
            return str(value)
        return json.dumps(value, separators=(",", ":"))
    except Exception:
        return ""


def session_id() -> str:
    """Return the session_id from the hook input, or '' if absent."""
    return field(".session_id")


def session_dir() -> str:
    """
    Return the per-session state directory, creating it on demand.
    Returns '' if no session id is present.
    Directory mode follows umask 077 (mode 700).
    """
    try:
        sid = session_id()
        if not sid:
            return ""
        d = os.path.join(RUNTIME_ROOT, sid)
        if not os.path.isdir(d):
            old_umask = os.umask(0o077)
            try:
                os.makedirs(d, exist_ok=True)
            except Exception:
                pass
            finally:
                os.umask(old_umask)
        return d
    except Exception:
        return ""


def abspath(p: str) -> str:
    """
    Resolve a path to absolute. For a directory, returns realpath.
    For a non-directory, resolves the parent dir's realpath and re-appends
    the basename so a leaf symlink remains unresolved (matches common.sh
    lines 52-70: we key on the path the tool referenced).
    Tolerates non-existent paths. Returns '' for empty input.
    """
    if not p:
        return ""
    try:
        if os.path.isdir(p):
            return os.path.realpath(p)
        d = os.path.dirname(p) or "."
        b = os.path.basename(p)
        if os.path.isdir(d):
            return os.path.join(os.path.realpath(d), b)
        return p
    except Exception:
        return p


def atomic_append(file: str, line: str) -> None:
    """
    Append line + newline to file atomically, creating parent dirs as needed.
    Uses fcntl.flock when available to serialize concurrent writers.
    Never raises.
    """
    try:
        d = os.path.dirname(file)
        if d:
            os.makedirs(d, exist_ok=True)
        try:
            import fcntl
            lock_path = file + ".lock"
            with open(lock_path, "a") as lf:
                fcntl.flock(lf, fcntl.LOCK_EX)
                try:
                    with open(file, "a") as f:
                        f.write(line + "\n")
                finally:
                    fcntl.flock(lf, fcntl.LOCK_UN)
        except ImportError:
            with open(file, "a") as f:
                f.write(line + "\n")
    except Exception:
        pass


def emit_pre_context(event: str, msg: str) -> None:
    """Print a PreToolUse additionalContext JSON object to stdout."""
    print(json.dumps(
        {"hookSpecificOutput": {"hookEventName": event, "additionalContext": msg}},
        separators=(",", ":"),
    ))


def emit_pre_deny(reason: str) -> None:
    """Print a PreToolUse deny decision JSON object to stdout."""
    print(json.dumps(
        {"hookSpecificOutput": {
            "hookEventName": "PreToolUse",
            "permissionDecision": "deny",
            "permissionDecisionReason": reason,
        }},
        separators=(",", ":"),
    ))


def emit_prompt_context(msg: str) -> None:
    """Print a UserPromptSubmit additionalContext JSON object to stdout."""
    print(json.dumps(
        {"hookSpecificOutput": {"hookEventName": "UserPromptSubmit", "additionalContext": msg}},
        separators=(",", ":"),
    ))


def emit_system_message(msg: str) -> None:
    """Print a systemMessage JSON object to stdout."""
    print(json.dumps({"systemMessage": msg}, separators=(",", ":")))


def incr_counter(file: str) -> int:
    """
    Atomically increment the integer stored in file.
    Uses a .lock directory for mutual exclusion, with up to 50 retries (10ms apart).
    Returns the new integer value. Never raises.
    """
    try:
        lock = file + ".lock"
        i = 0
        while True:
            try:
                os.mkdir(lock)
                break
            except FileExistsError:
                i += 1
                if i >= 50:
                    break
                time.sleep(0.01)
        n = 0
        try:
            try:
                with open(file) as f:
                    n = int(f.read().strip() or "0")
            except Exception:
                n = 0
            n += 1
            parent = os.path.dirname(file) or "."
            tmp_fd, tmp_path = tempfile.mkstemp(dir=parent)
            try:
                os.write(tmp_fd, str(n).encode())
                os.close(tmp_fd)
                os.replace(tmp_path, file)
            except Exception:
                try:
                    os.close(tmp_fd)
                except Exception:
                    pass
                try:
                    os.unlink(tmp_path)
                except Exception:
                    pass
        finally:
            try:
                os.rmdir(lock)
            except Exception:
                pass
        return n
    except Exception:
        return 0


def repo_slug() -> str:
    """
    Return the <owner>/<repo> slug for the current git repo's origin remote.
    Returns '' outside a repo or when no origin is configured.
    Applies the same normalisation as common.sh line 148.
    """
    try:
        result = subprocess.run(
            ["git", "--no-optional-locks", "remote", "get-url", "origin"],
            capture_output=True, text=True, timeout=5,
        )
        if result.returncode != 0:
            return ""
        url = result.stdout.strip()
        url = re.sub(r"\.git/?$", "", url)
        url = re.sub(r"^[a-zA-Z]+://", "", url)
        url = re.sub(r"^[^@/]+@", "", url)
        url = re.sub(r"^[^/:]+[:/]", "", url)
        return url
    except Exception:
        return ""
