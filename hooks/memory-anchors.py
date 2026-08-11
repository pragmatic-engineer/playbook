#!/usr/bin/env python3
# SPDX-FileCopyrightText: 2026 Igor Santos
# SPDX-License-Identifier: MIT
# PreToolUse hook on Edit|Write: when the target path is anchored in the
# graph-first memory store (~/.claude/memory/graph.json), surface the facts
# that describe it, plus their depends_on and contradicts neighbours, as
# additionalContext before the edit lands. Emits nothing when there is no
# match. Never blocks.
#
# Performance: this hook fires on every single Edit and Write, so it must
# not parse the 200+ KB graph.json on every call: a 50 file refactor would
# otherwise pay 50 full graph parses. The anchor index is built once per
# session into a flat, tab separated file under the session dir, and every
# lookup after that is a plain scan of that file, no JSON parsing.
#
# Staleness: the index is built once, on the first Edit or Write of the
# session, and never rebuilt within that session. A fact added to the graph
# mid-session (via rebuild-memory-graph.py) will not appear in this hook's
# output until the next session starts with a fresh cache. This is
# deliberate: facts are written rarely, this hook is advisory context only,
# and a file watcher or a per-edit freshness check would be a lot of
# machinery for a rare case. This behaviour is pinned by the stale cache
# scenario in memory-anchors.test.sh.

import json
import os
import re
import sys

sys.path.insert(0, os.path.join(os.path.dirname(os.path.abspath(__file__)), "lib"))
import common as c  # noqa: E402


def main() -> int:
    dir_ = c.session_dir()
    if not dir_:
        return 0

    raw_path = c.field(".tool_input.file_path")
    if not raw_path:
        return 0

    # Anchors in the graph are repo-relative paths. The tool gives us an
    # (usually absolute) file_path, so strip the git worktree root off it.
    root = _git_toplevel()
    if root and raw_path.startswith(root + "/"):
        relpath = raw_path[len(root) + 1:]
    else:
        relpath = raw_path[1:] if raw_path.startswith("/") else raw_path
    if not relpath:
        return 0

    idx = os.path.join(dir_, "memory-anchor-index.tsv")

    # Build the index once per session. Its mere existence, even empty, is the
    # "already built" marker; see the Staleness note above.
    if not os.path.exists(idx):
        _build_index(idx)

    try:
        if os.path.getsize(idx) <= 0:
            return 0
    except OSError:
        return 0

    # Match exact repo-relative path first, then any anchor that is a containing
    # directory of it (an anchor of src/ matches an edit to src/deep/b.py).
    matches = []
    seen_from = set()
    try:
        with open(idx) as f:
            for line in f:
                line = line.rstrip("\n")
                if not line:
                    continue
                cols = line.split("\t")
                anchor = cols[0]
                dirp = anchor if anchor.endswith("/") else anchor + "/"
                if anchor == relpath or relpath.startswith(dirp):
                    from_id = cols[1] if len(cols) > 1 else ""
                    if from_id in seen_from:
                        continue
                    seen_from.add(from_id)
                    matches.append(cols)
    except Exception:
        return 0

    if not matches:
        return 0

    msg = "Memory facts anchored to %s:" % relpath
    for cols in matches:
        name = cols[2] if len(cols) > 2 else ""
        desc = cols[3] if len(cols) > 3 else ""
        neigh = cols[4] if len(cols) > 4 else ""
        if not name:
            continue
        line = "- " + name
        if desc:
            line += ": " + desc
        if neigh:
            line += " (" + neigh + ")"
        msg += "\n" + line

    c.emit_pre_context("PreToolUse", msg)
    return 0


def _git_toplevel() -> str:
    import subprocess
    try:
        r = subprocess.run(
            ["git", "--no-optional-locks", "rev-parse", "--show-toplevel"],
            capture_output=True, text=True, timeout=5,
        )
        return r.stdout.strip() if r.returncode == 0 else ""
    except Exception:
        return ""


def _build_index(idx: str) -> None:
    """Build the tab separated anchor index from graph.json for the current
    repo scope. Mirrors the jq query in the bash original: one row per
    in-scope anchors edge, columns anchor, from_id, name, description,
    neighbours."""
    graph_path = os.path.join(os.path.expanduser("~"), ".claude", "memory", "graph.json")
    repo = c.repo_slug()
    tmp = "%s.tmp.%d" % (idx, os.getpid())
    rows = []
    try:
        with open(graph_path, encoding="utf-8") as f:
            graph = json.load(f)
        nodes = graph.get("nodes", []) or []
        edges = graph.get("edges", []) or []
        byid = {n.get("id"): n for n in nodes if isinstance(n, dict)}

        def in_scope(n):
            s = n.get("scope")
            return s == "global" or (s == "project" and n.get("project") == repo)

        inscope = {n.get("id") for n in nodes if isinstance(n, dict) and in_scope(n)}

        # depends_on / contradicts neighbours, keyed by source node id.
        neigh = {}
        for e in edges:
            if not isinstance(e, dict):
                continue
            if e.get("relation") in ("depends_on", "contradicts") and e.get("from") in inscope:
                tgt = byid.get(e.get("to")) or {}
                name = tgt.get("name") or e.get("to")
                neigh.setdefault(e.get("from"), []).append((e.get("relation"), name))

        for e in edges:
            if not isinstance(e, dict) or e.get("relation") != "anchors":
                continue
            if e.get("from") not in inscope:
                continue
            f_node = byid.get(e.get("from"))
            c_node = byid.get(e.get("to"))
            if f_node is None or c_node is None:
                continue
            cfile = c_node.get("file") or ""
            if cfile == "":
                continue
            anchor = re.sub(r"#.*$", "", cfile)
            name = f_node.get("name") or ""
            desc = (f_node.get("description") or "").replace("\n", " ")
            nb = ", ".join("%s:%s" % (rel, nm) for rel, nm in neigh.get(e.get("from"), []))
            rows.append("\t".join([anchor, str(e.get("from")), name, desc, nb]))
    except Exception:
        rows = []

    try:
        with open(tmp, "w", encoding="utf-8") as out:
            for r in rows:
                out.write(r + "\n")
        os.replace(tmp, idx)
    except Exception:
        try:
            os.unlink(tmp)
        except OSError:
            pass


if __name__ == "__main__":
    sys.exit(main())
