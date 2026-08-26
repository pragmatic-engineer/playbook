#!/usr/bin/env bash
# SPDX-FileCopyrightText: 2026 Igor Santos
# SPDX-License-Identifier: MIT
set +e   # Never let an error silently kill the status line
# ╔══════════════════════════════════════════════════════════════════════════════════════════════╗
# ║  Claude Code Status Line                                                                     ║
# ║                                                                                              ║
# ║  Two-line minimalist status bar rendered below the Claude Code prompt.                       ║
# ║  Receives session data as JSON on stdin; outputs ANSI-colored text.                          ║
# ║                                                                                              ║
# ║  Line 1 (LEFT):  ~/path branch [$]                                                           ║
# ║  Line 1 (RIGHT): PR #42 CI ✗ JIRA @author CHANGES REQUESTED by name (1h ago)                ║
# ║  Line 2: Opus 4.8 (high) | Ctx [███░░░] 38% | 5h 23% (2h left) | $0.42 | Cache 99%           ║
# ║                                                                                              ║
# ║  Secondary segments self-suppress until actionable (CI only on fail/run, 7d                  ║
# ║  rate only >50%, compaction arrow only >65%, session age only >10m).                         ║
# ║  Configure via STATUSLINE_* env vars or the feature flags below.                             ║
# ╚══════════════════════════════════════════════════════════════════════════════════════════════╝

# ┌──────────────────────────────────────────────────────────────────────────────┐
# │  Feature Flags                                                              │
# │                                                                             │
# │  Toggle individual sections on/off. Set to "true" to enable, anything       │
# │  else to disable. Override via environment: STATUSLINE_SHOW_GIT=false       │
# └──────────────────────────────────────────────────────────────────────────────┘

SHOW_GIT="${STATUSLINE_SHOW_GIT:-true}"             # Git branch + dirty indicator
SHOW_PR="${STATUSLINE_SHOW_PR:-true}"                # GitHub PR number, author, review status
SHOW_CI="${STATUSLINE_SHOW_CI:-true}"                # CI status rollup (workspace projects only)
SHOW_MODEL="${STATUSLINE_SHOW_MODEL:-true}"          # Model name + effort
SHOW_CONTEXT="${STATUSLINE_SHOW_CONTEXT:-true}"      # Context window % (with compaction proximity arrow)
SHOW_SESSION_AGE="${STATUSLINE_SHOW_SESSION_AGE:-true}"   # "Up 18m" since session start (needs hooks)
SHOW_CACHE_RATIO="${STATUSLINE_SHOW_CACHE_RATIO:-true}"   # Cache hit ratio %
SHOW_RATE_LIMITS="${STATUSLINE_SHOW_RATE_LIMITS:-true}" # 5h quota (+ 7d when >50%)

CTX_BAR_WIDTH="${STATUSLINE_CTX_BAR_WIDTH:-10}"   # cells in the context bar (1 per 10%)

PR_CACHE_TTL="${STATUSLINE_PR_CACHE_TTL:-60}"        # Seconds before cached PR data is refetched
CI_CACHE_TTL="${STATUSLINE_CI_CACHE_TTL:-60}"        # Seconds before cached CI data is refetched

# Cache directory. Defaults to $XDG_CACHE_HOME/statusline or ~/.cache/statusline.
# Created with 0700 perms (owner-only) on first run so cached PR bodies stay private.
CACHE_DIR="${STATUSLINE_CACHE_DIR:-${XDG_CACHE_HOME:-$HOME/.cache}/statusline}"

# GitHub remote parsing (github.com + GitHub Enterprise *.ghe.com) lives in a
# standalone, unit-tested helper so the status line builds host-correct PR and
# branch links. A missing file just leaves the GitHub segments disabled.
STATUSLINE_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
# shellcheck source=shell/gh-remote.sh
[[ -r "$STATUSLINE_DIR/shell/gh-remote.sh" ]] && source "$STATUSLINE_DIR/shell/gh-remote.sh"

# ┌──────────────────────────────────────────────────────────────────────────────┐
# │  Colour Palette: Catppuccin Mocha                                           │
# │                                                                             │
# │  Uses true-colour (24-bit) ANSI escapes. Requires a terminal that           │
# │  supports \033[38;2;R;G;Bm sequences (iTerm2, Kitty, Alacritty, etc).      │
# └──────────────────────────────────────────────────────────────────────────────┘

GREEN='\033[38;2;166;227;161m'    # #a6e3a1: paths, values, approved
RED='\033[38;2;243;139;168m'      # #f38ba8: git branch, changes requested, closed
YELLOW='\033[38;2;249;226;175m'   # #f9e2af: git status brackets, review required
ORANGE='\033[38;2;250;179;135m'   # #fab387: model name, cost, PR number
WHITE='\033[38;2;205;214;244m'    # #cdd6f4: connectors ("via"), PR author
TEAL='\033[38;2;148;226;213m'     # #94e2d5: cache statistics
MAUVE='\033[38;2;203;166;247m'    # #cba6f7: merged PRs
DIM='\033[38;2;127;132;156m'      # #7f849c: labels, separators
RESET='\033[0m'

SEP="${DIM} | ${RESET}"           # Dim pipe between line-2 segments

# ┌──────────────────────────────────────────────────────────────────────────────┐
# │  Utility Functions                                                           │
# └──────────────────────────────────────────────────────────────────────────────┘

# Colour a rate-limit percentage: green < 50%, yellow 50-80%, red > 80%
rl_color() {
    local pct="${1%%.*}"
    if [[ "${pct:-0}" -gt 80 ]]; then printf '%b' "$RED"
    elif [[ "${pct:-0}" -gt 50 ]]; then printf '%b' "$YELLOW"
    else printf '%b' "$GREEN"
    fi
}

# Colour a context-usage percentage with thresholds tuned to autocompact at 90%.
# < 65% green (plenty of room), 65-80% yellow (plan to wrap), > 80% red (urgent).
ctx_color() {
    local pct="${1%%.*}"
    if [[ "${pct:-0}" -gt 80 ]]; then printf '%b' "$RED"
    elif [[ "${pct:-0}" -gt 65 ]]; then printf '%b' "$YELLOW"
    else printf '%b' "$GREEN"
    fi
}

# Render a context-usage progress bar of `width` cells, one cell per (100/width)%.
# Filled cells use █, empty cells ░. Fill rounds to the nearest cell and clamps
# to [0, width]. Caller wraps it in color. Usage: ctx_bar 38 10  ->  ████░░░░░░
ctx_bar() {
    local used="${1%%.*}" width="${2:-10}"
    [[ -z "$used" ]] && used=0
    local filled=$(( (used * width + 50) / 100 ))
    [[ "$filled" -lt 0 ]] && filled=0
    [[ "$filled" -gt "$width" ]] && filled=$width
    local empty=$(( width - filled )) bar="" i
    for ((i = 0; i < filled; i++)); do bar="${bar}█"; done
    for ((i = 0; i < empty;  i++)); do bar="${bar}░"; done
    printf '%s' "$bar"
}

# Format an age in seconds as a compact "Xs", "Xm", or "XhYm" string.
fmt_age() {
    local s="${1:-0}"
    if   [[ "$s" -lt 60   ]]; then printf '%ds' "$s"
    elif [[ "$s" -lt 3600 ]]; then printf '%dm' "$((s / 60))"
    else
        local h=$((s / 3600)) m=$(((s % 3600) / 60))
        if [[ "$m" -eq 0 ]]; then printf '%dh' "$h"
        else printf '%dh%dm' "$h" "$m"
        fi
    fi
}

# Coarse age for the "N ago" displays: whole minutes, then hours, then days.
fmt_ago() {
    local s="${1:-0}"
    if   [[ "$s" -lt 3600  ]]; then printf '%dm' "$((s / 60))"
    elif [[ "$s" -lt 86400 ]]; then printf '%dh' "$((s / 3600))"
    else printf '%dd' "$((s / 86400))"; fi
}

# Format a raw token count as a compact "NNNk" (rounded) or bare integer under
# 1000. Usage: fmt_tokens 127483 -> 127k
fmt_tokens() {
    local n="${1:-0}"
    [[ -z "$n" ]] && n=0
    if [[ "$n" -ge 1000 ]]; then
        printf '%dk' $(( (n + 500) / 1000 ))
    else
        printf '%d' "$n"
    fi
}

# Context-rot threshold: once raw token usage crosses 100k, model quality is
# known to degrade regardless of how much headroom remains in the window (a
# 1M-context model at 100k tokens is nowhere near its own autocompact trigger
# but has already crossed this). Independent of ctx_color's window-relative
# percentage thresholds. Empty below the threshold so the marker only shows
# once it's actionable.
CONTEXT_ROT_THRESHOLD=100000
context_rot_warning() {
    local tokens="${1:-0}"
    [[ -z "$tokens" ]] && return 0
    [[ "$tokens" -lt "$CONTEXT_ROT_THRESHOLD" ]] && return 0
    printf '1'
}

# Cache hit ratio as integer percent: read / (read + write). Empty if no cache
# activity yet. High ratio = warm cache = cheap turns. Low ratio = paying full
# input price each turn.
cache_hit_pct() {
    local write="${1:-0}" read="${2:-0}"
    local total=$((write + read))
    [[ "$total" -eq 0 ]] && return 0
    printf '%d' $(( (read * 100) / total ))
}

# Colour for cache hit ratio: <50% red (cache cold), 50-80% yellow, >80% green.
cache_color() {
    local pct="${1:-0}"
    if   [[ "$pct" -ge 80 ]]; then printf '%b' "$GREEN"
    elif [[ "$pct" -ge 50 ]]; then printf '%b' "$YELLOW"
    else printf '%b' "$RED"
    fi
}

# Cost burn rate in $/min, rounded to 4 decimals. Empty if no usage yet.
cost_per_min() {
    local cost="${1:-0}" wall_ms="${2:-0}"
    [[ "$wall_ms" -le 0 ]] && return 0
    awk -v c="$cost" -v w="$wall_ms" 'BEGIN { printf "%.4f", (c * 60000) / w }'
}

# Compaction proximity: returns the gap from current % to the autocompact
# trigger (CLAUDE_AUTOCOMPACT_PCT_OVERRIDE, default 90). Empty when far away
# (< 50% used) so the indicator only appears when actionable.
compact_gap() {
    local used="${1%%.*}"
    local trigger="${CLAUDE_AUTOCOMPACT_PCT_OVERRIDE:-90}"
    [[ "${used:-0}" -lt 50 ]] && return 0
    local gap=$((trigger - used))
    [[ "$gap" -lt 0 ]] && gap=0
    printf '%d' "$gap"
}

# Append a pre-rendered segment to a named line variable, inserting SEP between
# segments. Usage: append_raw line_2 "$(render_model)"
append_raw() {
    local var="$1" content="$2"
    local cur="${!var}"
    [[ -n "$cur" ]] && cur="${cur}${SEP}"
    cur="${cur}${content}"
    printf -v "$var" '%s' "$cur"
}

# Strip ANSI escape codes to measure visible string length
strip_ansi() {
    printf '%s' "$1" | sed 's/\x1b\[[0-9;]*m//g'
}

# Count visible display columns (codepoint count: total bytes minus UTF-8 continuation
# bytes 0x80-0xBF). Locale-independent; handles multibyte glyphs like █ ░ ✗ ● ϓ.
visible_len() { strip_ansi "$1" | LC_ALL=C awk '{ t=length($0); c=gsub(/[\200-\277]/,"",$0); print t-c }'; }

# File mtime in epoch seconds. macOS uses stat -f %m, GNU stat uses -c %Y.
file_mtime() {
    stat -f %m "$1" 2>/dev/null || stat -c %Y "$1" 2>/dev/null || echo 0
}

# Parse an ISO 8601 UTC timestamp (e.g. "2024-01-15T10:30:00Z") to epoch seconds.
# macOS/BSD: date -j -f; Linux/GNU: date -d. TZ=UTC ensures correct epoch on both.
# Returns empty string on parse failure (both branches fail).
iso_to_epoch() { TZ=UTC date -j -f "%Y-%m-%dT%H:%M:%SZ" "$1" +%s 2>/dev/null || TZ=UTC date -d "$1" +%s 2>/dev/null; }

# Filesystem-safe slug from arbitrary string (for cache file names).
cache_slug() {
    printf '%s' "$1" | sed 's/[^A-Za-z0-9_.-]/_/g'
}

# True when the host terminal is known to support OSC 8 hyperlinks.
# Honours STATUSLINE_OSC8=true/false to force-enable or force-disable.
# Without an override, recognises Ghostty, iTerm2, kitty, WezTerm, VS Code,
# Hyper, foot, and libvte (GNOME Terminal) ≥ 0.50. Unknown terminals fall
# back to plain text so the statusline never shows raw escape sequences.
terminal_supports_osc8() {
    case "${STATUSLINE_OSC8:-}" in
        true|1) return 0 ;;
        false|0) return 1 ;;
    esac
    case "${TERM_PROGRAM:-}" in
        ghostty|iTerm.app|WezTerm|vscode|Hyper) return 0 ;;
    esac
    case "${TERM:-}" in
        xterm-kitty|foot|foot-extra) return 0 ;;
    esac
    [[ -n "${GHOSTTY_RESOURCES_DIR:-}" ]] && return 0
    [[ -n "${KITTY_WINDOW_ID:-}" ]] && return 0
    [[ -n "${WEZTERM_EXECUTABLE:-}" ]] && return 0
    if [[ "${VTE_VERSION:-0}" =~ ^[0-9]+$ ]] && [[ "${VTE_VERSION:-0}" -ge 5000 ]]; then
        return 0
    fi
    return 1
}

# Wrap text in an OSC 8 hyperlink escape so supporting terminals render it
# as clickable. Detects terminal capability via terminal_supports_osc8 and
# falls back to plain text on terminals that would otherwise show raw escapes
# (older xterm, Apple Terminal, Alacritty, etc).
#
# Emits literal \033 sequences (not pre-interpreted ESC bytes) so that the
# downstream `printf '%b'` that renders the status line interprets these
# escapes together with the color codes embedded in $text. Pre-interpreting
# them here would put a real ESC \ before the literal \033 of a color code
# and %b would treat the pair as \\\\033, dropping a backslash.
# Usage: osc8_link "https://example.com" "click me"
osc8_link() {
    local url="$1" text="$2"
    if [[ -z "$url" ]] || ! terminal_supports_osc8; then
        printf '%s' "$text"
        return
    fi
    printf '%s' '\033]8;;'"$url"'\033\\'"$text"'\033]8;;\033\\'
}

# True when the path is under ~/Workspace/ (work projects). The trailing slash is
# load-bearing: without it ~/Workspace-personal/ would also match, but that tree
# is explicitly excluded from CI status display.
is_workspace_project() {
    [[ "$1" == "$HOME/Workspace/"* ]]
}

# Ensure cache dir exists with restrictive perms. chmod runs only on first creation
# so the user can loosen perms later without us silently resetting them.
if [[ "${BASH_SOURCE[0]}" == "$0" ]]; then
if [[ ! -d "$CACHE_DIR" ]]; then
    mkdir -p "$CACHE_DIR" 2>/dev/null && chmod 700 "$CACHE_DIR" 2>/dev/null
fi

# jq program shared by the PR-path CI rollup and the standalone CI rollup.
# Input: any object with a .statusCheckRollup array (may be absent).
# Output: one line "state failed running total"  (state: none | fail | running | pass).
JQ_CI_ROLLUP='
    def is_failed:
        (((.conclusion // "") | ascii_downcase) | (. == "failure" or . == "cancelled" or . == "timed_out" or . == "action_required" or . == "startup_failure" or . == "stale"))
        or (((.state // "") | ascii_downcase) | (. == "failure" or . == "error"));
    def is_running:
        (((.status // "") | ascii_downcase) | (. == "in_progress" or . == "queued" or . == "pending" or . == "waiting"))
        or (((.state // "") | ascii_downcase) | (. == "pending"));
    (.statusCheckRollup // []) as $checks
    | ($checks | length) as $total
    | ($checks | map(select(is_failed)) | length) as $failed
    | ($checks | map(select(is_running)) | length) as $running
    | (if $total == 0 then "none"
       elif $failed > 0 then "fail"
       elif $running > 0 then "running"
       else "pass" end) as $state
    | "\($state) \($failed) \($running) \($total)"
'

# ┌──────────────────────────────────────────────────────────────────────────────┐
# │  Parse JSON Input                                                            │
# │                                                                              │
# │  Claude Code pipes session state as JSON to stdin. A single jq call          │
# │  extracts all fields at once using @sh quoting, then eval sets them as       │
# │  shell variables. This avoids spawning jq multiple times.                    │
# └──────────────────────────────────────────────────────────────────────────────┘

input=$(cat 2>/dev/null || true)

if command -v jq >/dev/null 2>&1 && [[ -n "$input" ]]; then
    _jq_out=$(printf '%s' "$input" | jq -r '
        @sh "cwd=\(.cwd // .workspace.current_dir // env.PWD // "")",
        @sh "session_id=\(.session_id // "")",
        @sh "model=\(.model.display_name // "")",
        @sh "used=\(.context_window.used_percentage // "")",
        @sh "ctx_total_tokens=\(.context_window.total_input_tokens // "")",
        @sh "ctx_window_size=\(.context_window.context_window_size // "")",
        @sh "cache_create=\(.context_window.current_usage.cache_creation_input_tokens // "")",
        @sh "cache_read=\(.context_window.current_usage.cache_read_input_tokens // "")",
        @sh "rl_5h=\(.rate_limits.five_hour.used_percentage // "")",
        @sh "rl_5h_reset=\(.rate_limits.five_hour.resets_at // "")",
        @sh "rl_7d=\(.rate_limits.seven_day.used_percentage // "")",
        @sh "json_effort=\(.effort.level // "")",
        @sh "json_thinking=\(if .thinking.enabled then "true" else "" end)",
        @sh "cost_usd=\(.cost.total_cost_usd // "")",
        @sh "wall_ms=\(.cost.total_duration_ms // "")"
    ' 2>/dev/null) && eval "$_jq_out" 2>/dev/null || true
fi

cwd="${cwd:-$PWD}"

# ┌──────────────────────────────────────────────────────────────────────────────┐
# │  Telemetry: context usage and cost                                          │
# │                                                                              │
# │  statusline.sh is the only component whose stdin payload carries used       │
# │  percentage and cost, so it persists one sample per render for the memory   │
# │  capture pipeline and marks when usage crosses the capture threshold.       │
# │  Every write is guarded: a missing session_id, an unwritable directory, or  │
# │  a failed write must still leave the status line printing and exiting 0.    │
# └──────────────────────────────────────────────────────────────────────────────┘

if [[ -n "${session_id:-}" ]]; then
    telemetry_dir="$HOME/.claude/runtime/${session_id}"
    mkdir -p "$telemetry_dir" 2>/dev/null
    if [[ -d "$telemetry_dir" && -w "$telemetry_dir" ]]; then
        printf '{"ts":%s,"cost_usd":%s,"used_pct":%s}\n' \
            "$(date +%s)" "${cost_usd:-0}" "${used:-0}" \
            >> "$telemetry_dir/telemetry.jsonl" 2>/dev/null || true

        # Fire once per CROSSING, not once per render above the line. Without
        # the latch the Stop hook consumes the marker and the next render
        # re-arms it, costing a turn every turn past the threshold.
        #
        # ABSENCE of `capture-fired` means armed, and that direction matters: a
        # fresh session dir has no state files, and a session can open above the
        # threshold on a resume, so a sentinel that had to exist first would
        # silently never fire. Dropping back under the line clears the latch, so
        # a dip and climb gets a second prompt.
        capture_at="${CC_CAPTURE_AT:-70}"
        used_int="${used%%.*}"
        if [[ "${used_int:-0}" -ge "$capture_at" ]] 2>/dev/null; then
            if [[ ! -f "$telemetry_dir/capture-fired" ]]; then
                : > "$telemetry_dir/capture-due" 2>/dev/null || true
                : > "$telemetry_dir/capture-fired" 2>/dev/null || true
            fi
        else
            rm -f "$telemetry_dir/capture-fired" 2>/dev/null || true
        fi
    fi
fi

# ╔══════════════════════════════════════════════════════════════════════════════╗
# ║  LINE 1: Working directory, git info, PR status                              ║
# ╠══════════════════════════════════════════════════════════════════════════════╣

# ── Left side: path + git branch + dirty indicator + node version ──

# Replace $HOME with ~ for readability. $HOME is quoted inside the ${..#..}
# because the strip pattern is a glob, not a literal: an unquoted $HOME
# containing [, * or ? matches as a pattern and silently strips nothing, so a
# home directory like /tmp/a[b]c renders the full path instead of ~.
display_path="$cwd"
if [[ "$display_path" == "$HOME"* ]]; then
    display_path="~${display_path#"$HOME"}"
fi

left="${GREEN}${display_path}${RESET}"

# ── Git branch and dirty indicator ──

branch=""
git_in_repo=false
gh_path=""          # "<owner>/<repo>" when origin is GitHub, else empty
repo_owner=""
repo_name=""

if [[ "$SHOW_GIT" == true ]]; then
    if git --no-optional-locks -C "$cwd" rev-parse --git-dir >/dev/null 2>&1; then
        git_in_repo=true
        branch=$(git --no-optional-locks -C "$cwd" branch --show-current 2>/dev/null)
        [[ -z "$branch" ]] && branch="detached"

        # Parse origin remote once and reuse downstream. A missing remote (fresh
        # `git init`) or a non-GitHub host leaves gh_path empty, which is the
        # signal every gh-using block uses to bail out without spawning `gh`.
        # gh_remote_parse accepts github.com and GitHub Enterprise (*.ghe.com)
        # and yields the host, so the OSC 8 links point at the right domain.
        remote_url=$(git --no-optional-locks -C "$cwd" config --get remote.origin.url 2>/dev/null)
        if gh_remote=$(gh_remote_parse "$remote_url" 2>/dev/null); then
            gh_host="${gh_remote%%$'\t'*}"
            gh_path="${gh_remote#*$'\t'}"
            repo_owner="${gh_path%%/*}"
            repo_name="${gh_path#*/}"
        fi
    fi

    if [[ -n "$branch" ]]; then
        # Render branch as an OSC 8 clickable hyperlink to the GitHub branch URL
        # when origin is a GitHub remote. For non-GitHub remotes or detached
        # HEAD, fall back to plain text.
        branch_link=""
        if [[ "$branch" != "detached" && -n "$gh_path" ]]; then
            branch_link="https://${gh_host}/${gh_path}/tree/${branch}"
        fi

        if [[ -n "$branch_link" ]]; then
            # OSC 8 hyperlink: ESC ] 8 ;; URL ST  TEXT  ESC ] 8 ;; ST   (ST = ESC + \)
            # Use the same \033 + \\\\ pattern as the colour vars so printf %b
            # interprets them uniformly at the final output step.
            branch_display="\\033]8;;${branch_link}\\033\\\\${branch}\\033]8;;\\033\\\\"
        else
            branch_display="${branch}"
        fi
        left="${left} ${RED}ϓ ${branch_display}${RESET}"
    fi

    # [$] = clean, [+] = dirty. Uses diff (not status --porcelain) to skip untracked scan.
    if [[ "$git_in_repo" == true ]]; then
        if git --no-optional-locks -C "$cwd" diff --quiet 2>/dev/null \
            && git --no-optional-locks -C "$cwd" diff --cached --quiet 2>/dev/null; then
            left="${left} ${YELLOW}[\$]${RESET}"
        else
            left="${left} ${YELLOW}[+]${RESET}"
        fi
    fi
fi


# ── Right side: GitHub PR status ──
# render_pr_right reads from cache and renders immediately; if the cache is stale
# or missing it fires a fully-detached background gh call to refresh for the next
# render. The synchronous path NEVER blocks on gh.

# Fire an async cache-refresh for the PR JSON, guarded by a lock file so only one
# refresh is in flight at a time. Treats locks older than 30s as stale.
_pr_fire_refresh() {
    local pr_cache_file="$1" pr_lock pr_lock_age
    pr_lock="${pr_cache_file}.lock"
    pr_lock_age=0
    [[ -f "$pr_lock" ]] && pr_lock_age=$(( $(date +%s) - $(file_mtime "$pr_lock") ))
    if [[ "$pr_lock_age" -gt 30 ]]; then
        rm -f "$pr_lock" 2>/dev/null || true
    fi
    if ( set -o noclobber; : > "$pr_lock" ) 2>/dev/null; then
        nohup bash -c '
            trap "rm -f \"$1\"" EXIT
            NO_COLOR=1 GIT_TERMINAL_PROMPT=0 gh pr view \
                --json number,author,reviewDecision,state,mergedAt,closedAt,body,latestReviews,reviewRequests,statusCheckRollup \
                2>/dev/null > "$2.tmp.$$" && mv "$2.tmp.$$" "$2"
        ' _ "$pr_lock" "$pr_cache_file" </dev/null >/dev/null 2>&1 &
        disown 2>/dev/null || true
    fi
}

# Render the PR section of line 1's right side.
# Sets global `right`, and for workspace projects sets ci_state/ci_failed/
# ci_running/ci_total so the standalone CI block below can skip its own fetch.
render_pr_right() {
    [[ "$SHOW_PR" == true ]] || return 0
    [[ "$git_in_repo" == true ]] || return 0
    [[ -n "$branch" && "$branch" != "detached" ]] || return 0
    [[ -n "$gh_path" ]] || return 0
    command -v gh >/dev/null 2>&1 || return 0

    # Shared cache (repo + branch). May not exist, be empty (known "no PR" marker),
    # or be stale; we use it as-is and refresh in the background.
    local pr_cache_file pr_cache_age pr_json
    pr_cache_file="${CACHE_DIR}/pr-$(cache_slug "${cwd}::${branch}").json"
    pr_cache_age=999999
    [[ -f "$pr_cache_file" ]] && pr_cache_age=$(( $(date +%s) - $(file_mtime "$pr_cache_file") ))

    pr_json=""
    [[ -f "$pr_cache_file" ]] && pr_json=$(cat "$pr_cache_file" 2>/dev/null || true)

    [[ "$pr_cache_age" -ge "$PR_CACHE_TTL" ]] && _pr_fire_refresh "$pr_cache_file"

    [[ -n "$pr_json" ]] || return 0

    # Single jq pass: extract all PR fields + CI rollup in one fork.
    # (CI rollup uses the same is_failed/is_running logic as $JQ_CI_ROLLUP.)
    # ci_* are NOT declared local: the standalone CI block reads them after return.
    local _pr_out pr_number pr_author pr_review pr_state pr_merged_at pr_closed_at
    local jira_from_body ci_summary pr_pending pr_completed
    _pr_out=$(printf '%s' "$pr_json" | jq -r '
        def is_failed:
            (((.conclusion // "") | ascii_downcase) | (. == "failure" or . == "cancelled" or . == "timed_out" or . == "action_required" or . == "startup_failure" or . == "stale"))
            or (((.state // "") | ascii_downcase) | (. == "failure" or . == "error"));
        def is_running:
            (((.status // "") | ascii_downcase) | (. == "in_progress" or . == "queued" or . == "pending" or . == "waiting"))
            or (((.state // "") | ascii_downcase) | (. == "pending"));
        (.reviewRequests // []) as $req
        | [$req[]? | (.login // .name // "team")] as $pending_logins
        | (.statusCheckRollup // []) as $checks
        | ($checks | length) as $ci_total
        | ($checks | map(select(is_failed)) | length) as $ci_failed
        | ($checks | map(select(is_running)) | length) as $ci_running
        | (if $ci_total == 0 then "none"
           elif $ci_failed > 0 then "fail"
           elif $ci_running > 0 then "running"
           else "pass" end) as $ci_st
        | @sh "pr_number=\(.number // "" | tostring | if . == "null" then "" else . end)",
          @sh "pr_author=\(.author.login // "")",
          @sh "pr_review=\(.reviewDecision // "")",
          @sh "pr_state=\(.state // "")",
          @sh "pr_merged_at=\(.mergedAt // "")",
          @sh "pr_closed_at=\(.closedAt // "")",
          @sh "jira_from_body=\(.body // "" | [scan("[A-Z][A-Z0-9]+-[0-9]+")] | if length > 0 then .[0] else "" end)",
          @sh "ci_summary=\("\($ci_st) \($ci_failed) \($ci_running) \($ci_total)")",
          @sh "pr_pending=\([$pending_logins[]] | join("\n"))",
          @sh "pr_completed=\([.latestReviews[]? |
              select(.author.login != "coderabbitai" and .state != "COMMENTED"
                  and ([.author.login] | inside($pending_logins) | not)) |
              (if .state == "APPROVED" then "g"
               elif .state == "CHANGES_REQUESTED" then "r"
               else "d" end) + ":" + .author.login + ":" + (.submittedAt // "")] | join("\n"))"
    ' 2>/dev/null) && eval "$_pr_out" 2>/dev/null || true

    # Populate CI globals (workspace projects only). The standalone CI block below
    # skips its own fetch when ci_state is already set.
    ci_state=""; ci_failed=0; ci_running=0; ci_total=0
    if [[ "$SHOW_CI" == true ]] && is_workspace_project "$cwd"; then
        read -r ci_state ci_failed ci_running ci_total <<< "$ci_summary"
    fi

    [[ -n "$pr_number" ]] || return 0

    # Jira ticket: branch name takes priority over first match in PR body.
    local jira_ticket=""
    if [[ "$branch" =~ ([A-Z][A-Z0-9]+-[0-9]+) ]]; then
        jira_ticket="${BASH_REMATCH[1]}"
    elif [[ -n "$jira_from_body" ]]; then
        jira_ticket="$jira_from_body"
    fi

    local pr_label
    pr_label="${ORANGE}PR #${pr_number}${RESET}"
    if [[ -n "$repo_owner" && -n "$repo_name" ]]; then
        pr_label=$(osc8_link "https://${gh_host}/${repo_owner}/${repo_name}/pull/${pr_number}" "$pr_label")
    fi
    right="$pr_label"

    # CI badge sits right after the PR number. Pass is silent; only fail/running surfaces.
    case "$ci_state" in
        fail)    right="${right} ${RED}CI ✗ ${ci_failed}/${ci_total}${RESET}" ;;
        running) right="${right} ${YELLOW}CI ● ${ci_running}/${ci_total}${RESET}" ;;
    esac

    if [[ -n "$jira_ticket" ]]; then
        local jira_label
        jira_label="${TEAL}${jira_ticket}${RESET}"
        jira_label=$(osc8_link "https://clipboard.atlassian.net/browse/${jira_ticket}" "$jira_label")
        right="${right} ${jira_label}"
    fi

    [[ -n "$pr_author" ]] && right="${right} ${WHITE}@${pr_author}${RESET}"

    # PR lifecycle state takes priority over review decision.
    # A merged PR's reviewDecision is still "APPROVED" but the user
    # needs to see MERGED, not the stale review status.
    local show_reviewers=true
    local state_epoch diff_s state_ago
    case "$pr_state" in
        MERGED)
            right="${right} ${MAUVE}MERGED${RESET}"
            if [[ -n "$pr_merged_at" ]]; then
                state_epoch=$(iso_to_epoch "$pr_merged_at")
                if [[ -n "$state_epoch" ]]; then
                    diff_s=$(( $(date +%s) - state_epoch ))
                    state_ago="$(fmt_ago "$diff_s") ago"
                    right="${right} ${DIM}(${state_ago})${RESET}"
                fi
            fi
            show_reviewers=false ;;
        CLOSED)
            right="${right} ${RED}CLOSED${RESET}"
            if [[ -n "$pr_closed_at" ]]; then
                state_epoch=$(iso_to_epoch "$pr_closed_at")
                if [[ -n "$state_epoch" ]]; then
                    diff_s=$(( $(date +%s) - state_epoch ))
                    state_ago="$(fmt_ago "$diff_s") ago"
                    right="${right} ${DIM}(${state_ago})${RESET}"
                fi
            fi
            show_reviewers=false ;;
        *)
            case "$pr_review" in
                APPROVED)           right="${right} ${GREEN}APPROVED${RESET}" ;;
                CHANGES_REQUESTED)  right="${right} ${RED}CHANGES REQUESTED${RESET}" ;;
                REVIEW_REQUIRED)    right="${right} ${YELLOW}REVIEW REQUIRED${RESET}" ;;
                *)                  right="${right} ${DIM}NO REVIEW${RESET}"; show_reviewers=false ;;
            esac
            ;;
    esac

    # Append completed reviewers: "by name (2h ago) name (1d ago)"
    if [[ -n "$pr_completed" && "$show_reviewers" == true ]]; then
        right="${right} ${DIM}by${RESET}"
        local now_epoch; now_epoch=$(date +%s)
        local entry local_color local_rest local_name local_ts local_ago review_epoch
        while IFS= read -r entry; do
            [[ -z "$entry" ]] && continue
            local_color="${entry%%:*}"
            local_rest="${entry#*:}"
            local_name="${local_rest%%:*}"
            local_ts="${local_rest#*:}"
            local_ago=""
            if [[ -n "$local_ts" ]]; then
                review_epoch=$(iso_to_epoch "$local_ts")
                if [[ -n "$review_epoch" ]]; then
                    diff_s=$(( now_epoch - review_epoch ))
                    local_ago="$(fmt_ago "$diff_s")"
                fi
            fi
            case "$local_color" in
                g) right="${right} ${GREEN}${local_name}${RESET}" ;;
                r) right="${right} ${RED}${local_name}${RESET}" ;;
                *) right="${right} ${DIM}${local_name}${RESET}" ;;
            esac
            [[ -n "$local_ago" ]] && right="${right} ${DIM}(${local_ago} ago)${RESET}"
        done <<< "$pr_completed"
    fi

    # Append pending reviewers: "waiting on name, name"
    if [[ -n "$pr_pending" ]]; then
        local pending_list="" name
        while IFS= read -r name; do
            [[ -z "$name" ]] && continue
            [[ -n "$pending_list" ]] && pending_list="${pending_list}, "
            pending_list="${pending_list}${name}"
        done <<< "$pr_pending"
        [[ -n "$pending_list" ]] && right="${right} ${DIM}waiting on${RESET} ${YELLOW}${pending_list}${RESET}"
    fi
}

right=""
ci_state=""; ci_failed=0; ci_running=0; ci_total=0
render_pr_right

# ╔══════════════════════════════════════════════════════════════════════════════╗
# ║  Standalone CI fetch (workspace projects without an open PR)                 ║
# ║                                                                              ║
# ║  When the branch has a PR, statusCheckRollup is already on the cached PR     ║
# ║  JSON (no extra round trip). For branches without a PR (e.g., master), we    ║
# ║  run a small GraphQL query against the branch's HEAD commit so the CI badge  ║
# ║  still appears on line 1.                                                    ║
# ╚══════════════════════════════════════════════════════════════════════════════╝

if [[ "$SHOW_CI" == true ]] && is_workspace_project "$cwd" \
    && [[ "$git_in_repo" == true ]] \
    && [[ -n "$branch" && "$branch" != "detached" ]] \
    && [[ -n "$repo_owner" && -n "$repo_name" ]] \
    && [[ -z "$ci_state" ]] \
    && command -v gh >/dev/null 2>&1; then

    ci_cache_file="${CACHE_DIR}/ci-$(cache_slug "${cwd}::${branch}").json"
    ci_cache_age=999999
    [[ -f "$ci_cache_file" ]] && ci_cache_age=$(( $(date +%s) - $(file_mtime "$ci_cache_file") ))

    # Read cache as-is (may be empty or stale).
    ci_json=""
    [[ -f "$ci_cache_file" ]] && ci_json=$(cat "$ci_cache_file" 2>/dev/null || true)

    # Refresh asynchronously when stale or missing; never block the render.
    if [[ "$ci_cache_age" -ge "$CI_CACHE_TTL" ]]; then
        ci_lock="${ci_cache_file}.lock"
        ci_lock_age=0
        [[ -f "$ci_lock" ]] && ci_lock_age=$(( $(date +%s) - $(file_mtime "$ci_lock") ))
        if [[ "$ci_lock_age" -gt 30 ]]; then
            rm -f "$ci_lock" 2>/dev/null || true
        fi
        if ( set -o noclobber; : > "$ci_lock" ) 2>/dev/null; then
            nohup bash -c '
                trap "rm -f \"$1\"" EXIT
                NO_COLOR=1 GIT_TERMINAL_PROMPT=0 gh api graphql \
                    -F owner="$2" -F repo="$3" -F branch="$4" \
                    -f query="query(\$owner: String!, \$repo: String!, \$branch: String!) { repository(owner: \$owner, name: \$repo) { ref(qualifiedName: \$branch) { target { ... on Commit { statusCheckRollup { contexts(first: 100) { nodes { __typename ... on CheckRun { status conclusion } ... on StatusContext { state } } } } } } } } }" \
                    2>/dev/null > "$5.tmp.$$" && mv "$5.tmp.$$" "$5"
            ' _ "$ci_lock" "$repo_owner" "$repo_name" "$branch" "$ci_cache_file" </dev/null >/dev/null 2>&1 &
            disown 2>/dev/null || true
        fi
    fi

    if [[ -n "$ci_json" ]]; then
        # Reshape into {statusCheckRollup: [...]} then apply the shared CI rollup program.
        ci_summary=$(printf '%s' "$ci_json" \
            | jq '{statusCheckRollup: (.data.repository.ref.target.statusCheckRollup.contexts.nodes // [])}' 2>/dev/null \
            | jq -r "$JQ_CI_ROLLUP" 2>/dev/null)
        read -r ci_state ci_failed ci_running ci_total <<< "$ci_summary"
    fi

    # Render onto the right side. With no PR the right side starts empty, so the
    # CI badge becomes the only token; that keeps line 1 the source of truth.
    # Pass is silent; only failing/running CI surfaces.
    case "$ci_state" in
        fail)    right="${right:+${right} }${RED}CI ✗ ${ci_failed}/${ci_total}${RESET}" ;;
        running) right="${right:+${right} }${YELLOW}CI ● ${ci_running}/${ci_total}${RESET}" ;;
    esac
fi

# ── Render line 1 with right-aligned PR info ──

term_width="${COLUMNS:-120}"
left_len=$(visible_len "$left")
right_len=$(visible_len "$right")

pad=$(( term_width - left_len - right_len ))
[[ "$pad" -lt 1 ]] && pad=1

if [[ -n "$right" ]]; then
    printf '%b%*s%b\n' "$left" "$pad" "" "$right"
else
    printf '%b\n' "$left"
fi

# ╔══════════════════════════════════════════════════════════════════════════════╗
# ║  LINE 2: Model state + session economics                                     ║
# ║                                                                              ║
# ║  Opus 4.8 (high) | Ctx [███░░░] 38% | 5h 23% (2h left) | $0.42 | Cache 99%    ║
# ║                                                                              ║
# ║  Each section is a small render_* function (easy to disable / re-order).     ║
# ╠══════════════════════════════════════════════════════════════════════════════╣

# Effort + thinking come straight from the JSON input (.effort.level / .thinking.enabled).
effort="${json_effort:-}"
thinking="${json_thinking:-}"

# ── Section renderers ──
# Each emits one composite segment or nothing, gating on its SHOW_* flag + data.
# Composition into line 2 happens at the bottom via append_raw.

render_model() {
    [[ "$SHOW_MODEL" == true && -n "$model" ]] || return 0
    local extras="$effort"
    [[ "$thinking" == "true" ]] && extras="${extras:+${extras}, }thinking"
    local label="${ORANGE}${model}${RESET}"
    [[ -n "$extras" ]] && label="${label} ${DIM}(${extras})${RESET}"
    printf '%s' "$label"
}

render_context() {
    [[ "$SHOW_CONTEXT" == true && -n "$used" ]] || return 0
    local pct_int; pct_int=$(printf '%.0f' "$used")
    local color; color="$(ctx_color "$used")"
    local bar; bar="$(ctx_bar "$used" "$CTX_BAR_WIDTH")"
    local out="${DIM}Ctx${RESET} ${color}[${bar}] ${pct_int}%${RESET}"
    # Raw token count alongside the percentage, when the JSON carries it
    # (absent before the first API call and briefly after /compact).
    if [[ -n "$ctx_total_tokens" ]]; then
        local tok_str; tok_str="$(fmt_tokens "$ctx_total_tokens")"
        if [[ -n "$ctx_window_size" ]]; then
            out="${out} ${DIM}(${tok_str}/$(fmt_tokens "$ctx_window_size"))${RESET}"
        else
            out="${out} ${DIM}(${tok_str})${RESET}"
        fi
    fi
    # Compaction proximity arrow only surfaces in the warning zone (>65%).
    local gap; gap="$(compact_gap "$used")"
    [[ "${pct_int:-0}" -le 65 ]] && gap=""
    if [[ -n "$gap" ]]; then
        if [[ "$gap" -eq 0 ]]; then
            out="${out} ${DIM}→${RESET} ${RED}COMPACTING NEXT TURN${RESET}"
        else
            out="${out} ${DIM}→${RESET} ${color}${gap}%${RESET} ${DIM}to compact${RESET}"
        fi
    fi
    # Context-rot alert: independent of the compaction arrow above, fires on
    # raw token count so it still warns on a 1M-context model far from its
    # own autocompact trigger.
    if [[ -n "$(context_rot_warning "$ctx_total_tokens")" ]]; then
        out="${out} ${DIM}·${RESET} ${RED}⚠ context rot risk${RESET}"
    fi
    printf '%s' "$out"
}

render_session_age() {
    [[ "$SHOW_SESSION_AGE" == true && -n "${session_id:-}" ]] || return 0
    local start_file="$HOME/.claude/runtime/${session_id}/start-ts"
    [[ -s "$start_file" ]] || return 0
    local start; start=$(cat "$start_file" 2>/dev/null || echo 0)
    [[ "$start" -gt 0 ]] || return 0
    local age=$(( $(date +%s) - start ))
    [[ "$age" -lt 600 ]] && return 0   # hide under 10 minutes
    printf '%s' "${DIM}Up${RESET} ${GREEN}$(fmt_age "$age")${RESET}"
}

render_cache_ratio() {
    [[ "$SHOW_CACHE_RATIO" == true ]] || return 0
    [[ -n "$cache_create" || -n "$cache_read" ]] || return 0
    local pct; pct="$(cache_hit_pct "${cache_create:-0}" "${cache_read:-0}")"
    [[ -z "$pct" ]] && return 0
    local color; color="$(cache_color "$pct")"
    printf '%s' "${DIM}Cache${RESET} ${color}${pct}%${RESET}"
}

render_cost() {
    [[ -n "$cost_usd" ]] || return 0
    awk -v c="$cost_usd" 'BEGIN { exit (c + 0 > 0) ? 0 : 1 }' 2>/dev/null || return 0
    local est_cost; est_cost=$(printf '%.4f' "$cost_usd")
    local out="${ORANGE}\$${est_cost}${RESET}"   # $ is the cue; no label
    if [[ -n "$wall_ms" && "$wall_ms" -gt 0 ]]; then
        local rate; rate=$(cost_per_min "$cost_usd" "$wall_ms")
        [[ -n "$rate" ]] && out="${out} ${DIM}(\$${rate}/min)${RESET}"
    fi
    printf '%s' "$out"
}

# Always-on 5h quota: used % colored by threshold, plus time until the window resets.
render_rate_5h() {
    [[ "$SHOW_RATE_LIMITS" == true && -n "$rl_5h" ]] || return 0
    local out
    out="${DIM}5h${RESET} $(rl_color "$rl_5h")$(printf '%.0f' "$rl_5h")%${RESET}"
    if [[ -n "$rl_5h_reset" ]]; then
        local left=$(( rl_5h_reset - $(date +%s) ))
        [[ "$left" -gt 0 ]] && out="${out} ${DIM}($(fmt_age "$left") left)${RESET}"
    fi
    printf '%s' "$out"
}

# Trailing 7d quota: surfaces only once it crosses 50% (5h has its own segment).
render_rate_7d() {
    [[ "$SHOW_RATE_LIMITS" == true && -n "$rl_7d" ]] || return 0
    awk -v v="${rl_7d:-0}" 'BEGIN { exit (v + 0 > 50) ? 0 : 1 }' 2>/dev/null || return 0
    printf '%s' "${DIM}Rate 7d:${RESET} $(rl_color "$rl_7d")$(printf '%.0f' "$rl_7d")%${RESET}"
}

# ── Compose line 2 and line 3 ──
# Line 2: model · ctx bar. Line 3: cost · cache · 5h quota · age · 7d-rate,
# 5h quota kept next to age since both are session-clock reads at a glance.
# Split across two lines so line 2 never runs long enough to wrap in a narrow
# terminal. Secondary segments self-suppress; empties are skipped so
# separators never double up.
line_2=""
for seg in \
    "$(render_model)" \
    "$(render_context)"; do
    [[ -n "$seg" ]] && append_raw line_2 "$seg"
done
[[ -n "$line_2" ]] && printf '%b\n' "$line_2"

line_3=""
for seg in \
    "$(render_cost)" \
    "$(render_cache_ratio)" \
    "$(render_rate_5h)" \
    "$(render_session_age)" \
    "$(render_rate_7d)"; do
    [[ -n "$seg" ]] && append_raw line_3 "$seg"
done
[[ -n "$line_3" ]] && printf '%b\n' "$line_3"
exit 0
fi
