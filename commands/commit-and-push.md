---
description: Use when committing staged changes with a generated message and pushing. Handles staging, formatting, a signed commit, optional rebase, and push.
allowed-tools: Bash, Read, Skill
argument-hint: "[--all|-A] [--update|-u] [--amend|-a]"
context: fork
agent: git
---

# Commit and Push

Generate a commit message from the staged diff, commit, optionally rebase, and push. One flow.

## Run this now

Execute the steps below immediately, end to end, running every bash block for real. Do **not** narrate a plan, summarize `git status`, offer a numbered menu, or ask "what would you like me to do?" / "proceed? [Y/n]". There is **no confirmation gate**.

The flow is one pass: stage (per flags), draft the commit message, commit, rebase if behind, push. Staging flags (`-A`, `-u`, `-a`) parse from `$ARGUMENTS`; no flags means commit only what is already staged.

This command is built to run in an isolated subagent (`context: fork`) so the diff and drafting stay out of the main context. When it forks, your final message is the only thing the main conversation sees, so end with a concise outcome summary (commit SHA, branch, and the generated message). If you are instead reading this in the main conversation, run it here exactly the same way; do not wait for a fork and do not defer to the user.

## Argument flags

Parse these from `$ARGUMENTS`. Each bash block below runs in its **own shell**, so a variable set in one step is NOT visible in a later one. Apply each flag by setting its variable to `true`/`false` at the top of the block that reads it (do not rely on an env var carried over from an earlier step):

- `--all` or `-A` → `STAGE_ALL=true` (run `git add -A`). Set at the top of **Step 1**.
- `--update` or `-u` → `STAGE_UPDATE=true` (run `git add -u`, tracked files only). Set at the top of **Step 1**.
- `--amend` or `-a` → `AMEND_COMMIT=true` (amend the previous commit). Set at the top of **Step 1** AND again in **Step 4** (both blocks read it).

Combined flags are fine: `-Au`, `-a -u`, etc. No flags means every variable stays `false`: commit only what is already staged. There is no confirmation gate; the flow always runs to completion.

## Execution rules

1. Run every bash block in this command for real. Do not simulate output.
2. Use the actual command output to drive the next step.
3. Do not assume file contents or git state; check them.
4. Combine independent bash operations into single tool calls.
5. Never run destructive git commands (`reset --hard`, `push --force`, `clean -f`) unless the user explicitly asks.
6. Never skip hooks (`--no-verify`, `--no-gpg-sign`).
7. Never amend automatically: only when `AMEND_COMMIT=true`.
8. Pass commit messages via heredoc to preserve formatting, never `-m "..."` for multi-line.

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
DEFAULT_BRANCH=$(git symbolic-ref refs/remotes/origin/HEAD 2>/dev/null | sed 's@^refs/remotes/origin/@@' || echo main)
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

**Header (<= 72 chars, no trailing period):**

- If the branch matches `[A-Z]{2,}-\d+` (e.g. `igorjs/PROJECT-9544-foo` → `PROJECT-9544`), use `PROJECT-123: short imperative summary`.
- Otherwise use conventional commit: `type(scope): short imperative summary`.
  - Types: `feat`, `fix`, `perf`, `refactor`, `docs`, `test`, `build`, `ci`, `chore`, `revert`.

**Body:**

- Blank line after the header.
- 2-4 bullets describing the change, derived strictly from the diff.
- Each bullet starts with a verb, under ~15 words where possible.
- Group related file changes into one bullet; don't list every file.

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
# push decision below), matching Step 1; leave false otherwise.
AMEND_COMMIT=false

# Commit (signed + signoff). Heredoc preserves formatting.
git commit ${AMEND_FLAG} --signoff --gpg-sign --file - <<'EOF'
<message>
EOF

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

# Push. Never force on the shared default branch, under any condition: a
# lease evaluated immediately after a rejected push can match the very
# remote commit it was meant to protect (the failed push already refreshed
# the local tracking ref as a side effect of reading the remote's advertised
# refs), so --force-with-lease is not actually safe there. This force-pushed
# main and silently discarded another actor's commit once
# (commit-push-lease-force-loses-commits). Step 1's hard stop should make
# BRANCH=main/master unreachable here already; this is the second,
# independent layer in case that check is ever bypassed or this block runs
# in isolation.
if [ "$BRANCH" = "main" ] || [ "$BRANCH" = "master" ]; then
  if ! git push origin "HEAD:refs/heads/$BRANCH" 2>&1; then
    echo "ERROR: push to '$BRANCH' was rejected. This is the shared default branch: force-with-lease is never used here. Run 'git pull --rebase origin $BRANCH', resolve any conflict by hand, then push again." >&2
    exit 1
  fi
elif [ "$AMEND_COMMIT" = "true" ] || [ "$REBASED_THIS_RUN" = "true" ]; then
  # Force-with-lease when we amended OR when a rebase rewrote history.
  git push --force-with-lease origin "HEAD:refs/heads/$BRANCH" 2>&1
else
  git push origin "HEAD:refs/heads/$BRANCH" 2>&1 || {
    # Fallback: if the regular push was rejected as non-fast-forward, the remote
    # likely holds a pre-rebase ancestor (e.g. an earlier session pushed and then
    # rebased locally). Retry with force-with-lease which refuses to push if
    # someone else also moved the remote.
    git push --force-with-lease origin "HEAD:refs/heads/$BRANCH" 2>&1
  }
fi

echo "Pushed: $(git log -1 --oneline) -> origin/$BRANCH"
```

## Notes

- Hooks (pre-commit, commit-msg, pre-push) run normally; do not skip them.
- If a hook fails: investigate, fix, re-stage, and create a NEW commit. Never amend to dodge the hook unless the user explicitly asks.
- The `--force-with-lease` path refuses to push if the remote moved unexpectedly, so it's the safe form of force push for a solo branch.
