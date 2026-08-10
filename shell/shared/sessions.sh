# SPDX-FileCopyrightText: 2026 Igor Santos
# SPDX-License-Identifier: MIT
# shellcheck shell=bash
# (sourced, no shebang; the directive tells shellcheck the dialect)
#
# sessions.sh: session lookup, enumeration, and listing.
#
# A "session" is a UUID-named .jsonl transcript under the project dir; the
# customTitle inside it is matched against the directory name for resume.
# Sourceable in bash and zsh. No zsh-only builtins or glob qualifiers.

# Find the most recent .jsonl in $project_dir whose body contains the
# customTitle for $name. Prints the session UUID (no extension), or nothing.
_cc_find_session_by_title() {
    local project_dir="$1" name="$2"
    [ -d "$project_dir" ] || return 0
    local -a jsonl_files=()
    local f
    while IFS= read -r f; do
        jsonl_files+=("$f")
    done < <(find "$project_dir" -maxdepth 1 -name '*.jsonl' -type f 2>/dev/null)
    [ "${#jsonl_files[@]}" -eq 0 ] && return 0
    local match_file
    match_file=$(grep -rl "\"customTitle\":\"$name\"" "${jsonl_files[@]}" 2>/dev/null \
                  | xargs ls -t 2>/dev/null | head -1)
    if [ -n "$match_file" ]; then
        local b="${match_file##*/}"; b="${b%.jsonl}"
        printf '%s\n' "$b"
    fi
}

# UUID pattern check: excludes "memory/" and other non-session dirs.
_cc_is_uuid() {
    [[ "$1" =~ ^[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}$ ]]
}

# Enumerate transcripts under $project_dir, newest first.
# Output: lines of "<mtime>\t<sid>\t<title>".
_cc_enumerate_sessions() {
    local project_dir="$1"
    [ -d "$project_dir" ] || return 0
    find "$project_dir" -mindepth 1 -maxdepth 1 -name '*.jsonl' -type f 2>/dev/null |
      while IFS= read -r f; do
          local sid="${f##*/}"; sid="${sid%.jsonl}"
          _cc_is_uuid "$sid" || continue
          local ts
          ts=$(stat -f %m "$f" 2>/dev/null || stat -c %Y "$f" 2>/dev/null || printf '0')
          local title
          title=$(grep -m1 -oE '"customTitle":"[^"]*"' "$f" 2>/dev/null | head -1 \
                    | sed 's/"customTitle":"//; s/"$//')
          printf '%s\t%s\t%s\n' "$ts" "$sid" "${title:-(no title)}"
      done | sort -rnu -k1,1
}

_cc_list_sessions() {
    local project_dir="$1"
    if [ ! -d "$project_dir" ]; then
        printf '%s\n' "no sessions for ${PWD##*/}"
        return 0
    fi
    printf '%s\n' "Recent sessions for ${PWD##*/}:"
    _cc_enumerate_sessions "$project_dir" | head -10 |
      while IFS=$'\t' read -r ts sid title; do
          local when
          when=$(date -r "$ts" '+%Y-%m-%d %H:%M' 2>/dev/null \
                 || date -d "@$ts" '+%Y-%m-%d %H:%M' 2>/dev/null)
          printf '  %s  %s  %s\n' "$when" "${sid:0:8}..." "$title"
      done
}
