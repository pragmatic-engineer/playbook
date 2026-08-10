# SPDX-FileCopyrightText: 2026 Igor Santos
# SPDX-License-Identifier: MIT
# cc module: clean resume (clone transcript, strip config overrides)
#
# Clone the latest matching transcript with config-override slash commands
# stripped, then resume the clone. Conversation preserved, runtime config
# resets to settings.json defaults. Original transcript is untouched.
#
# Stripped (type=system, subtype=local_command):
#   /model, /effort, /config, /output-style, /style
#
# NOT stripped:
#   - corresponding stdout entries (just display text, harmless on replay)
#   - other slash commands (/clear, /resume, /mcp, /plugin, /permissions, …)
#   - permission-mode entries (security-relevant; user should re-grant)
_cc_clean_resume() {
    local project_dir="$1" name="$2"
    shift 2

    local old_sid
    old_sid="$(_cc_find_session_by_title "$project_dir" "$name")"
    if [[ -z "$old_sid" ]]; then
        print -- "→ cc clean: no matching session for '$name'; starting fresh"
        command claude -n "$name" "$@"
        return $?
    fi

    local old_jsonl="$project_dir/$old_sid.jsonl"
    if [[ ! -f "$old_jsonl" ]]; then
        print -- "→ cc clean: transcript missing at $old_jsonl"
        return 1
    fi

    if ! command -v jq >/dev/null 2>&1; then
        print -- "→ cc clean: jq required but not found"
        return 1
    fi
    if ! command -v uuidgen >/dev/null 2>&1; then
        print -- "→ cc clean: uuidgen required but not found"
        return 1
    fi

    local new_sid
    new_sid="$(uuidgen | tr 'A-Z' 'a-z')"
    local new_jsonl="$project_dir/$new_sid.jsonl"

    local strip_re='<command-name>/(model|effort|config|output-style|style)</command-name>'

    local total_in
    total_in=$(wc -l < "$old_jsonl" | tr -d ' ')

    jq -c --arg pat "$strip_re" '
        select(.type != "system" or ((.content // "") | test($pat) | not))
    ' "$old_jsonl" > "$new_jsonl" || {
        print -- "→ cc clean: jq filter failed"
        rm -f "$new_jsonl"
        return 1
    }

    local total_out stripped
    total_out=$(wc -l < "$new_jsonl" | tr -d ' ')
    stripped=$((total_in - total_out))

    # Rewrite sessionId fields so the harness sees a coherent transcript.
    local tmp="$new_jsonl.tmp"
    jq -c --arg sid "$new_sid" '
        if .sessionId then .sessionId = $sid else . end
    ' "$new_jsonl" > "$tmp" && mv "$tmp" "$new_jsonl"

    # Copy the tool-results sidecar dir. Copy, not symlink: the harness may write
    # into the new session's sidecar, and symlinks would leak those writes back
    # into the original, breaking the "original transcript is untouched" promise.
    if [[ -d "$project_dir/$old_sid" ]]; then
        mkdir -p "$project_dir/$new_sid"
        for f in "$project_dir/$old_sid"/*(N); do
            [[ -e "$f" ]] || continue
            cp -R "$f" "$project_dir/$new_sid/${f:t}" 2>/dev/null
        done
    fi

    print -- "→ cc clean: cloned ${old_sid:0:8}… → ${new_sid:0:8}… (stripped $stripped override entries)"
    print -- "→ cc clean: settings.json + plugins + hooks reload from current config"
    _cc_config_stamp
    command claude --resume "$new_sid" -n "$name" "$@"
}
