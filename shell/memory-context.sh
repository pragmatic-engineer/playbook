#!/usr/bin/env bash
# SPDX-FileCopyrightText: 2026 Igor Santos
# SPDX-License-Identifier: MIT
#
# memory-context.sh: render a compact, repo-scoped markdown slice of the
# graph-first memory store (~/.config/playbook/memory/memory.graph.json) for injection into
# a session's context. Three parts: the facts in scope (every global, org, or project fact matching the repo/owner), the typed edges among
# those facts (supersedes, depends_on, relates_to, contradicts; anchors
# edges are excluded here, they belong to the anchor index), and the anchor
# index mapping each anchored path to the facts that describe it.
#
# Prints nothing and exits 0 when the graph file is absent, unreadable, or
# not valid JSON, so callers never break because memory is missing.
#
# Run:  shell/memory-context.sh [--repo <owner/repo>] [--graph <path>]
#   --repo   defaults to the origin remote slug, derived the same way as
#            the hook library repo_slug (src/common/repo.rs): strip protocol,
#            user, host, and the trailing .git suffix from `git remote get-url origin`.
#   --graph  defaults to $HOME/.config/playbook/memory/memory.graph.json
set -u

die() { echo "memory-context: $*" >&2; exit 1; }

REPO=""
GRAPH="${HOME}/.config/playbook/memory/memory.graph.json"

while [[ $# -gt 0 ]]; do
  case "$1" in
    --repo)
      [[ $# -ge 2 ]] || die "--repo requires a value"
      REPO="$2"
      shift 2
      ;;
    --graph)
      [[ $# -ge 2 ]] || die "--graph requires a value"
      GRAPH="$2"
      shift 2
      ;;
    *)
      die "unknown option: $1"
      ;;
  esac
done

if [[ -z "$REPO" ]]; then
  REPO="$(git --no-optional-locks remote get-url origin 2>/dev/null \
    | sed -E 's#\.git/?$##; s#^[a-zA-Z]+://##; s#^[^@/]+@##; s#^[^/:]+[:/]##')"
fi

# Memory is optional. A missing or unreadable graph, or no playbook binary,
# is not an error: callers must never break because memory is missing.
[[ -n "$GRAPH" && -r "$GRAPH" ]] || exit 0
command -v playbook >/dev/null 2>&1 || exit 0

# Rendering itself (facts in scope, their typed edges, and the anchor index)
# lives in src/json/memorycontext.rs, ported from this script's original
# jq filter; see that module for the exact three-part shape.
output="$(playbook json memory-context "$GRAPH" "$REPO" 2>/dev/null)"
[[ $? -eq 0 && -n "$output" ]] || exit 0

printf '%s\n' "$output"
