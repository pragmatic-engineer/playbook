# SPDX-FileCopyrightText: 2026 Igor Santos
# SPDX-License-Identifier: MIT
# shellcheck shell=bash
# (sourced, so it has no shebang; the directive tells shellcheck the dialect)
#
# worktree.sh: engine for the `cc worktree <branch>` / `ccd worktree`
# subcommand: creates/enters a git worktree with smart defaults, then cd's the
# current shell into it. Not exposed as a standalone command (the entry point is
# the private `_cc_worktree`, called only by the _claude dispatcher).
#
# Sourceable in bash and zsh (no zsh-only builtins or options).
#
# Features: a grouped worktree base dir (WORKTREE_BASE_DIR, default .worktrees)
# holding <repo>/<folder>, JIRA-key folder naming (PROJECT-1234-foo ->
# PROJECT-1234/), .env copy, upstream tracking, node_modules hardlink reuse,
# base-branch rebase, and a daily-rate-limited background cleanup of
# merged/stale worktrees.
#
# Flags: --ai-resolve (or WORKTREE_AI_RESOLVE=1) lets Claude resolve rebase
# conflicts; otherwise a conflict just aborts the rebase.
#
# Test seam: set _WT_NO_LAUNCH=1 to skip the session launch after setup.

# Default prompt sent to claude when auto-resolving rebase conflicts.
# Override before sourcing: _WT_AI_RESOLVE_PROMPT="your prompt"
# Quoted so an override containing * or ? is not glob-expanded against the cwd
# before : discards it. The assigned value is correct either way, so this is
# wasted work rather than a wrong prompt, but the cwd here is a repo checkout.
: "${_WT_AI_RESOLVE_PROMPT:="Resolve the current git rebase conflicts. Run 'git diff --name-only --diff-filter=U' to find conflicted files, read each, fix the markers, 'git add' them, then 'git rebase --continue'. Keep both sides where intent is clear; if ambiguous, prefer the incoming (origin/\$BASE_REF) version."}"

_wt_die() { printf '%s\n' "worktree: $1" >&2; return "${2:-1}"; }

# Decide the conflict action once AI-resolve is on and claude is available.
# $1 silent(0/1)  $2 is_tty(0/1)  $3 answer(raw read)  ->  spawn|abort
_wt_ai_resolve_decision() {
    (( $1 )) && { printf '%s\n' spawn; return; }
    if (( $2 )); then
        case "$3" in ""|[Yy]|[Yy][Ee][Ss]) printf '%s\n' spawn ;; *) printf '%s\n' abort ;; esac
    else
        printf '%s\n' abort
    fi
}

# Print an info block (to stderr) describing what auto-resolve will do.
# $1 branch  $2 base (e.g. origin/main)  $3 conflicted file list
_wt_ai_resolve_info() {
    local branch="$1" base="$2" files="$3"
    printf '%s\n' "--- worktree: rebase conflict ---" >&2
    printf '%s\n' "  Branch : $branch" >&2
    printf '%s\n' "  Base   : $base" >&2
    printf '%s\n' "  Files  : $files" >&2
    printf '%s\n' "Auto-resolve will: spawn 'claude -p --model haiku', read each conflicted" >&2
    printf '%s\n' "  file, fix markers, run 'git add', then 'git rebase --continue'." >&2
    printf '%s\n' "  When ambiguous, prefer origin/<base> (incoming) version." >&2
    printf '%s\n' "---------------------------------" >&2
}

_wt_help() {
    printf '%s\n' 'cc worktree <branch> [env-base-folder]   (also ccd worktree)'
    printf '%s\n' '  Create or enter a git worktree for <branch> and start a session in it.'
    printf '%s\n' '  Folder name is the JIRA key in the branch, else the branch leaf.'
    printf '%s\n' '  Worktrees live in <repo-parent>/<base>/<repo>/<folder> (base = WORKTREE_BASE_DIR, default .worktrees).'
    printf '%s\n' '  Claude auto-resolves rebase conflicts on your own branches (WORKTREE_AI_RESOLVE=0 disables).'
    printf '%s\n' '  --no-push   Skip auto-creating the remote branch (env: WORKTREE_NO_PUSH=1).'
    printf '%s\n' '              Run "git push -u <remote> <branch>" manually when ready.'
}

# md5 (macOS) | md5sum (Linux) -> short stable hash of a string
_wt_hash() { printf '%s\n' "$1" | md5 -q 2>/dev/null || md5sum <<< "$1" | cut -c1-8; }

# Detect base branch as a remote-tracking ref, e.g. origin/main
_wt_base_branch() {
    local base
    base="$(git symbolic-ref --quiet --short refs/remotes/origin/HEAD 2>/dev/null | sed 's#^origin/##')"
    if [[ -z "$base" ]]; then
        local cand
        for cand in main master trunk develop; do
            if git show-ref --verify --quiet "refs/remotes/origin/$cand"; then base="$cand"; break; fi
        done
    fi
    printf '%s\n' "origin/${base:-master}"
}

# Resolve the base directory that holds this repo's worktrees: <base>/<repo>,
# where <base> is WORKTREE_BASE_DIR (default .worktrees). A relative base sits
# under the repo's parent; an absolute base is used as-is. The <repo> leaf keeps
# same-named branches in sibling repos from colliding.
# Args: 1=repo root, 2=repo parent. Echoes the resolved directory.
_wt_resolve_base() {
    local repo_root="$1" repo_parent="$2"
    local repo_name="${repo_root##*/}"
    local base="${WORKTREE_BASE_DIR:-.worktrees}"
    [[ "$base" == "." ]] && base=".worktrees"   # never collapse into the repo root
    if [[ "$base" == /* ]]; then
        printf '%s\n' "$base/$repo_name"
    else
        printf '%s\n' "$repo_parent/$base/$repo_name"
    fi
}

# Create a worktree, recovering from prune/repair if needed. Echoes the path.
_wt_create_worktree() {
    local dest="$1" ref="$2" new_branch="${3:-}"
    local -a cmd_args=()
    [[ -n "$new_branch" ]] && cmd_args+=(-b "$new_branch")
    cmd_args+=("$dest" "$ref")

    if ! git worktree add "${cmd_args[@]}" >/dev/null 2>&1; then
        git worktree prune  >/dev/null 2>&1 || true
        git worktree repair >/dev/null 2>&1 || true
        if ! git worktree add -f "${cmd_args[@]}" >/dev/null 2>&1; then
            local existing
            existing="$(git worktree list --porcelain | awk -v b="refs/heads/${new_branch:-$ref}" '$1=="worktree"{w=$2} $1=="branch"&&$2==b{print w}')"
            if [[ -n "$existing" && -d "$existing" ]]; then
                printf '%s\n' "branch already at $existing" >&2
                printf '%s\n' "$existing"
                return 0
            fi
            printf '%s\n' "git worktree add failed" >&2
            return 3
        fi
    fi
    printf '%s\n' "$dest"
}

# Correct upstream tracking (push to create the remote branch if missing)
_wt_setup_upstream() {
    local current expected="$REMOTE/$BRANCH"
    current="$(git rev-parse --abbrev-ref --symbolic-full-name '@{u}' 2>/dev/null || echo "")"
    [[ "$current" == "$expected" ]] && return 0
    if git ls-remote --heads "$REMOTE" "$BRANCH" | grep -q .; then
        git branch --set-upstream-to="$expected" "$BRANCH" >/dev/null 2>&1 || true
    elif (( NO_PUSH )); then
        return 0
    else
        git push -u "$REMOTE" "$BRANCH" || _wt_die "failed to create upstream" 6
    fi
}

# Find the folder (relative to REPO_ROOT, or ".") that holds a .env
_wt_find_env_base() {
    local base_arg="${1:-}"
    if [[ -n "$base_arg" ]]; then
        [[ "$base_arg" == "." && -f "$REPO_ROOT/.env" ]] && { printf '%s\n' "."; return; }
        [[ -f "$REPO_ROOT/$base_arg/.env" ]] && { printf '%s\n' "$base_arg"; return; }
    else
        [[ -f "$REPO_ROOT/.env" ]] && { printf '%s\n' "."; return; }
        local env_path
        env_path="$(find "$REPO_ROOT" -mindepth 2 -maxdepth 2 -type f -name .env 2>/dev/null | head -n1)"
        [[ -n "$env_path" ]] && { basename "$(dirname "$env_path")"; return; }
    fi
    printf '%s\n' ""
}

# Copy .env into a destination worktree (no-clobber). Only copies when the .env
# is gitignored in the source repo, so a `git add` inside the worktree can't
# stage secrets that were never meant to be tracked.
_wt_copy_env() {
    local dest="$1"
    [[ -z "$ENV_BASE" ]] && return 0
    local rel
    if [[ "$ENV_BASE" == "." ]]; then rel=".env"; else rel="$ENV_BASE/.env"; fi
    local src="$REPO_ROOT/$rel"
    [[ -f "$src" ]] || return 0
    if ! git -C "$REPO_ROOT" check-ignore -q "$rel" 2>/dev/null; then
        printf '%s\n' "worktree: $rel is not gitignored in the source repo; skipping .env copy to avoid staging secrets." >&2
        return 0
    fi
    [[ "$ENV_BASE" == "." ]] || mkdir -p "$dest/$ENV_BASE" 2>/dev/null || true
    cp -n "$src" "$dest/$rel" 2>/dev/null || true
}

# Reuse node_modules via hardlinks when the lockfile matches
_wt_node_modules() {
    [[ -f "package.json" ]] || return 0
    [[ -f "$REPO_ROOT/package-lock.json" ]] || return 0
    [[ -d "$REPO_ROOT/node_modules" ]] || return 0

    local -a _hasher_cmd=()
    command -v sha256sum >/dev/null && _hasher_cmd=(sha256sum)
    { command -v shasum >/dev/null && [[ ${#_hasher_cmd[@]} -eq 0 ]]; } && _hasher_cmd=(shasum -a 256)
    [[ ${#_hasher_cmd[@]} -eq 0 ]] && return 0

    [[ -f "package-lock.json" ]] || cp "$REPO_ROOT/package-lock.json" "./package-lock.json" 2>/dev/null || true

    local base_hash here_hash
    base_hash="$("${_hasher_cmd[@]}" < "$REPO_ROOT/package-lock.json" | awk '{print $1}')"
    here_hash="$("${_hasher_cmd[@]}" < "package-lock.json" | awk '{print $1}')"
    [[ "$base_hash" != "$here_hash" ]] && return 0

    [[ -e "node_modules" ]] && rm -rf "node_modules"
    cp -cR "$REPO_ROOT/node_modules" "node_modules" 2>/dev/null \
      || { rm -rf "node_modules"; cp -R --reflink=auto "$REPO_ROOT/node_modules" "node_modules" 2>/dev/null; } \
      || { rm -rf "node_modules"; cp -R "$REPO_ROOT/node_modules" "node_modules"; }
    printf '%s\n' "Cloned node_modules (copy-on-write)" >&2
    command -v npm >/dev/null && { npm install --prefer-offline --no-audit --no-fund >&2 2>&1 || true; }
}

# Remove worktrees whose branch is merged or whose last commit is >30 days old.
# Daily-rate-limited; skips open-PR branches, in-use dirs, and the new TARGET.
_wt_cleanup_stale() {
    local cache
    cache="/tmp/.git-wt-cleanup-$(_wt_hash "$REPO_ROOT")"
    local age=$(( $(date +%s) - $(stat -f%m "$cache" 2>/dev/null || stat -c %Y "$cache" 2>/dev/null || echo 0) ))
    (( age < 86400 )) && return 0
    touch "$cache"

    cd "$REPO_ROOT" || return 0

    local base_branch cutoff_epoch merged open_prs=""
    base_branch="$(_wt_base_branch)"
    cutoff_epoch="$(date -v-30d +%s 2>/dev/null || date -d '30 days ago' +%s)"
    merged="$(git branch --merged "$base_branch" 2>/dev/null | sed 's/^[[:space:]*+]*//')"
    if command -v gh >/dev/null 2>&1; then
        open_prs="$(gh pr list --state open --author '@me' --limit 200 --json headRefName --jq '.[].headRefName' 2>/dev/null || echo "")"
    fi

    git worktree list --porcelain | awk '$1=="worktree" && c++>0{print $2}' | while read -r wt_path; do
        [[ "$wt_path" == "$TARGET" ]] && continue
        if command -v lsof >/dev/null 2>&1 && lsof -d cwd +c0 2>/dev/null | grep -qF "$wt_path"; then continue; fi

        local wt_branch
        wt_branch="$(git -C "$wt_path" rev-parse --abbrev-ref HEAD 2>/dev/null || echo "")"
        [[ -z "$wt_branch" ]] && continue
        if [[ -n "$open_prs" ]] && printf '%s\n' "$open_prs" | grep -qxF "$wt_branch"; then continue; fi

        local dominated=0
        if printf '%s\n' "$merged" | grep -qxF "$wt_branch"; then dominated=1; fi
        if (( ! dominated )); then
            local commit_epoch
            commit_epoch="$(git -C "$wt_path" log -1 --format='%ct' 2>/dev/null || echo 0)"
            (( commit_epoch < cutoff_epoch )) && dominated=1
        fi

        if (( dominated )); then
            git worktree remove --force "$wt_path" 2>/dev/null || continue
            [[ "$wt_branch" != "HEAD" ]] && git branch -D "$wt_branch" 2>/dev/null || true
        fi
    done

    git worktree prune 2>/dev/null || true
}

# Pop the main-worktree auto-stash, if we took one.
# Use `git -C` rather than `cd`: this runs after _wt_main has cd'd the shell into
# the new worktree, and cd-ing back to main here would strand the shell there, so
# the session would start on the base branch instead of the new one.
_wt_restore_stash() {
    (( STASH_APPLIED )) || return 0
    git -C "$MAIN_WORKTREE" stash pop --quiet 2>/dev/null || true
}

# The body: everything that can fail with `return`. Dynamic scoping lets it and
# the helpers above read worktree()'s locals (REMOTE, BRANCH, REPO_ROOT, ...).
_wt_main() {
    git rev-parse --is-inside-work-tree >/dev/null 2>&1 || { _wt_die "not a git repository" 10; return $?; }

    MAIN_WORKTREE="$(git worktree list --porcelain | awk '$1=="worktree" && !f{print $2; f=1}')"
    cd "$MAIN_WORKTREE" || { _wt_die "couldn't cd to main worktree: $MAIN_WORKTREE" 10; return $?; }

    [[ -z "$BRANCH" ]] && { _wt_die "usage: worktree <branch-name> [env-base-folder]" 2; return $?; }
    BRANCH="$(printf '%s' "$BRANCH" | tr -d '\r' | sed -E 's/^[[:space:]]+//; s/[[:space:]]+$//')"
    git check-ref-format --branch "$BRANCH" >/dev/null 2>&1 || { _wt_die "invalid branch name: '$BRANCH'" 2; return $?; }

    REPO_ROOT="$(git rev-parse --show-toplevel)"
    REPO_PARENT="$(dirname "$REPO_ROOT")"
    WT_ROOT="$(_wt_resolve_base "$REPO_ROOT" "$REPO_PARENT")"
    mkdir -p "$WT_ROOT" 2>/dev/null || true

    FETCH_CACHE="/tmp/.git-fetch-$(_wt_hash "$REPO_ROOT")"
    BASE_REF="$(git symbolic-ref --quiet --short refs/remotes/origin/HEAD 2>/dev/null | sed 's#^origin/##')"
    [[ -z "$BASE_REF" ]] && BASE_REF="master"

    git fetch "$REMOTE" --quiet "$BASE_REF" "$BRANCH" 2>/dev/null \
      || git fetch "$REMOTE" --quiet "$BASE_REF" 2>/dev/null || true

    # Auto-stash a dirty main worktree (restored by _wt_restore_stash after return)
    if ! git diff-index --quiet HEAD -- 2>/dev/null || ! git diff --quiet 2>/dev/null; then
        git stash push -m "worktree: auto-stash" --quiet 2>/dev/null && STASH_APPLIED=1
    fi

    JIRA_KEY="$(printf '%s' "$BRANCH" | grep -oiE '[A-Z]{2,}-[0-9]+' | head -1 | tr '[:lower:]' '[:upper:]' || true)"
    FOLDER="${JIRA_KEY:-${BRANCH##*/}}"
    TARGET="$WT_ROOT/$FOLDER"

    # Disambiguate: a JIRA-key folder already on a different branch -> use the leaf
    if [[ -d "$TARGET" && -n "$JIRA_KEY" && "$FOLDER" == "$JIRA_KEY" ]]; then
        local in_use
        in_use="$(git worktree list --porcelain | awk -v t="$TARGET" '$1=="worktree"{w=$2} w==t && $1=="branch"{sub("refs/heads/","",$2); print $2}')"
        if [[ -n "$in_use" && "$in_use" != "$BRANCH" ]]; then
            FOLDER="${BRANCH##*/}"; TARGET="$WT_ROOT/$FOLDER"
            printf '%s\n' "Worktree $JIRA_KEY in use by '$in_use'. Using '$FOLDER' instead." >&2
        fi
    fi

    ENV_BASE="$(_wt_find_env_base "$ENV_BASE_ARG")"

    local worktree_path current_branch is_registered existing
    if [[ -d "$TARGET" ]]; then
        current_branch="$(git worktree list --porcelain | awk -v t="$TARGET" '$1=="worktree"{w=$2} w==t && $1=="branch"{sub("refs/heads/","",$2); print $2}')"
        is_registered="$(git worktree list --porcelain | awk -v t="$TARGET" '$1=="worktree" && $2==t{print "yes"}')"

        if [[ -z "$current_branch" && "$is_registered" == "yes" ]]; then
            printf '%s\n' "Worktree at $TARGET has detached HEAD. Recovering..." >&2
            git -C "$TARGET" rebase --abort 2>/dev/null || true
            git -C "$TARGET" merge  --abort 2>/dev/null || true
            git -C "$TARGET" checkout "$BRANCH" 2>/dev/null || {
                git show-ref --verify --quiet "refs/remotes/$REMOTE/$BRANCH" \
                  && git -C "$TARGET" checkout -b "$BRANCH" "refs/remotes/$REMOTE/$BRANCH" 2>/dev/null || true
            }
            current_branch="$(git -C "$TARGET" rev-parse --abbrev-ref HEAD 2>/dev/null || echo "")"
            if [[ "$current_branch" == "HEAD" ]]; then
                printf '%s\n' "Recovery failed. Removing and recreating worktree..." >&2
                git worktree remove --force "$TARGET" 2>/dev/null || { rm -rf "$TARGET"; git worktree prune 2>/dev/null; }
                current_branch=""; is_registered=""
            fi
        fi

        if [[ -z "$current_branch" && "$is_registered" != "yes" ]]; then
            printf '%s\n' "Orphaned directory at $TARGET (not a registered worktree). Cleaning up..." >&2
            git worktree prune >/dev/null 2>&1 || true
            rm -rf "$TARGET"
            worktree_path="$(_wt_make "$TARGET")" || return $?
            _wt_copy_env "$worktree_path"
        elif [[ "$current_branch" == "$BRANCH" ]]; then
            git fetch "$REMOTE" "$BRANCH" --quiet 2>/dev/null || true
            git show-ref --verify --quiet "refs/remotes/$REMOTE/$BRANCH" 2>/dev/null \
              && git -C "$TARGET" pull --ff-only --quiet 2>/dev/null || true
            _wt_copy_env "$TARGET"
            worktree_path="$TARGET"
        else
            if git ls-remote --heads "$REMOTE" "$current_branch" 2>/dev/null | grep -q .; then
                _wt_die "worktree at $TARGET is on branch '$current_branch' which still exists on remote. Finish it or remove the worktree first." 5
                return $?
            fi
            printf '%s\n' "Previous branch '$current_branch' merged/deleted on remote. Recycling worktree for '$BRANCH'..." >&2
            git worktree remove --force "$TARGET" >/dev/null 2>&1 || { rm -rf "$TARGET"; git worktree prune >/dev/null 2>&1 || true; }
            git branch -D "$current_branch" >/dev/null 2>&1 || true
            worktree_path="$(_wt_make "$TARGET")" || return $?
            _wt_copy_env "$worktree_path"
        fi
    else
        existing="$(git worktree list --porcelain | awk -v b="refs/heads/$BRANCH" '$1=="worktree"{w=$2} $1=="branch"&&$2==b{print w}')"
        if [[ -n "$existing" && -d "$existing" ]]; then
            git fetch "$REMOTE" "$BRANCH" --quiet 2>/dev/null || true
            git show-ref --verify --quiet "refs/remotes/$REMOTE/$BRANCH" 2>/dev/null \
              && git -C "$existing" pull --ff-only --quiet 2>/dev/null || true
            _wt_copy_env "$existing"
            worktree_path="$existing"
        else
            [[ -n "$existing" && ! -d "$existing" ]] && { printf '%s\n' "Stale worktree for '$BRANCH' at $existing (missing). Pruning..." >&2; git worktree prune >/dev/null 2>&1 || true; }
            worktree_path="$(_wt_make "$TARGET")" || return $?
            _wt_copy_env "$worktree_path"
        fi
    fi

    cd "$worktree_path" || { _wt_die "couldn't cd to $worktree_path" 4; return $?; }

    # If somehow detached, attach to the branch
    if [[ "$(git rev-parse --abbrev-ref HEAD 2>/dev/null)" == "HEAD" ]]; then
        printf '%s\n' "Worktree created detached. Attaching to $BRANCH..." >&2
        git checkout -B "$BRANCH" HEAD 2>/dev/null || git checkout "$BRANCH" 2>/dev/null || true
    fi

    _wt_maybe_rebase

    local worktree_dir; worktree_dir="$(pwd)"
    (( NO_PUSH )) && printf '%s\n' "worktree: --no-push set; will not auto-create the remote branch. Run 'git push -u $REMOTE $BRANCH' when ready." >&2
    printf '%s\n' "Ready: $worktree_dir" >&2

    # Background: full fetch, upstream, sync, housekeeping, cleanup
    (
        git fetch "$REMOTE" --prune --quiet 2>/dev/null && touch "$FETCH_CACHE" || true
        local expected="$REMOTE/$BRANCH" cur
        cur="$(git rev-parse --abbrev-ref --symbolic-full-name '@{u}' 2>/dev/null || echo "")"
        [[ "$cur" != "$expected" ]] && git show-ref --verify --quiet "refs/remotes/$expected" 2>/dev/null \
          && git branch --set-upstream-to="$expected" "$BRANCH" >/dev/null 2>&1 || true
        if git show-ref --verify --quiet "refs/remotes/$expected" 2>/dev/null; then
            git pull --ff-only --quiet 2>/dev/null || true
        fi
        git worktree prune 2>/dev/null || true
        _wt_setup_upstream
        _wt_node_modules
        _wt_cleanup_stale
    ) </dev/null >/dev/null 2>&1 &
    disown 2>/dev/null || true
}

# Create a worktree picking the right ref source (local branch / remote / base)
_wt_make() {
    local target="$1" wt_path
    if git show-ref --verify --quiet "refs/heads/$BRANCH"; then
        wt_path="$(_wt_create_worktree "$target" "$BRANCH")" || return $?
    elif git show-ref --verify --quiet "refs/remotes/$REMOTE/$BRANCH"; then
        wt_path="$(_wt_create_worktree "$target" "refs/remotes/$REMOTE/$BRANCH" "$BRANCH")" || return $?
    else
        wt_path="$(_wt_create_worktree "$target" "$(_wt_base_branch)" "$BRANCH")" || return $?
        git branch --unset-upstream "$BRANCH" 2>/dev/null || true
    fi
    printf '%s\n' "$wt_path"
}

# Rebase own branches onto latest base. Conflicts abort unless --ai-resolve.
_wt_maybe_rebase() {
    local current_branch git_user branch_author gh_user is_own=0
    current_branch="$(git rev-parse --abbrev-ref HEAD 2>/dev/null)"
    git_user="$(git config user.name 2>/dev/null || echo "")"
    branch_author="$(git log -1 --format='%an' "$current_branch" 2>/dev/null || echo "")"
    gh_user="$(gh api user --jq '.login' 2>/dev/null || echo "")"

    # Protected/shared branches never auto-rebase, even if you authored the last
    # commit on them. Closes the "any slash-free branch is mine" hole that would
    # rebase develop/release/hotfix onto base.
    case "$current_branch" in
        main|master|trunk|develop|dev|staging|release|release/*|hotfix|hotfix/*) return 0 ;;
    esac

    if [[ -n "$git_user" && "$branch_author" == "$git_user" ]]; then is_own=1
    elif [[ -n "$gh_user" && "$BRANCH" == *"$gh_user"* ]]; then is_own=1; fi

    (( is_own )) || return 0
    [[ "$current_branch" == "$BASE_REF" || "$current_branch" == "HEAD" ]] && return 0

    git fetch "$REMOTE" "$BASE_REF" --quiet 2>/dev/null || true
    git merge-base --is-ancestor "$REMOTE/$BASE_REF" HEAD 2>/dev/null && return 0

    local -a rebase_args=()
    rebase_args=("$REMOTE/$BASE_REF" --quiet)
    git log --merges --oneline "$REMOTE/$BASE_REF..HEAD" 2>/dev/null | grep -q . && rebase_args+=(--rebase-merges)

    if ! git rebase "${rebase_args[@]}" 2>/dev/null; then
        if (( AI_RESOLVE )) && command -v claude >/dev/null 2>&1; then
            local conflicted; conflicted="$(git diff --name-only --diff-filter=U 2>/dev/null)"
            _wt_ai_resolve_info "$current_branch" "$REMOTE/$BASE_REF" "$conflicted"
            local silent=0 is_tty=0 answer=""
            [[ -n "${WORKTREE_AI_RESOLVE_SILENT:-}" && "$WORKTREE_AI_RESOLVE_SILENT" != "0" ]] && silent=1
            [[ -t 0 ]] && is_tty=1
            if (( ! silent && is_tty )); then
                printf '%s' "Run auto-resolve now? [Y/n] " >&2
                read -r answer
            fi
            case "$(_wt_ai_resolve_decision "$silent" "$is_tty" "$answer")" in
                spawn) command claude -p --model haiku "$_WT_AI_RESOLVE_PROMPT" 2>/dev/null \
                         || { printf '%s\n' "Auto-resolve failed. Aborting rebase." >&2; git rebase --abort 2>/dev/null || true; } ;;
                abort) printf '%s\n' "Skipped auto-resolve. Aborting rebase; re-run with the branch checked out to retry." >&2; git rebase --abort 2>/dev/null || true ;;
            esac
        else
            printf '%s\n' "Rebase conflict on $current_branch onto $REMOTE/$BASE_REF. Aborting (pass --ai-resolve to let Claude fix it)." >&2
            git rebase --abort 2>/dev/null || true
        fi
    fi

    # Never leave a detached HEAD behind
    if [[ "$(git rev-parse --abbrev-ref HEAD 2>/dev/null)" == "HEAD" ]]; then
        printf '%s\n' "HEAD detached after rebase. Aborting and restoring." >&2
        git rebase --abort 2>/dev/null || true
        git checkout "$current_branch" 2>/dev/null || git checkout "$BRANCH" 2>/dev/null || true
    fi
}

# Private entry point (called only by the _claude dispatcher's worktree case).
# Parses flags, then runs the body and restores any auto-stash.
_cc_worktree() {
    local REMOTE="origin" BRANCH="" ENV_BASE_ARG="" AI_RESOLVE=0 NO_PUSH=0
    local REPO_ROOT REPO_PARENT WT_ROOT MAIN_WORKTREE BASE_REF JIRA_KEY FOLDER TARGET ENV_BASE FETCH_CACHE
    local STASH_APPLIED=0 _wt_origin="$PWD" rc=0

    local a
    for a in "$@"; do
        case "$a" in
            --ai-resolve) AI_RESOLVE=1 ;;
            --no-push)    NO_PUSH=1 ;;
            -h|--help)    _wt_help; return 0 ;;
            --)           ;;
            -*)           _wt_die "unknown option: $a" 2; return $? ;;
            *)            if [[ -z "$BRANCH" ]]; then BRANCH="$a"; elif [[ -z "$ENV_BASE_ARG" ]]; then ENV_BASE_ARG="$a"; fi ;;
        esac
    done
    # WORKTREE_AI_RESOLVE overrides the flag either way: set to 0 to force-disable
    # even on the `cc worktree` path (which always passes --ai-resolve); any other
    # non-empty value enables it without the flag.
    if [[ -n "${WORKTREE_AI_RESOLVE:-}" ]]; then
        if [[ "$WORKTREE_AI_RESOLVE" == "0" ]]; then AI_RESOLVE=0; else AI_RESOLVE=1; fi
    fi
    # WORKTREE_NO_PUSH mirrors the --no-push flag via the environment.
    [[ -n "${WORKTREE_NO_PUSH:-}" && "$WORKTREE_NO_PUSH" != "0" ]] && NO_PUSH=1

    printf '%s\n' "worktree: setting up '$BRANCH'..." >&2
    _wt_main; rc=$?
    _wt_restore_stash
    (( rc )) && { cd "$_wt_origin" 2>/dev/null || true; return $rc; }
    # Test seam: when _WT_NO_LAUNCH is set, worktree is ready but session launch
    # is skipped. The dispatcher also checks this before calling claude.
    [[ -n "${_WT_NO_LAUNCH:-}" ]] && return 0
    return $rc
}
