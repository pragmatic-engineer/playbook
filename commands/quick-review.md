---
description: Quick single-pass PR review using grounding-review discipline + Conventional Comments. Report-only with no PR number given, --self, or when the resolved PR is yours; otherwise posts findings as a pending GitHub review for human submit.
allowed-tools: Bash, Read, Grep, Glob, Write, Agent, Skill
argument-hint: "[PR number] [--self]"
model: sonnet
effort: high
---

# Quick Review

Review a pull request with grounding-review discipline. Output a structured report, then orchestrate posting findings as inline comments on a **pending** GitHub review so the user picks the submit verb.

## Argument parsing

Parse `$ARGUMENTS` (strip `--self` before reading the rest, same as `--help`):

- **Integer or `#N`** (e.g. `4265`, `#4265`) → explicit PR number; resolve `HEAD_SHA` via `gh pr view <PR_NUMBER> --json headRefOid -q .headRefOid`.
- **Branch name** (anything that isn't an integer and isn't empty, and passes `git check-ref-format --branch <arg>`) → resolve to its open PR number via:
  ```bash
  PR_NUMBER=$(gh pr list --head <branch> --json number -q '.[0].number' 2>/dev/null)
  ```
  Error (abort) if no PR found: `error: no open PR for branch <name>; create one first or pass a PR number`.
- **Empty (no PR number or branch left after stripping `--self`)** → resolve the current branch's PR via `gh pr view --json number,headRefOid,author,headRefName`, and set `SELF_MODE=true`: nothing was named to post to, so review report-only, same as passing `--self` explicitly.

`--self` forces `SELF_MODE=true` regardless of whether a PR number was also given: review and report, skip Step 4's posting orchestration entirely. `SELF_MODE` also becomes true whenever the resolved PR turns out to be authored by you, even with an explicit PR number: GitHub rejects `APPROVE` and `REQUEST_CHANGES` from a PR's own author, and a comment-only review of your own PR has no independent reviewer behind it, so self-authorship gets the same report-only treatment as `--self` rather than a narrower posting path.

## Self-review awareness

`SELF_REVIEW` (the resolved PR is authored by you, detected by comparing `gh pr view --json author -q .author.login` against `gh api /user -q .login`) is computed for every run and is one of the three conditions that sets `SELF_MODE=true` (the others: an empty argument list, or explicit `--self`). It stays a distinct variable purely so the status line can log it independently, but it never posts a restricted review on its own: once `SELF_REVIEW` is true, `SELF_MODE` is true too, and Step 4 never runs.

## Worktree vs in-place mode

After resolving `PR_NUMBER` and `HEAD_SHA`, decide how to read the PR's files:

**In-place** (no worktree): use the current working tree when BOTH conditions hold:
1. `git rev-parse HEAD` equals `HEAD_SHA`
2. `git status --porcelain --untracked-files=no` is empty (no staged or unstaged tracked-file changes)

With no argument (`SELF_MODE`), the in-place predicate runs the same check. If the current branch's HEAD matches `HEAD_SHA` and the tree is clean, review in place.

**Worktree mode** (all other cases): set up an isolated worktree:

```bash
[ -r "${CLAUDE_PLUGIN_ROOT}/shell/review-worktree.sh" ] || { echo "error: review-worktree.sh not found under \$CLAUDE_PLUGIN_ROOT/shell/" >&2; exit 1; }
WT_ERR="$(mktemp)"
WT="$(bash "${CLAUDE_PLUGIN_ROOT}/shell/review-worktree.sh" setup "$PR_NUMBER" "$HEAD_SHA" 2>"$WT_ERR")"
if [[ $? -ne 0 || -z "$WT" ]]; then
  echo "error: worktree setup failed: $(cat "$WT_ERR")" >&2
  rm -f "$WT_ERR"
  exit 1
fi
rm -f "$WT_ERR"
```

On failure this prints the script's stderr and stops. No fallback, no degraded mode.
Capture stdout only: `review-worktree.sh` prints the worktree path on stdout and sends
git's progress to stderr on purpose, so folding them together corrupts the path.

When in worktree mode, read and grep all files under `$WT` instead of the local working tree. Store `WT_CREATED=true` for the teardown step.

## Voice rules (mandatory)

Invoke the `playbook:grounding-review` skill before drafting any finding, and load the `playbook:writing-style` skill alongside it (grounding-review depends on it for voice, banned words, and GitHub comment patterns). The full discipline lives in those two skills.

Comment bodies are read by another engineer, so they use the humane `playbook:writing-style` register (warm, contractions, constructive), NOT the terse operator voice from the "Concise & Direct" output style or system prompt `## Output`. Where those would conflict, `playbook:writing-style` wins for anything posted to GitHub. The non-negotiable points for inline comments posted to GitHub:

- **Conventional Comments label on every finding, PLAIN TEXT (no bold), bare.** Start the body with the bare label: `blocking:`, `issue:`, `suggestion:`, `nitpick:`, `question:`. NEVER wrap in `**...**`. Per writing-style: "a human typing fast doesn't wrap labels in `**`." Valid labels: `blocking`, `issue`, `suggestion`, `nitpick`, `question`. `blocking` replaces `issue` for a finding that must be fixed before merge; `issue` is reserved for a real problem that is not merge-blocking. The label itself orders the findings and is what posts, no separate decoration.
- **One sentence by default, two at most: the problem, then what breaks.** State the defect and its failure, then stop. A second sentence only when the mechanism is genuinely non-obvious. A finding that argues a real decision can run a little longer. Avoid jargon; plainest words available. Don't teach the author what they already know or recap the diff.
- **Pick one pragmatic fix.** No "X, or Y" options. If both work, prefer the smallest diff and recommend that one.
- **Paraphrase, don't quote.** Block-quoting the README or source code is almost always longer than restating it in your own words.
- **Don't restate the diff or the anchor.** The author wrote the code; the comment is already on the line. Skip "this function adds X" and skip "at file:line" when the comment IS at that line.
- **Cause or consequence, not both.** State the cause; trust the reader to infer the consequence.
- **Drop intermediate-state padding.** "X is blank" beats "ships a blank X to the CSV".
- **No hedging.** Ban: "may actually be", "I'd lean toward", "that said", "worth noting", "it's worth mentioning", "one could argue".
- **No meta-justification.** "since X is a foot-gun" is reviewer-reasoning, not actionable info.
- **Casual register.** Fragments OK. Lowercase verbs fine.
- **No em dashes or en dashes.** Use commas, colons, or periods. Hard rule, also enforced in the system prompt and `playbook:writing-style`.

## Execution rules

1. Run every bash block for real. Don't simulate.
2. The `reviewer` subagent reads every file it cites at the PR's head SHA (grounding-review evidence rule); the orchestrator does not re-read them.
3. Combine independent bash calls into a single tool call.
4. Anchor every inline comment to a real `file:line` in the diff. If the line isn't in the diff (e.g. a referenced helper), make it a report-level finding instead.
5. Never auto-submit. Always create the review in `PENDING` state and ask the user how to submit.
6. Never post findings in the review body. The body is for a short human-voiced framing sentence or blank. All findings go inline.

## Step 1: Resolve PR and gather context

```bash
ARGS="$ARGUMENTS"
SELF_MODE=false
[[ "$ARGS" == *"--self"* ]] && SELF_MODE=true
ARGS="${ARGS//--self/}"
ARGS="${ARGS// /}"

if [ -z "$ARGS" ]; then
  # Nothing named to post to: resolve current branch's PR, report-only.
  PR_JSON=$(gh pr view --json number,headRefOid,author,headRefName 2>/dev/null) || { echo "error: no PR found for current branch; create one first or pass a PR number" >&2; exit 1; }
  PR_NUMBER=$(echo "$PR_JSON" | jq -r .number)
  HEAD_SHA=$(echo "$PR_JSON" | jq -r .headRefOid)
  SELF_MODE=true
else
  ARGS="${ARGS#\#}"
  if [[ "$ARGS" =~ ^[0-9]+$ ]]; then
    # Integer: explicit PR number
    PR_NUMBER="$ARGS"
    HEAD_SHA=$(gh pr view "$PR_NUMBER" --json headRefOid -q .headRefOid)
  elif git check-ref-format --branch "$ARGS" 2>/dev/null; then
    # Branch name: resolve to open PR
    PR_NUMBER=$(gh pr list --head "$ARGS" --json number -q '.[0].number' 2>/dev/null)
    if [ -z "$PR_NUMBER" ]; then
      echo "error: no open PR for branch $ARGS; create one first or pass a PR number" >&2
      exit 1
    fi
    HEAD_SHA=$(gh pr view "$PR_NUMBER" --json headRefOid -q .headRefOid)
  else
    echo "error: pass an integer PR number, a branch name, --self, or no args (report-only)" >&2
    exit 1
  fi
fi

REPO=$(gh repo view --json nameWithOwner -q .nameWithOwner)
PR_AUTHOR=$(gh pr view "$PR_NUMBER" --json author -q .author.login)
ME=$(gh api /user -q .login)
SELF_REVIEW=$([ "$PR_AUTHOR" = "$ME" ] && echo true || echo false)
# A self-authored PR gets the same report-only treatment as --self: GitHub
# blocks approve/request-changes from the author, and a comment-only review
# of your own PR has no independent reviewer behind it.
[[ "$SELF_REVIEW" == "true" ]] && SELF_MODE=true

# Decide: review in-place or via isolated worktree
LOCAL_HEAD=$(git rev-parse HEAD 2>/dev/null)
DIRTY=$(git status --porcelain --untracked-files=no 2>/dev/null)
if [[ "$LOCAL_HEAD" == "$HEAD_SHA" && -z "$DIRTY" ]]; then
  WT=""
  WT_CREATED=false
  echo "Mode: in-place (HEAD matches, tree clean)"
else
  WT_ERR="$(mktemp)"
  WT="$(bash "${CLAUDE_PLUGIN_ROOT}/shell/review-worktree.sh" setup "$PR_NUMBER" "$HEAD_SHA" 2>"$WT_ERR")"
  if [[ $? -ne 0 || -z "$WT" ]]; then
    echo "error: worktree setup failed: $(cat "$WT_ERR")" >&2
    rm -f "$WT_ERR"
    exit 1
  fi
  rm -f "$WT_ERR"
  WT_CREATED=true
  echo "Mode: worktree at $WT"
fi

REVIEW_JSON="/tmp/$REPO/quick-review-$PR_NUMBER.json"
mkdir -p "$(dirname "$REVIEW_JSON")"

echo "PR: $REPO#$PR_NUMBER"
echo "Head SHA: $HEAD_SHA"
echo "Author: $PR_AUTHOR (self-review: $SELF_REVIEW, self-mode/report-only: $SELF_MODE)"
echo "Review JSON: $REVIEW_JSON"

gh pr view "$PR_NUMBER"
gh pr diff "$PR_NUMBER"
```

Capture: `REPO`, `PR_NUMBER`, `HEAD_SHA`, `SELF_REVIEW`, `SELF_MODE`, `REVIEW_JSON`. You'll need them for the API calls in Step 4. `REVIEW_JSON` resolves to `/tmp/<org>/<repo>/quick-review-<number>.json`, and its directory is created here so the Step 4 write succeeds.

## Step 2: Delegate the review pass (isolated reviewer subagent)

Reading and analysing the changed files is where main-context rot accumulates, so it runs in an isolated `reviewer` subagent, not the main session. The orchestrator keeps only the returned report, never the file contents.

Spawn ONE `reviewer` subagent (`subagent_type: playbook:reviewer`); it pins its own model tier, so the orchestrator doesn't set `model` on this call. Because the review is single-pass, its focus is the ENTIRE diff (logic, tests, security, data, types, perf, docs), not one lens.

The subagent prompt MUST include:

- The PR diff and `HEAD_SHA`.
- How to read files: **worktree mode** → the absolute `$WT` path with "read and grep files under $WT; do not install or build"; **in-place mode** (`WT` empty) → "read the local working tree, which is at HEAD_SHA".
- The full **Voice rules (mandatory)** and **Anti-patterns to refuse** sections from this command, verbatim, plus the instruction to load `playbook:grounding-review` and `playbook:writing-style` for the rest of the discipline.
- The output contract in Step 3: it MUST return exactly that report, one `Post:` block per finding.
- Read every cited file at `HEAD_SHA` before drafting; quote exact evidence; tag anything unconfirmed `[unverified]`.

Spawn it with a stable `name` (e.g. `qr-<PR_NUMBER>`); the moment it returns its report, `TaskStop` it. There is no gh-api fallback; if the worktree setup in Step 1 failed, execution has already stopped.

## Step 3: Review report contract

**The `reviewer` agent has only one channel, and it is unreliable. Plan for that** (`playbook:delegating-subagents`). It is structurally read-only (Read, Grep, Glob, Skill), so it cannot write its report to a file: `playbook agents check` forbids `Write` and `Bash` for that tier by design, and granting them would fail CI. Its return value is therefore the only route, and Agent-tool return values have failed outright in measured use.

So:

- **Run your own grounding pass on the diff in parallel**, starting as soon as the subagent is dispatched rather than after it goes quiet. That way a silent reviewer costs latency, not coverage.
- **A silent reviewer is NOT a clean review.** If nothing comes back, say the review did not run. Do not report zero findings, and do not post a pending review implying the diff was reviewed. Those are different outcomes and only one is safe to act on.
- After the idle notification fires, one `SendMessage` asking for partial results is worth a single try; it sometimes works. Do not spend more than one round on it.

The report is rendered in the `playbook:grounding-review` Review Report Format. `/playbook:quick-review` is single-pass, so it OMITS the `### Reviewers` line; every other line matches the canonical shape. Each finding carries its `Post:` block (the exact GitHub comment), or `Report-only: not on a changed line, no inline draft.` when the evidence is not on a changed diff line.

Relay the report to the user unchanged, then proceed to posting. Post findings verbatim from their `Post:` blocks; the orchestrator does NOT re-read source files (the subagent already grounded every citation), which is what keeps main context lean.

## Step 4: Orchestrate posting

If `SELF_MODE` is true (explicit `--self`, no PR number/branch was given, or the resolved PR is authored by you), stop here: the report IS the deliverable, no GitHub posting.

Otherwise, **ask the user, one question at a time** (memory rule):

**Q1**: "Post which findings as a pending review?" Offer exactly these six tiers, each a strict superset of the one before, blocking and questions take precedence, suggestions and nitpicks stay optional:

- `only blockers` (`blocking` findings only)
- `all issues` (`blocking` + `issue`)
- `all issues + questions` (`blocking` + `issue` + `question`)
- `all except nitpicks` (`blocking` + `issue` + `question` + `suggestion`)
- `all findings` (everything, `nitpick` included)
- `none` (stop, nothing posted)

Wait for response. If `none`, stop here.

Build each inline comment from that finding's `Post:` block verbatim as the comment `body`, anchored to the finding's `file:line`. What the user read in the report is exactly what posts. Skip any finding marked `Report-only`.

Build a JSON payload at `$REVIEW_JSON` (`/tmp/<org>/<repo>/quick-review-<number>.json`; the directory was created in Step 1):

```json
{
  "commit_id": "<HEAD_SHA>",
  "comments": [
    {"path": "...", "line": N, "side": "RIGHT", "body": "blocking: ..."},
    {"path": "...", "start_line": N, "start_side": "RIGHT", "line": M, "side": "RIGHT", "body": "..."}
  ]
}
```

**No `body` field on the pending review.** The author chooses their own framing when they submit from the GitHub UI. (If the user explicitly supplies a body, include it.)

Create the pending review:

```bash
gh api -X POST /repos/$REPO/pulls/$PR_NUMBER/reviews --input "$REVIEW_JSON" --jq '{id, state, html_url}'
```

Confirm `state: PENDING` and capture the review id + html_url. Show the user the link.

**Q2**: "Submit verb? approve / comment / request-changes / skip." Reaching this question already means `SELF_MODE` was false, so the PR is never self-authored here and all four verbs are always valid; GitHub's author restriction is exactly why `SELF_REVIEW` forces `SELF_MODE` earlier instead of trying to offer a narrower menu here.

If `skip`, stop. The pending review stays for manual submit from the UI. Otherwise:

**Q3**: "Add a comment for the review?" (optional free text, blank to skip). Leave `BODY` empty on a blank answer, except: on `approve` with a blank answer, default `BODY` to `LGTM`.

```bash
if [ -n "$BODY" ]; then
  gh api -X POST /repos/$REPO/pulls/$PR_NUMBER/reviews/$REVIEW_ID/events -f event=<APPROVE|COMMENT|REQUEST_CHANGES> -f body="$BODY"
else
  gh api -X POST /repos/$REPO/pulls/$PR_NUMBER/reviews/$REVIEW_ID/events -f event=<APPROVE|COMMENT|REQUEST_CHANGES>
fi
```

Confirm the returned `state` flipped from `PENDING` to the corresponding terminal state.

## Step 5: Verify and report

Final user-facing message: one sentence per outcome.

- `SELF_MODE`: "Report-only, nothing posted. N findings above."
- "Pending review id `<id>` created, 7 inline comments queued. Submit from the UI when ready."
- OR: "Submitted as `COMMENT` at <timestamp>. Author will get one notification."

## Step 6: Teardown (MUST run, even on failure, abort, or skip)

If `WT_CREATED` is true, always run:

```bash
bash "${CLAUDE_PLUGIN_ROOT}/shell/review-worktree.sh" teardown "$WT"
```

This step is unconditional: run it whether the review completed, failed, was skipped, or was aborted by the user. It is a no-op if the worktree was already cleaned up.

## Anti-patterns to refuse

If you catch yourself doing any of these while drafting findings, stop and rewrite:

1. **Diff restatement.** "This change moves X into Y so that...". Delete the entire setup sentence and lead with the finding.
2. **Hedging stack.** "may actually be" + "I'd lean toward" + "that said" in a single comment is a tell.
3. **Meta-justification.** "since a non-timestamp string in a timestamp column is its own foot-gun". The recommendation is enough; trust the reader.
4. **Bullet-list explanation inside an inline comment.** Use short prose paragraphs (up to three), not bullets, inside an inline comment. If bullets feel necessary, the finding is too big: split or simplify.
5. **Posting findings in the review body** instead of inline.
6. **Auto-submitting** without the two-question orchestration.

## Tradeoffs intentionally accepted

- **Self-review submit is COMMENT-only.** Documented limitation of the GitHub API, not a bug to work around.
- **Once submitted, the review wrapper can't be deleted.** Body can be mutated via `PUT /reviews/{id}` but must remain non-empty for COMMENT/REQUEST_CHANGES state. Prefer leaving in PENDING until the body and findings are settled.
- **Conventional Comments labels are mandatory even on nits.** They cost a few characters; they buy bot/human triage.
