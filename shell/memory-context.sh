#!/usr/bin/env bash
# SPDX-FileCopyrightText: 2026 Igor Santos
# SPDX-License-Identifier: MIT
#
# memory-context.sh: render a compact, repo-scoped markdown slice of the
# graph-first memory store (~/.claude/memory/memory.graph.json) for injection into
# a session's context. Three parts: the facts in scope (every global fact
# plus every fact whose project matches the repo), the typed edges among
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
#   --graph  defaults to $HOME/.claude/memory/memory.graph.json
set -u

die() { echo "memory-context: $*" >&2; exit 1; }

REPO=""
GRAPH="${HOME}/.claude/memory/memory.graph.json"

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

# Memory is optional. A missing or unreadable graph, or no jq, is not an
# error: callers must never break because memory is missing.
[[ -n "$GRAPH" && -r "$GRAPH" ]] || exit 0
command -v jq >/dev/null 2>&1 || exit 0

# The filter, in three parts matching the output:
#   1. $facts_block: name: description, one line per in-scope fact.
#   2. $edges_block: the typed edges (not anchors) whose endpoints are both
#      in scope, rendered as "name relation name".
#   3. $anchors_block: each anchored path mapped to the in-scope facts that
#      describe it, "path: name, name".
# Nodes carry id/file/scope/type/name/description(/project); edges carry
# from/to/relation. Anchors edges always run fact -> code node in the same
# project (or both global), so filtering edges on the in-scope fact ids
# keeps both endpoints in scope with no separate code-node scoping pass.
JQ_FILTER='
  def in_scope: .scope == "global" or (.scope == "project" and .project == $repo);

  (.nodes | map(select(in_scope))) as $facts
  | ($facts | map(.id)) as $ids
  | (reduce $ids[] as $i ({}; .[$i] = true)) as $inscope
  | (.nodes | map({(.id): .}) | add) as $byid
  | ($facts | sort_by(.name) | map("\(.name): \(.description)") | join("\n")) as $facts_block
  | (
      [.edges[] | select(.relation != "anchors")
                | select($inscope[.from] == true and $inscope[.to] == true)]
      | sort_by([.relation, .from, .to])
      | map("\($byid[.from].name) \(.relation) \($byid[.to].name)")
      | join("\n")
    ) as $edges_block
  | (
      [.edges[] | select(.relation == "anchors") | select($inscope[.from] == true)]
      | group_by(.to)
      | map({path: $byid[.[0].to].file, names: (map($byid[.from].name) | sort | join(", "))})
      | sort_by(.path)
      | map("\(.path): \(.names)")
      | join("\n")
    ) as $anchors_block
  | [
      (if $facts_block   != "" then "Facts:\n"   + $facts_block   else empty end),
      (if $edges_block   != "" then "Edges:\n"   + $edges_block   else empty end),
      (if $anchors_block != "" then "Anchors:\n" + $anchors_block else empty end)
    ]
  | join("\n\n")
'

output="$(jq -r --arg repo "$REPO" "$JQ_FILTER" "$GRAPH" 2>/dev/null)"
[[ $? -eq 0 && -n "$output" ]] || exit 0

printf '%s\n' "$output"
