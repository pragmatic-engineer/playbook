---
description: Create a pull request with pre-flight checks, a conventional-commit title, and the team PR template, following engineering-standards and writing-style.
allowed-tools: Bash, Read, Skill
argument-hint: "[--ready] [--base <branch>] [--ticket <ID>]"
context: fork
agent: git
---

# Create Pull Request

Push the current branch and open a pull request. The title is a conventional-commit summary, the body follows the team template, and both obey `playbook:engineering-standards` (readiness, size) and `playbook:writing-style` (voice, banned words, no dashes). Every PR opens as a **draft**, always: `--ready` no longer publishes it immediately, it marks it for promotion to ready once Step 9's self-review passes, since a human should never be the first reviewer of unreviewed code.

This creates a **new** PR. If one already exists for the branch, this stops and points you at `/playbook:address-pr-comments` or `/playbook:quick-review`.

## Run this now

Execute the steps below immediately, end to end, running every bash block for real. Do **not** narrate a plan, summarize `git status`, offer a numbered menu, or ask "what would you like me to do?" / "proceed? [Y/n]". There is **no confirmation gate**.

Run end to end: auto-detect the base and ticket, draft the title and body, then push and create. Readiness problems (uncommitted work, a diff over the soft or enforced size limit, no tests) print as warnings and never pause. Only the hard aborts (on the base branch, nothing ahead of base, an existing PR, a diff over the 1500-line hard size limit) stop the run.

This command is built to run in an isolated subagent (`context: fork`) so the diff and drafting stay out of the main context. When it forks, your final message is the only thing the main conversation sees, so end with a concise outcome summary (the PR URL, title, base, and draft state). If you are instead reading this in the main conversation, run it here exactly the same way; do not wait for a fork and do not defer to the user.

## Argument flags

Parse these from `$ARGUMENTS` **once**, in Step 1, and persist them to `$PR_TMP/args.env`. Every later step `source`s that file instead of re-deriving flag values from `$ARGUMENTS` by hand.

> **Why persisted to a file, not re-parsed per step:** each bash block runs in its own shell, so nothing set inline in one block reliably survives to the next (the Bash tool keeps the working directory but not shell state). An earlier version of this command told each step to "set FOO_ARG at the top of that block" from memory of `$ARGUMENTS`; in practice `--base` was silently dropped that way, three times in a row, and every PR opened against the repo default instead of the stacked branch it was pointed at. A file on disk survives regardless of how the executing agent batches its tool calls; re-deriving a value from a natural-language instruction each step does not.

- `--ready` → promote the PR to ready once Step 9's self-review passes, instead of leaving it a draft. Parsed into `READY_FLAG` in Step 1. Does NOT skip the draft stage: every PR opens as a draft regardless of this flag.
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

- `playbook:writing-style`: voice, banned words, the "PR descriptions" guidance, and the golden rule (no em or en dashes). Every line of the title and body MUST follow it.
- `playbook:engineering-standards`: PR readiness criteria and size limits, enforced in Step 2.

The PR title and body are read by another engineer, so they use the humane `playbook:writing-style` register (warm, contractions, active voice), NOT the terse operator voice. Where they conflict, `playbook:writing-style` wins for anything posted to GitHub.

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

**Hard check, not a sanity note (MUST run before continuing to Step 2).** A prior
version of this step relied on the authoring agent noticing a mismatch and fixing
it by hand; that still let `--base` silently drop on roughly a third of runs, per
`feedback-create-pr-base-flag-drops`, because the check was prose the agent could
skim past under momentum, not something that could fail the run. Run this as its
own bash block, with the raw `$ARGUMENTS` text embedded literally (single-quoted,
verbatim, by the authoring agent, not re-derived from memory):

```bash
RAW_ARGUMENTS='<the literal, unmodified $ARGUMENTS text for this invocation>'
source "$PR_TMP/args.env"
if echo "$RAW_ARGUMENTS" | grep -q -- '--base' && [ -z "$BASE_ARG" ]; then
  echo "ERROR: --base is present in the invocation but BASE_ARG in args.env is empty. Re-open $PR_TMP/args.env with the Edit tool, set BASE_ARG to the branch named after --base, then re-run Step 1's base-resolution block before continuing." >&2
  exit 1
fi
echo "Hard check passed: --base presence in \$ARGUMENTS matches BASE_ARG."
```

If this exits non-zero, fix `$PR_TMP/args.env` with the Edit tool and re-run the
base-resolution block above; do not proceed to Step 2 on a non-zero exit here.
This is the exact failure mode the file-persistence design in this step exists to
catch: silently opening a PR against the wrong base is a correctness bug, not a
style nit, especially for stacked PRs where the base is load-bearing.

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
# Anchor the directory patterns with (^|/): git returns repo-relative paths with
# no leading slash, so a bare `/tests?/` never matches a top-level `tests/` dir
# and every Rust PR reads as "no tests". Also count Rust inline `#[cfg(test)]`.
TESTS=$(git diff --name-only "origin/$BASE_BRANCH...HEAD" | grep -ciE '(\.test\.|\.spec\.|_test\.|test_|(^|/)tests?/|(^|/)__tests__/)' || true)
INLINE_TESTS=$(git diff -U0 "origin/$BASE_BRANCH...HEAD" -- '*.rs' | grep -c '^+.*#\[cfg(test)\]' || true)
TESTS=$((TESTS + INLINE_TESTS))

echo "commits_ahead=$AHEAD changed_lines=${CHANGED:-0} dirty_files=$DIRTY test_files_touched=$TESTS"

# Hard stops (always end the run): nothing to PR, or over the 1500-line hard size limit.
if [ "$AHEAD" -eq 0 ]; then
  echo "ABORT: nothing ahead of $BASE_BRANCH; there is nothing to open a PR for"; exit 1
fi
if [ "${CHANGED:-0}" -gt 1500 ]; then
  echo "ABORT: ${CHANGED} changed lines is over the 1500-line hard size limit; split the work into smaller PRs"; exit 1
fi

# The thresholds are applied HERE, not narrated downstream.
#
# This block used to print the four raw numbers and leave the caller to apply
# the limits and describe the result in prose. Three separate runs then reported
# `test_files_touched=0` for diffs that really did touch tests, and twice
# invented the same false cause (a "missing regex anchor" that is present two
# lines above and demonstrably works). A value the script can compute must never
# be restated from memory: the script decides, the caller copies.
verdict() { printf 'VERDICT %s\n' "$1"; }

if [ "$DIRTY" -gt 0 ]; then
  verdict "dirty: WARN - $DIRTY uncommitted file(s) will NOT be in the PR"
else
  verdict "dirty: OK - nothing uncommitted"
fi

if [ "${CHANGED:-0}" -gt 1000 ]; then
  verdict "size: OVER - ${CHANGED} lines is above the 1000-line enforced limit and needs explicit justification in the PR body"
elif [ "${CHANGED:-0}" -gt 500 ]; then
  verdict "size: SOFT - ${CHANGED} lines is above the 500-line soft limit"
else
  verdict "size: OK - ${CHANGED:-0} lines"
fi

if [ "$TESTS" -eq 0 ]; then
  verdict "tests: NONE - the diff adds no test files or inline test blocks; the readiness criteria expect tests for behaviour changes"
else
  verdict "tests: OK - $TESTS test file(s) or inline test block(s) touched"
fi
```

**Copy every `VERDICT` line verbatim into the readiness block.** Do not recompute
them, re-derive them from the diff, or paraphrase them: the script has already
applied every threshold in `playbook:engineering-standards`. If a `VERDICT`
contradicts your own reading of the diff, the `VERDICT` is right and your reading
is wrong; report the `VERDICT` and, if it seems worth investigating, say so as a
separate remark rather than replacing the line.

The two hard stops (`AHEAD` = 0, `CHANGED` > 1500) have already exited above.
Every `VERDICT` is non-blocking: print them and move on without pausing.

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

Fill this template exactly. Keep the section order. Follow `playbook:writing-style` throughout: active voice, contractions, no banned words, no em or en dashes, no "This PR..." filler.

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

Every PR opens as a **draft**, unconditionally. `READY_FLAG` (from `$PR_TMP/args.env`, parsed once in Step 1) is NOT used here: it's read by Step 9, after the self-review, to decide whether to promote the draft. This step never publishes a PR ready for review directly.

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

# Always draft: --ready (READY_FLAG) is Step 9's job, after the self-review, not this step's.
DRAFT_ARG="--draft"

echo "Creating PR: $CURRENT_BRANCH -> $BASE_BRANCH (draft: $([ -n "$DRAFT_ARG" ] && echo yes || echo no))"

# The push MUST gate the create. This block has no `set -e`, so without the
# explicit check a rejected push (non-fast-forward, network, no write access)
# falls straight through and `gh pr create` opens a PR against whatever the
# remote branch held before, silently missing the local commits.
if ! git push -u origin "HEAD:refs/heads/$CURRENT_BRANCH"; then
  echo "ABORT: push of $CURRENT_BRANCH failed; not creating a PR (it would be missing your local commits)" >&2
  exit 1
fi

# Confirm the remote actually carries this HEAD before opening the PR.
LOCAL_SHA=$(git rev-parse HEAD)
REMOTE_SHA=$(git ls-remote origin "refs/heads/$CURRENT_BRANCH" | cut -f1)
if [ "$LOCAL_SHA" != "$REMOTE_SHA" ]; then
  echo "ABORT: origin/$CURRENT_BRANCH is at ${REMOTE_SHA:-<missing>}, local HEAD is $LOCAL_SHA; the push did not land" >&2
  exit 1
fi

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

Show the PR URL and a one-line summary (title, base, draft state). Include the `READY_FLAG` value from `$PR_TMP/args.env`: Step 9 below reads it to decide what happens next.

## Step 9: Self-review before ready (for the orchestrating session, not this forked agent)

This step is not executable from inside this command's own forked context: it runs as `context: fork, agent: git`, and the `git` agent's tools are `Bash, Read, Skill` only, no `Agent`. It cannot spawn `deep-review`'s reviewer swarm itself. This step is the instruction the orchestrating session (whoever invoked this skill) follows after it returns:

1. Run `/clear`, then run `/playbook:deep-review --self` against the PR just created. A draft PR is a real PR, so this works; reviewing before any PR exists does not (`deep-review` needs `gh pr view`/`gh pr diff`).
2. Fix any findings it surfaces. A push updates the draft automatically, no new PR needed.
3. If `READY_FLAG` was `--ready` (the caller wanted this published, not left as a draft): run `gh pr ready <branch>` now, after the review and fixes, not before. `--ready` means "ready once self-reviewed," not "skip the review."
4. If `READY_FLAG` was empty: stop after the review. The caller asked for a draft; leave it one.

Never skip straight to `gh pr ready` on a fresh draft without running the review first.
