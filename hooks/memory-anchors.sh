#!/usr/bin/env bash
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
# lookup after that is a plain awk scan of that file, no JSON parsing.
#
# Staleness: the index is built once, on the first Edit or Write of the
# session, and never rebuilt within that session. A fact added to the graph
# mid-session (via rebuild-memory-graph.sh) will not appear in this hook's
# output until the next session starts with a fresh cache. This is
# deliberate: facts are written rarely, this hook is advisory context only,
# and a file watcher or a per-edit freshness check would be a lot of
# machinery for a rare case. This behaviour is pinned by the stale cache
# scenario in memory-anchors.test.sh.
. "$(dirname "$0")/lib/common.sh"

dir="$(session_dir)"
[[ -z "$dir" ]] && exit 0

raw_path="$(hi_field '.tool_input.file_path')"
[[ -z "$raw_path" ]] && exit 0

command -v jq >/dev/null 2>&1 || exit 0

# Anchors in the graph are repo-relative paths. The tool gives us an
# (usually absolute) file_path, so strip the git worktree root off it.
root="$(git --no-optional-locks rev-parse --show-toplevel 2>/dev/null)"
if [[ -n "$root" && "$raw_path" == "$root"/* ]]; then
  relpath="${raw_path#"$root"/}"
else
  relpath="${raw_path#/}"
fi
[[ -z "$relpath" ]] && exit 0

idx="$dir/memory-anchor-index.tsv"

# Build the index once per session. Its mere existence, even empty, is the
# "already built" marker; see the Staleness note above for why a graph that
# changes after this point is not picked up until the next session.
if [[ ! -e "$idx" ]]; then
  graph="${HOME}/.claude/memory/graph.json"
  repo="$(git --no-optional-locks remote get-url origin 2>/dev/null \
    | sed -E 's#\.git/?$##; s#^[a-zA-Z]+://##; s#^[^@/]+@##; s#^[^/:]+[:/]##')"

  tmp="$idx.tmp.$$"
  : > "$tmp" 2>/dev/null
  if [[ -r "$graph" ]]; then
    jq -r --arg repo "$repo" '
      def in_scope: .scope == "global" or (.scope == "project" and .project == $repo);

      (.nodes | map(select(in_scope)) | map(.id)) as $ids
      | (reduce $ids[] as $i ({}; .[$i] = true)) as $inscope
      | (.nodes | map({(.id): .}) | add) as $byid
      | (
          reduce (
            .edges[]
            | select(.relation == "depends_on" or .relation == "contradicts")
            | select($inscope[.from] == true)
          ) as $e
            ({}; .[$e.from] = ((.[$e.from] // []) + [{rel: $e.relation, name: ($byid[$e.to].name // $e.to)}]))
        ) as $neigh
      | [.edges[] | select(.relation == "anchors") | select($inscope[.from] == true)]
      | map(
          ($byid[.from]) as $f
          | ($byid[.to]) as $c
          | select($f != null and $c != null and (($c.file // "") != ""))
          | [
              ($c.file | sub("#.*$"; "")),
              .from,
              ($f.name // ""),
              (($f.description // "") | gsub("\n"; " ")),
              (($neigh[.from] // []) | map("\(.rel):\(.name)") | join(", "))
            ]
          | @tsv
        )
      | .[]
    ' "$graph" >> "$tmp" 2>/dev/null
  fi
  mv "$tmp" "$idx" 2>/dev/null
fi

[[ -s "$idx" ]] || exit 0

# Match exact repo-relative path first, then any anchor that is a containing
# directory of it (an anchor of src/ matches an edit to src/deep/b.py).
matches="$(awk -F'\t' -v p="$relpath" '
  {
    anchor = $1
    dirp = anchor
    if (substr(dirp, length(dirp), 1) != "/") dirp = dirp "/"
    if (anchor == p || index(p, dirp) == 1) print
  }
' "$idx" 2>/dev/null | awk -F'\t' '!seen[$2]++')"

[[ -z "$matches" ]] && exit 0

msg="Memory facts anchored to ${relpath}:"
while IFS=$'\t' read -r _ _ name desc neigh; do
  [[ -z "$name" ]] && continue
  line="- ${name}"
  [[ -n "$desc" ]] && line="${line}: ${desc}"
  [[ -n "$neigh" ]] && line="${line} (${neigh})"
  msg="${msg}
${line}"
done <<<"$matches"

emit_pre_context "PreToolUse" "$msg"
exit 0
