# SPDX-FileCopyrightText: 2026 Igor Santos
# SPDX-License-Identifier: MIT
# cc module: session lookup, enumeration, and listing
#
# Shared transcript helpers used by the dispatcher and the `cc list` subcommand.
# A "session" is a UUID-named .jsonl transcript under the project dir; the
# customTitle inside it is matched against the directory name for resume.

# Find the most recent .jsonl in $project_dir whose body contains the
# customTitle for $name. Exactly matches the original _claude's lookup,
# extracted into a function so the subcommands can reuse it.
_cc_find_session_by_title() {
    local project_dir="$1" name="$2"
    [[ -d "$project_dir" ]] || return 0
    local match_file
    match_file=$(grep -rl "\"customTitle\":\"$name\"" "$project_dir"/*.jsonl(N) 2>/dev/null \
                  | xargs ls -t 2>/dev/null | head -1)
    [[ -n "$match_file" ]] && print -- "${match_file:t:r}"
}

# UUID pattern check: excludes "memory/" and other non-session dirs.
_cc_is_uuid() {
    [[ "$1" =~ ^[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}$ ]]
}

# Enumerate transcripts under $project_dir, newest first.
# Output: lines of "<mtime>\t<sid>\t<title>".
_cc_enumerate_sessions() {
    local project_dir="$1"
    [[ -d "$project_dir" ]] || return 0
    find "$project_dir" -mindepth 1 -maxdepth 1 -name '*.jsonl' -type f 2>/dev/null |
      while IFS= read -r f; do
          local sid="${f##*/}"; sid="${sid%.jsonl}"
          _cc_is_uuid "$sid" || continue
          local ts
          ts=$(stat -f %m "$f" 2>/dev/null || stat -c %Y "$f" 2>/dev/null || echo 0)
          # Pull customTitle from the transcript (cheap, usually in the first few lines).
          local title
          title=$(grep -m1 -oE '"customTitle":"[^"]*"' "$f" 2>/dev/null | head -1 \
                    | sed 's/"customTitle":"//; s/"$//')
          printf '%s\t%s\t%s\n' "$ts" "$sid" "${title:-(no title)}"
      done | sort -rnu -k1,1
}

_cc_list_sessions() {
    local project_dir="$1"
    if [[ ! -d "$project_dir" ]]; then
        print -- "no sessions for ${PWD##*/}"
        return 0
    fi
    print -- "Recent sessions for ${PWD##*/}:"
    _cc_enumerate_sessions "$project_dir" | head -10 |
      while IFS=$'\t' read -r ts sid title; do
          local when
          when=$(date -r "$ts" '+%Y-%m-%d %H:%M' 2>/dev/null \
                 || date -d "@$ts" '+%Y-%m-%d %H:%M' 2>/dev/null)
          printf '  %s  %s  %s\n' "$when" "${sid:0:8}…" "$title"
      done
}
