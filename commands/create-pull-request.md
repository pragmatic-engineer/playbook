---
description: Create a pull request with pre-flight checks, a conventional-commit title, and the team PR template, following engineering-standards and writing-style.
allowed-tools: Bash, Read, Skill
argument-hint: "[--ready] [--base <branch>] [--ticket <ID>]"
context: fork
agent: git
---

# Create Pull Request

Push the current branch and open a pull request. The title is a conventional-commit summary, the body follows the team template, and both obey `engineering-standards` (readiness, size) and `writing-style` (voice, banned words, no dashes). The PR opens as a **draft** by default; pass `--ready` to publish it for review.

This creates a **new** PR. If one already exists for the branch, this stops and points you at `/playbook:address-pr-comments` or `/playbook:quick-review`.

## Run this now

Execute the steps below immediately, end to end, running every bash block for real. Do **not** narrate a plan, summarize `git status`, offer a numbered menu, or ask "what would you like me to do?" / "proceed? [Y/n]". There is **no confirmation gate**.

Run end to end: auto-detect the base and ticket, draft the title and body, then push and create. Readiness problems (uncommitted work, a diff over the soft or enforced size limit, no tests) print as warnings and never pause. Only the hard aborts (on the base branch, nothing ahead of base, an existing PR, a diff over the 1500-line hard size limit) stop the run.

This command is built to run in an isolated subagent (`context: fork`) so the diff and drafting stay out of the main context. When it forks, your final message is the only thing the main conversation sees, so end with a concise outcome summary (the PR URL, title, base, and draft state). If you are instead reading this in the main conversation, run it here exactly the same way; do not wait for a fork and do not defer to the user.

## Argument flags

Parse these from `$ARGUMENTS` **once**, in Step 1, and persist them to `$PR_TMP/args.env`. Every later step `source`s that file instead of re-deriving flag values from `$ARGUMENTS` by hand.

> **Why persisted to a file, not re-parsed per step:** each bash block runs in its own shell, so nothing set inline in one block reliably survives to the next (the Bash tool keeps the working directory but not shell state). An earlier version of this command told each step to "set FOO_ARG at the top of that block" from memory of `$ARGUMENTS`; in practice `--base` was silently dropped that way, three times in a row, and every PR opened against the repo default instead of the stacked branch it was pointed at. A file on disk survives regardless of how the executing agent batches its tool calls; re-deriving a value from a natural-language instruction each step does not.

- `--ready` → open the PR ready for review instead of a draft. Parsed into `READY_FLAG` in Step 1.
- `--base <branch>` → override the base branch. Parsed into `BASE_ARG` in Step 1.
- `--ticket <ID>` → force the ticket, skipping branch auto-detect (`none` omits the line). Parsed into `TICKET_ARG` in Step 1.
- `--help` → print the usage block above and stop.

There is no confirmation flag or gate: the command always runs end to end, auto-detecting base and ticket, then pushing and creating.

## Execution rules

1. Run every bash block for real. Do not simulate output; use the real result to drive the next step.
2. Do not assume git state, diff contents, or `gh` output. Check them.
3. Combine independent bash operations into single tool calls.
4. Never run destructive git commands (`reset --hard`, `push --force`, `clean -f`) or skip hooks (`--no-verify`).
5. Derive the title and body from the actual diff and commit log, never from the branch name alone or from memory.
6. Pass the PR body via `--body-file`, never `--body "..."`, to preserve formatting.

## Step 0: Load the skills (MUST run before drafting title or body)

Invoke both via the Skill tool before writing any prose:

- `writing-style`: voice, banned words, the "PR descriptions" guidance, and the golden rule (no em or en dashes). Every line of the title and body MUST follow it.
- `engineering-standards`: PR readiness criteria and size limits, enforced in Step 2.

The PR title and body are read by another engineer, so they use the humane `writing-style` register (warm, contractions, active voice), NOT the terse operator voice. Where they conflict, `writing-style` wins for anything posted to GitHub.

## Step 1: Parse flags, establish context, resolve the base branch

First, establish `CURRENT_BRANCH` and `PR_TMP` (needed before anything else can be persisted), and stop early if a PR already exists:

```bash
set -euo pipefail

CURRENT_BRANCH=$(git branch --show-current)
if [ -z "$CURRENT_BRANCH" ]; then echo "ERROR: detached HEAD; checkout a branch first"; exit 1; fi

PR_TMP="/tmp/create-pr/$(basename "$(git rev-parse --show-toplevel)")/$(echo "$CURRENT_BRANCH" | tr '/' '-')"
mkdir -p "$PR_TMP"
echo "Branch: $CURRENT_BRANCH"
echo "TMP: $PR_TMP"

EXISTING=$(gh pr view "$CURRENT_BRANCH" --json url,state -q 'select(.state=="OPEN") | .url' 2>/dev/null || true)
if [ -n "$EXISTING" ]; then
  echo "A PR already exists: $EXISTING"
  echo "Use /playbook:address-pr-comments or /playbook:quick-review instead."
  exit 0
fi
```

Now parse **every** flag out of `$ARGUMENTS` in a single pass and write them to `$PR_TMP/args.env`. This is the only place flags are parsed from `$ARGUMENTS`; every later step sources this file instead of re-deriving flag values by hand:

```bash
cat > "$PR_TMP/args.env" << 'ARGS_EOF'
BASE_ARG=""
TICKET_ARG=""
READY_FLAG=""
ARGS_EOF
```

**MUST:** immediately re-open `$PR_TMP/args.env` with the Edit tool and fill in the real values by reading `$ARGUMENTS` carefully:
- if `$ARGUMENTS` contains `--base <branch>`, set `BASE_ARG="<branch>"` (the exact branch name, verbatim, nothing else on that line).
- if `$ARGUMENTS` contains `--ticket <ID>`, set `TICKET_ARG="<ID>"`.
- if `$ARGUMENTS` contains `--ready`, set `READY_FLAG="--ready"`.
- leave any flag that is not present as `""`. Do not guess a value that was not actually passed.

Then resolve the base branch from the persisted flag and verify what was parsed before moving on:

```bash
source "$PR_TMP/args.env"
echo "parsed: BASE_ARG='${BASE_ARG}' TICKET_ARG='${TICKET_ARG}' READY_FLAG='${READY_FLAG}'"

# Resolve base: flag > repo default (gh) > git symbolic-ref > main
if [ -n "$BASE_ARG" ]; then
  BASE_BRANCH="$BASE_ARG"
  BASE_SOURCE="--base flag"
else
  BASE_BRANCH=$(gh repo view --json defaultBranchRef -q .defaultBranchRef.name 2>/dev/null \
    || git symbolic-ref refs/remotes/origin/HEAD 2>/dev/null | sed 's@^refs/remotes/origin/@@' \
    || echo main)
  BASE_SOURCE="repo default"
fi
echo "BASE_BRANCH=$BASE_BRANCH" >> "$PR_TMP/args.env"

if [ "$CURRENT_BRANCH" = "$BASE_BRANCH" ]; then
  echo "ERROR: on the base branch ($BASE_BRANCH); create a feature branch first"; exit 1
fi

git fetch origin "$BASE_BRANCH" --quiet 2>/dev/null || true
echo "Resolved base: $BASE_BRANCH (source: $BASE_SOURCE)"
```

**Before continuing to Step 2, sanity-check the echoed base against the actual request.** If `--base` appeared in `$ARGUMENTS` but `BASE_SOURCE` printed as "repo default", the edit to `args.env` was missed or wrong; fix `$PR_TMP/args.env` now and re-run the block above before proceeding. This is the exact failure mode the file-persistence design in this step exists to catch: silently opening a PR against the wrong base is a correctness bug, not a style nit, especially for stacked PRs where the base is load-bearing.

## Step 2: Pre-flight checks (engineering-standards)

```bash
# Fresh shell: re-derive PR_TMP the same way Step 1 did, then source the
# flags Step 1 persisted (BASE_BRANCH included) instead of assuming they
# survived from the previous block.
CURRENT_BRANCH=$(git branch --show-current)
PR_TMP="/tmp/create-pr/$(basename "$(git rev-parse --show-toplevel)")/$(echo "$CURRENT_BRANCH" | tr '/' '-')"
source "$PR_TMP/args.env"

# Commits ahead of base
AHEAD=$(git rev-list --count "origin/$BASE_BRANCH..HEAD" 2>/dev/null || echo 0)
# Size (additions + deletions)
SHORTSTAT=$(git diff --shortstat "origin/$BASE_BRANCH...HEAD")
CHANGED=$(echo "$SHORTSTAT" | grep -oE '[0-9]+ insertion|[0-9]+ deletion' | grep -oE '[0-9]+' | paste -sd+ - | bc 2>/dev/null || echo 0)
# Uncommitted work that would be left out of the PR
DIRTY=$(git status --porcelain | wc -l | tr -d ' ')
# Does the diff touch any test files?
TESTS=$(git diff --name-only "origin/$BASE_BRANCH...HEAD" | grep -ciE '(\.test\.|\.spec\.|_test\.|test_|/tests?/|/__tests__/)' || true)

echo "commits_ahead=$AHEAD changed_lines=${CHANGED:-0} dirty_files=$DIRTY test_files_touched=$TESTS"

# Hard stops (always end the run): nothing to PR, or over the 1500-line hard size limit.
if [ "$AHEAD" -eq 0 ]; then
  echo "ABORT: nothing ahead of $BASE_BRANCH; there is nothing to open a PR for"; exit 1
fi
if [ "${CHANGED:-0}" -gt 1500 ]; then
  echo "ABORT: ${CHANGED} changed lines is over the 1500-line hard size limit; split the work into smaller PRs"; exit 1
fi
```

Evaluate against `engineering-standards`, then decide (the two hard stops above have already exited; these are the non-blocking ones):

- **`DIRTY` > 0** → print a warning: those changes are uncommitted and won't be in the PR. Continue.
- **`CHANGED` > 1000** → print a prominent warning that it is over the enforced limit and needs explicit justification. Continue.
- **`CHANGED` > 500** → note it is above the soft limit and continue.
- **`TESTS` = 0** → note the diff adds no tests (the readiness criteria expect tests for behaviour changes). Continue.

Report the readiness picture in one short block. The two hard stops (`AHEAD` = 0, `CHANGED` > 1500) already exited in the block above; the rest print and move on without pausing.

## Step 3: Gather the diff and commit history

```bash
CURRENT_BRANCH=$(git branch --show-current)
PR_TMP="/tmp/create-pr/$(basename "$(git rev-parse --show-toplevel)")/$(echo "$CURRENT_BRANCH" | tr '/' '-')"
source "$PR_TMP/args.env"

echo "=== diff stat ==="
git diff --stat "origin/$BASE_BRANCH...HEAD"
echo "=== commit log ==="
git log "origin/$BASE_BRANCH..HEAD" --format='%h %s'
git diff "origin/$BASE_BRANCH...HEAD" > "$PR_TMP/pr-diff.txt"
echo "Full diff: $PR_TMP/pr-diff.txt ($(wc -l < "$PR_TMP/pr-diff.txt") lines)"
```

Read `$PR_TMP/pr-diff.txt` with the Read tool. This is the source of truth for the title and body. If it is large, read it in chunks; do not skip it.

## Step 4: Detect the ticket (optional)

```bash
CURRENT_BRANCH=$(git branch --show-current)
PR_TMP="/tmp/create-pr/$(basename "$(git rev-parse --show-toplevel)")/$(echo "$CURRENT_BRANCH" | tr '/' '-')"
source "$PR_TMP/args.env"

if [ -n "$TICKET_ARG" ]; then
  TICKET="$TICKET_ARG"   # may be the literal "none"
else
  # First PROJECT-1234 style token in the branch name
  TICKET=$(echo "$CURRENT_BRANCH" | grep -oE '[A-Z][A-Z0-9]+-[0-9]+' | head -1 || true)
fi
echo "TICKET=${TICKET:-<none>}"
```

- If `TICKET` is a real ID (not empty, not `none`) → include `Ticket: <ID>` as the first line of the body.
- If empty → the branch has no ticket; omit the line without asking.
- If `none` → omit the line.

## Step 5: Generate the title (conventional commits)

Derive the title from the diff and commit log gathered in Step 3.

- Format: `type(scope): summary`, e.g. `feat(auth): add SSO retry logic`. Scope is optional.
- Types: `feat`, `fix`, `refactor`, `perf`, `docs`, `test`, `build`, `ci`, `chore`.
- Imperative mood ("add", not "added" or "adds"), no trailing period.
- **Length: 72 characters maximum**, counting the entire line including the `type(scope):` prefix. This is a hard limit, not a target. If the draft exceeds it, tighten the summary (drop the scope, cut filler, shorten wording) until it fits; never open a PR with a title over 72 characters. Verify the count before creating the PR, e.g. `printf '%s' "$TITLE" | wc -m` must be `<= 72`.
- The summary states the **effect** of the change, not a list of files.
- The ticket goes in the body, not the title.

## Step 6: Generate the body (MANDATORY template)

Fill this template exactly. Keep the section order. Follow `writing-style` throughout: active voice, contractions, no banned words, no em or en dashes, no "This PR..." filler.

```markdown
Ticket: PROJECT-1234

## Summary

<Why are we doing this? 1-2 sentences, active voice. Focus on the why, not the what. The bug being fixed, the requirement, the motivation. Do not echo the title.>

## What Changed

- <Bullets in plain terms. Describe concepts and context, not files. Give the reviewer what they need to follow the change. 3-8 bullets, grouped logically.>

## Notes for reviewers

- <Optional. Oddities, trade-offs, intentional tech debt, anything that needs human context. Drop this whole section if there's nothing to add.>

## Related work

- <Optional. Cross-references to other PRs or tickets. Drop this whole section if there's nothing to add.>
```

Rules for filling it:

1. **`Ticket:` line**: include only when Step 4 found a real ID; otherwise delete the line so the body starts at `## Summary`.
2. **Summary**: the why, not the what. One or two sentences. If the title is `fix(cache): stop stale reads after invalidation`, the Summary explains why stale reads mattered, not that you changed the cache.
3. **What Changed**: every bullet maps to something real in the diff. Group by concept, don't enumerate files. Use the same terms the code uses (if it's a "handler", don't call it a "controller").
4. **Notes for reviewers**: drop the heading entirely if empty. Don't leave "N/A".
5. **Related work**: drop the heading entirely if empty.
6. No trailing "generated by" footer. No test-count noise. If CI covers it, the reviewer sees CI.

Write the finished body to a file:

```bash
CURRENT_BRANCH=$(git branch --show-current)
PR_TMP="/tmp/create-pr/$(basename "$(git rev-parse --show-toplevel)")/$(echo "$CURRENT_BRANCH" | tr '/' '-')"

cat > "$PR_TMP/pr-body.md" << 'PRBODY_EOF'
<the filled template goes here>
PRBODY_EOF
echo "Body written: $PR_TMP/pr-body.md"
```

## Step 7: Push and create

The PR opens as a **draft** unless `--ready` was passed. `READY_FLAG` came from `$PR_TMP/args.env` (parsed once, in Step 1); this step only translates it into the flag `gh pr create` expects, it does not re-parse `$ARGUMENTS`.

```bash
CURRENT_BRANCH=$(git branch --show-current)
PR_TMP="/tmp/create-pr/$(basename "$(git rev-parse --show-toplevel)")/$(echo "$CURRENT_BRANCH" | tr '/' '-')"
source "$PR_TMP/args.env"

# Hard limit: PR title is at most 72 characters (whole line, prefix included).
TITLE_LEN=$(printf '%s' "$TITLE" | wc -m | tr -d ' ')
if [ "$TITLE_LEN" -gt 72 ]; then
  echo "error: PR title is $TITLE_LEN characters (limit 72); tighten it before creating the PR: $TITLE" >&2
  exit 1
fi

# Draft by default; --ready (READY_FLAG, from args.env) publishes for review.
DRAFT_ARG="--draft"
[ -n "$READY_FLAG" ] && DRAFT_ARG=""

echo "Creating PR: $CURRENT_BRANCH -> $BASE_BRANCH (draft: $([ -n "$DRAFT_ARG" ] && echo yes || echo no))"

git push -u origin "HEAD:refs/heads/$CURRENT_BRANCH"

gh pr create \
  --title "$TITLE" \
  --body-file "$PR_TMP/pr-body.md" \
  --base "$BASE_BRANCH" \
  $DRAFT_ARG
```

**Verify after creation, do not trust the command's own success message:** run `gh pr view "$CURRENT_BRANCH" --json baseRefName,headRefName -q '{base:.baseRefName,head:.headRefName}'` and confirm `base` matches the `BASE_BRANCH` resolved in Step 1. If it does not, the PR was opened against the wrong base; fix it immediately with `gh pr edit "$CURRENT_BRANCH" --base "$BASE_BRANCH"` before reporting success in Step 8.

## Step 8: Report

```bash
PR_URL=$(gh pr view "$CURRENT_BRANCH" --json url -q .url 2>/dev/null || true)
echo "PR: $PR_URL"
```

Show the PR URL and a one-line summary (title, base, draft state). Done.
