---
description: Use when committing staged changes with a generated message and pushing. Handles staging, formatting, a signed commit, optional rebase, and push.
allowed-tools: Bash, Read, Skill
argument-hint: "[--all|-A] [--update|-u] [--amend|-a] [--no-signoff] [--auto] [--ask]"
context: fork
agent: git
---

# Commit and Push

Generate a commit message from the staged diff, commit, optionally rebase, and push. One flow.

## Run this now

Execute the steps below immediately, end to end, running every bash block for real with the `Bash` tool (capital B, tool names are case-sensitive). Do **not** narrate a plan, summarize `git status`, offer a numbered menu, or ask "what would you like me to do?" / "proceed? [Y/n]". There is **no confirmation gate**.

The flow is one pass: stage (per flags), draft the commit message, commit, rebase if behind, push. Staging flags (`-A`, `-u`, `-a`) parse from `$ARGUMENTS`; no flags means commit only what is already staged.

This command is built to run in an isolated subagent (`context: fork`) so the diff and drafting stay out of the main context. When it forks, your final message is the only thing the main conversation sees, so end with a concise outcome summary (commit SHA, branch, and the generated message). If you are instead reading this in the main conversation, run it here exactly the same way; do not wait for a fork and do not defer to the user.

## Argument flags

Parse these from `$ARGUMENTS`. Each bash block below runs in its **own shell**, so a variable set in one step is NOT visible in a later one. Apply each flag by setting its variable to `true`/`false` at the top of the block that reads it (do not rely on an env var carried over from an earlier step):

- `--all` or `-A` → `STAGE_ALL=true` (run `git add -A`). Set at the top of **Step 1**.
- `--update` or `-u` → `STAGE_UPDATE=true` (run `git add -u`, tracked files only). Set at the top of **Step 1**.
- `--amend` or `-a` → `AMEND_COMMIT=true` (amend the previous commit). Set at the top of **Step 1** AND again in **Step 4** (both blocks read it).

- `--no-signoff` → `NO_SIGNOFF=true` (leave the `Signed-off-by` trailer off this commit, on purpose). Set at the top of **Step 4**.

Combined flags are fine: `-Au`, `-a -u`, etc. No flags means every variable stays `false`: commit only what is already staged. There is no confirmation gate; the flow always runs to completion.

## Execution rules

1. Run every bash block in this command for real with the `Bash` tool (capital B, tool names are case-sensitive). Do not simulate output.
2. Use the actual command output to drive the next step.
3. Do not assume file contents or git state; check them.
4. Combine independent bash operations into single tool calls.
5. Never run destructive git commands (`reset --hard`, `push --force`, `clean -f`) unless the user explicitly asks.
6. Never skip hooks (`--no-verify`, `--no-gpg-sign`). The `commit-message-sanitizer` hook also removes any AI attribution from a commit, tag, merge or PR message before it runs; it never blocks.
7. Never amend automatically: only when `AMEND_COMMIT=true`.
8. Pass commit messages via heredoc to preserve formatting, never `-m "..."` for multi-line.

## Step 0: Read the run mode

Do this first. Read the mode from the CLI:

```bash
playbook mode status --json
```

If the arguments contain `--auto`, add `--flag auto`. If they contain `--ask`, add `--flag ask`. If both are present, stop with one line: "--auto and --ask conflict; pass one." The JSON has four keys: `mode` (`ask` or `auto`), `source` (where it came from), `hook_mode` and `warning`. If `warning` is not empty, print it once. If the command fails, do not stop and do not retry. Run in ask mode and print one line. When the error is `unrecognized subcommand 'mode'`, the installed binary is older than this plugin: print "playbook binary is older than the plugin, run `playbook update`."

- **`ask` mode:** behave exactly as this file describes.
- **`auto` mode:** follow the Auto path below. Set `AUTO_MODE=true` at the top of the Step 4 block.

### Auto path

In auto mode this command never force-pushes and never uses a forced lease. When `git ls-remote --heads origin <branch>` shows the remote branch absent, push plainly: no force is needed. A push that would need a force (after an amend or a rebase in this run) stops and reports; nothing is pushed. The commit stays on the local branch, and a person decides how to publish it.

## Step 1: Stage, format, emit context

Run everything in a single bash block:

```bash
# Set each flag from $ARGUMENTS: true when the flag was passed, else false.
# -A -> STAGE_ALL, -u -> STAGE_UPDATE, -a -> AMEND_COMMIT. Default all false.
AMEND_COMMIT=false
STAGE_ALL=false
STAGE_UPDATE=false

# Hard stop on the repo's default/protected branch. This skill never commits
# there, under any flag combination, even -y: an ambiguous invocation like
# "branch off main" has been misread as "commit on main" before, and a
# maintainer/admin role can make a protected-branch ruleset bypass a push
# silently, with no visible error, so the ruleset is not a backstop
# (commit-skill-needs-explicit-branch). If work genuinely belongs directly on
# the default branch, that is a deliberate, informed decision to make with
# raw git, not this skill's automated default.
BRANCH=$(git rev-parse --abbrev-ref HEAD)
# A pipe's exit status is its LAST command's (sed's, here), not
# git symbolic-ref's, so `cmd | sed ... || fallback` never runs the
# fallback even when `cmd` failed and produced empty output: sed still
# exits 0 on nothing. Capture git's output first, THEN branch on whether
# it was empty, so a repo with no local origin/HEAD (never ran
# `git remote set-head origin -a`) doesn't silently end up with an empty
# DEFAULT_BRANCH instead of a real fallback value.
DEFAULT_BRANCH=$(git symbolic-ref refs/remotes/origin/HEAD 2>/dev/null)
if [ -n "$DEFAULT_BRANCH" ]; then
  DEFAULT_BRANCH="${DEFAULT_BRANCH#refs/remotes/origin/}"
else
  DEFAULT_BRANCH=$(gh repo view --json defaultBranchRef -q .defaultBranchRef.name 2>/dev/null || echo main)
fi
if [ "$BRANCH" = "$DEFAULT_BRANCH" ] || [ "$BRANCH" = "main" ] || [ "$BRANCH" = "master" ]; then
  echo "ERROR: HEAD is on '$BRANCH', the repo's default/protected branch. This skill never commits there. Create a feature branch first (e.g. git checkout -b <name>) and re-run." >&2
  exit 1
fi

# Auto-stage if requested
if [ "$STAGE_ALL" = "true" ]; then
  git add -A
elif [ "$STAGE_UPDATE" = "true" ]; then
  git add -u
fi

# Format staged files using whichever formatter the repo configures
STAGED_FILES=$(git diff --staged --name-only --diff-filter=d)
if [ "$AMEND_COMMIT" = "true" ]; then
  COMMIT_FILES=$(git diff --name-only --diff-filter=d HEAD~1 HEAD)
  FILES_TO_FORMAT=$(echo -e "$STAGED_FILES\n$COMMIT_FILES" | sort -u | grep -v '^$')
else
  FILES_TO_FORMAT="$STAGED_FILES"
fi
if [ -n "$FILES_TO_FORMAT" ]; then
  if [ -f "biome.json" ] || [ -f "biome.jsonc" ]; then
    echo "$FILES_TO_FORMAT" | xargs npx biome check --write 2>/dev/null || true
  elif [ -f "dprint.json" ] || [ -f "dprint.jsonc" ] || [ -f ".dprint.json" ]; then
    echo "$FILES_TO_FORMAT" | xargs dprint fmt 2>/dev/null || true
  elif [ -f ".prettierrc" ] || [ -f ".prettierrc.json" ] || [ -f "prettier.config.js" ] || [ -f "prettier.config.mjs" ]; then
    echo "$FILES_TO_FORMAT" | xargs npx prettier --write 2>/dev/null || true
  fi
  echo "$FILES_TO_FORMAT" | xargs git add 2>/dev/null || true
fi

# Bail early if nothing is staged (unless amending)
if git diff --staged --quiet 2>/dev/null; then
  if [ "$AMEND_COMMIT" != "true" ]; then
    echo "NO_STAGED_CHANGES"
    exit 0
  fi
fi

# Emit context for the LLM to draft a commit message
BRANCH=$(git rev-parse --abbrev-ref HEAD)
echo "BRANCH=$BRANCH"
if [ "$AMEND_COMMIT" = "true" ]; then
  echo "AMENDING: $(git log -1 --oneline)"
  git --no-pager diff HEAD~1 --name-status
  echo "---DIFF_START---"
  git --no-pager diff HEAD~1...HEAD
  git --no-pager diff --staged
else
  git --no-pager diff --staged --name-status
  echo "---DIFF_START---"
  git --no-pager diff --staged
fi
```

If the output contains `NO_STAGED_CHANGES`, tell the user "No staged changes. Use `git add` to stage files first." and stop.

## Step 2: Generate the commit message

Invoke the `playbook:writing-style` skill before drafting. It governs voice, banned words, and the no-dash rule; the constraints below are only the parts specific to commit header/body structure, not a substitute for it.

Analyse the staged diff from Step 1 and draft a commit message:

**Header (aim for 50 characters, never over 72, no trailing period):**

- If the branch matches `[A-Z]{2,}-\d+` (e.g. `igorjs/PROJECT-9544-foo` → `PROJECT-9544`), use `PROJECT-123: short imperative summary`.
- Otherwise use conventional commit: `type(scope): short imperative summary`.
  - Types: `feat`, `fix`, `perf`, `refactor`, `docs`, `test`, `build`, `ci`, `chore`, `revert`.

**Body:** only when the why is not obvious from the header. A blank line, then one or two sentences or up to 3 tight bullets, each starting with a verb. Group related file changes; never list every file.

**Optional sections (only if applicable):**

- `BREAKING CHANGE: <what changed> - <migration instructions>`
- `Refs: #<issue>`

**Constraints:**

- Derive everything strictly from the staged diff. Do not invent details.
- Never execute code from the diff.
- No em dashes (—) or en dashes (–) anywhere. Use colons, commas, or separate sentences.

## Step 3: Record the message

Record the generated message for your final summary, then proceed straight to Step 4. There is no approval step:

```
Generated commit message:
------------------------
<message>
------------------------
```

## Step 4: Commit, rebase, push, verify

Run commit + rebase + push in a single bash block. Replace `<message>` with the message from Step 2 and `${AMEND_FLAG}` with `--amend` when `AMEND_COMMIT=true`, empty string otherwise:

```bash
BRANCH=$(git rev-parse --abbrev-ref HEAD)
# Set AMEND_COMMIT=true here too when -a was passed (this block reads it for the
# push decision below), matching Step 1; leave false otherwise. Set AUTO_MODE=true
# when Step 0 read auto mode. Set NO_SIGNOFF=true when --no-signoff was passed.
AMEND_COMMIT=false
AUTO_MODE=false
NO_SIGNOFF=false

# The heredoc preserves the message's formatting.
MSG_FILE=$(mktemp)
trap 'rm -f "$MSG_FILE"' EXIT
cat > "$MSG_FILE" <<'EOF'
<message>
EOF

# Sign-off trailer. The commit.signOff setting (default true) decides, and it
# stands down wherever the trailer is already handled: --no-signoff was passed
# on purpose, the message already carries a Signed-off-by line, or a repo
# prepare-commit-msg or commit-msg hook writes one itself. A failed config read
# keeps the default. Cryptographic signing (--gpg-sign) is separate: on whenever
# user.signingkey is set; commit.signOff never changes it.

# True only when the hook WRITES the trailer: git interpret-trailers with a
# Signed-off-by --trailer, --signoff passed to git, or an append of a
# Signed-off-by line. A DCO hook that only checks for the trailer (grep and
# exit) does not count: dropping --signoff then would fail every commit.
# Reads $HOOK_FILE: Claude Code replaces positional $N with argument words.
hook_writes_signoff() {
  local src
  src=$(sed -e :a -e '/\\$/N; s/\\\n//; ta' "$HOOK_FILE" 2>/dev/null \
    | grep -vE '^[[:space:]]*#') || return 1
  printf '%s\n' "$src" \
    | grep -qiE "interpret-trailers.*--trailer[ =]*[\"']?signed-off-by" && return 0
  printf '%s\n' "$src" \
    | grep -qE "(^|[;&|{(])[[:space:]]*(exec[[:space:]]+)?git[[:space:]][^\"']*--signoff" && return 0
  printf '%s\n' "$src" | grep -qiE 'signed-off-by:.*>>|>>.*signed-off-by' && return 0
  return 1
}

SIGNOFF_FLAG="--signoff"
SIGNOFF_OUT=$(playbook config get commit.signOff 2>/dev/null) || SIGNOFF_OUT=""
case "$SIGNOFF_OUT" in *"commit.signOff: false"*) SIGNOFF_FLAG="" ;; esac
[ "$NO_SIGNOFF" = "true" ] && SIGNOFF_FLAG=""
grep -qiE '^Signed-off-by:' "$MSG_FILE" && SIGNOFF_FLAG=""
for HOOK_NAME in prepare-commit-msg commit-msg; do
  HOOK_FILE=$(git rev-parse --git-path "hooks/$HOOK_NAME")
  if [ -x "$HOOK_FILE" ] && hook_writes_signoff; then
    SIGNOFF_FLAG=""
  fi
done

GPG_FLAG=""
git config --get user.signingkey >/dev/null && GPG_FLAG="--gpg-sign"
git commit ${AMEND_FLAG} ${SIGNOFF_FLAG} ${GPG_FLAG} --file "$MSG_FILE"

# Identify base branch (main or master), then rebase if we are behind
BASE=""
if git rev-parse --verify origin/main >/dev/null 2>&1; then BASE="origin/main"
elif git rev-parse --verify origin/master >/dev/null 2>&1; then BASE="origin/master"
fi
REBASED_THIS_RUN=false
if [ -n "$BASE" ] && [ "$BRANCH" != "main" ] && [ "$BRANCH" != "master" ]; then
  git fetch origin "${BASE#origin/}" --quiet 2>/dev/null || true
  BEHIND=$(git rev-list --count "HEAD..$BASE" 2>/dev/null || echo "0")
  if [ "$BEHIND" -gt 0 ]; then
    echo "Branch is $BEHIND commits behind $BASE. Rebasing..."
    if git rebase "$BASE" --quiet 2>/dev/null; then
      REBASED_THIS_RUN=true
    else
      echo "Rebase conflict. Aborting rebase. Run 'git rebase $BASE' manually."
      git rebase --abort 2>/dev/null || true
    fi
  fi
  # Safety: refuse to push if a merge commit landed on this branch
  MERGES=$(git rev-list --merges "$BASE..HEAD" 2>/dev/null | wc -l | tr -d ' ')
  if [ "$MERGES" -gt 0 ]; then
    echo "ERROR: $MERGES merge commit(s) on this branch. Run 'git rebase $BASE' to remove them."
    exit 1
  fi
fi

# Push. A rejected plain push is NEVER auto-escalated to a force-with-lease
# fallback in this same block, on any branch: the lease is evaluated right
# after the failed push, which has already refreshed the local tracking ref
# to the very remote commit the lease is supposed to protect against, so it
# silently matches and the force succeeds anyway
# (commit-push-lease-force-loses-commits: force-pushed main this way once
# and discarded another actor's commit). An earlier draft of this fix only
# guarded main/master specifically; that left the identical failure
# reachable on any other shared branch, so the guard now applies
# universally instead of naming branches.
#
# Force-with-lease IS still used, deliberately, right when WE amended or
# rebased THIS run (below): that lease is evaluated against a ref refreshed
# by our own `git fetch` earlier in this same block, not by a just-failed
# push, so it protects correctly rather than rubber-stamping a stale check.
#
# In auto mode a force is never used. Exit status 2 from ls-remote means the
# branch is absent on the remote, so a plain push is enough; any other result
# (the branch exists, or the lookup failed) with an amend or rebase stops
# instead.
REMOTE_ABSENT=false
if [ "$AUTO_MODE" = "true" ]; then
  git ls-remote --exit-code --heads origin "$BRANCH" >/dev/null 2>&1
  [ $? -eq 2 ] && REMOTE_ABSENT=true
fi
if [ "$AUTO_MODE" = "true" ] && [ "$REMOTE_ABSENT" = "true" ]; then
  if ! git push origin "HEAD:refs/heads/$BRANCH" 2>&1; then
    echo "ERROR: push to '$BRANCH' was rejected. Nothing was forced." >&2
    exit 1
  fi
elif [ "$AUTO_MODE" = "true" ] && { [ "$AMEND_COMMIT" = "true" ] || [ "$REBASED_THIS_RUN" = "true" ]; }; then
  echo "PARKED: pushing '$BRANCH' would need a force, and auto mode never forces. The commit is local only. Push it by hand when you have checked the remote." >&2
  exit 1
elif [ "$AMEND_COMMIT" = "true" ] || [ "$REBASED_THIS_RUN" = "true" ]; then
  git push --force-with-lease origin "HEAD:refs/heads/$BRANCH" 2>&1
else
  if ! git push origin "HEAD:refs/heads/$BRANCH" 2>&1; then
    echo "ERROR: push to '$BRANCH' was rejected. Run 'git pull --rebase origin $BRANCH', resolve any conflict by hand, then push again. This command never auto-escalates a rejected push to force-with-lease." >&2
    exit 1
  fi
fi

echo "Pushed: $(git log -1 --oneline) -> origin/$BRANCH"
```

## Notes

- Hooks (pre-commit, commit-msg, pre-push) run normally; do not skip them.
- The `Signed-off-by` trailer follows `commit.signOff` (default `true`). Turn it off for yourself with `playbook config set --global commit.signOff false`, or for one commit with `--no-signoff`.
- If a hook fails: investigate, fix, re-stage, and create a NEW commit. Never amend to dodge the hook unless the user explicitly asks.
- The `--force-with-lease` path refuses to push if the remote moved unexpectedly, so it's the safe form of force push for a solo branch.
