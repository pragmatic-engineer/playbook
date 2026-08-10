#!/usr/bin/env bash
# SPDX-FileCopyrightText: 2026 Igor Santos
# SPDX-License-Identifier: MIT
# Stop hook: statusline.sh drops a capture-due marker in the session dir once
# context usage crosses CC_CAPTURE_AT (see statusline.sh). This hook reads
# that marker and, if present, pauses the turn with a block decision asking
# the model to write down durable facts from the session while it still can,
# then clears the marker so the prompt fires once per crossing rather than on
# every turn after it.
#
# hooks/precompact-warn.sh lines 8 to 10 record that PreCompact has no
# additionalContext channel, so it cannot instruct the model at all. Stop
# fires after every assistant turn and can feed text back via decision block
# with a reason, which is why capture lives here instead.
#
# session-clean-exit.sh is the other Stop hook on this event; both are
# registered independently in hooks.json and must keep working side by side.
. "$(dirname "$0")/lib/common.sh"

dir="$(session_dir)"
[[ -z "$dir" ]] && exit 0

marker="$dir/capture-due"
[[ -f "$marker" ]] || exit 0

# Consume the marker before building the reason text. If anything below goes
# wrong, a plain block with a shorter reason is far better than a marker that
# survives and blocks every turn after this one.
rm -f "$marker" 2>/dev/null

max_paths=5
listed=""
more=0
edits="$dir/edits.jsonl"
if [[ -s "$edits" ]]; then
  # Unique edited paths, most recently edited first: reverse the append log,
  # then keep the first (i.e. most recent) occurrence of each path.
  all_unique="$(jq -s -r '
    [ .[] | .path ] | reverse
    | reduce .[] as $p ([]; if index($p) then . else . + [$p] end)
    | .[]
  ' "$edits" 2>/dev/null)"
  if [[ -n "$all_unique" ]]; then
    total=$(printf '%s\n' "$all_unique" | grep -c .)
    listed="$(printf '%s\n' "$all_unique" | head -n "$max_paths")"
    if [[ "$total" -gt "$max_paths" ]]; then
      more=$((total - max_paths))
    fi
  fi
fi

body="Context usage in this session just crossed the capture threshold. This is a good moment to pause, not a problem: write down anything from this session worth remembering next time, such as a decision made, a gotcha found, or a convention confirmed, using the memory tools or store this project keeps. Then continue with the rest of the turn."

if [[ -n "$listed" ]]; then
  path_lines="$(printf '%s\n' "$listed" | sed 's/^/- /')"
  body="${body}

Files edited this session, most recent first, worth checking for capture worthy facts:
${path_lines}"
  if [[ "$more" -gt 0 ]]; then
    body="${body}
...and ${more} more not shown."
  fi
fi

body="${body}

This prompt fires once per threshold crossing, so it will not interrupt the next turn unless usage climbs past the threshold again."

jq -cn --arg r "$body" '{decision:"block", reason:$r}'
exit 0
