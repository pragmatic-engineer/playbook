---
description: Use for substantial, risky, or cross-cutting PRs. A swarm of specialist reviewer subagents (logic, test, security, data, types, perf, plus conditional) run in parallel, consolidated and fact-checked, then posted as a pending GitHub review. Heavier than /playbook:quick-review.
allowed-tools: Bash, Read, Grep, Glob, Write, Agent, Skill
argument-hint: "[PR number] [--all] [--quick] [--preset <name>] [--self] [--help]"
model: opus
effort: xhigh
---

# Deep Review: Multi-Agent PR Review

Review a pull request with a swarm of specialist reviewer subagents run in parallel, each a focused `reviewer` subagent under the `grounding-review` and `grounding-research` discipline. The orchestrating session consolidates, dedups, and fact-checks their findings, then posts them as a **pending** GitHub review (same posting flow as `/playbook:quick-review`). This is heavier and slower than `/playbook:quick-review`; use it for substantial, risky, or cross-cutting PRs.

Invoked as `/playbook:deep-review`. The remaining arguments are an optional PR number and flags.

> **Security caveat:** running `/playbook:deep-review` on an explicit PR installs and runs the PR's code in your local environment: npm preinstall/postinstall hooks, build scripts, test suites. This exposes you to supply-chain attacks and code execution with your credentials in reach. Only run it on PRs you trust to execute locally.

## Help

If the arguments contain `--help`, print this and stop:

```
/playbook:deep-review - Multi-agent PR review with a specialist reviewer swarm

USAGE:
  /playbook:deep-review [PR_NUMBER] [options]

OPTIONS:
  --help            Show this help
  --all             Run every reviewer regardless of diff content
  --quick           Run only the core reviewers
  --preset <name>   Named reviewer set: security | architecture | data | docs | thorough
  --self            Local self-review (never posts to GitHub)

EXAMPLES:
  /playbook:deep-review               Review the current branch's PR (auto-selects reviewers)
  /playbook:deep-review 123           Review PR #123
  /playbook:deep-review 123 --all     PR #123 with every reviewer
  /playbook:deep-review --self        Self-review the current branch, no posting
```

## Reviewer Swarm

**Core reviewers** (run unless a preset narrows the set):

| Reviewer | Focus |
| --- | --- |
| logic | algorithm correctness, edge cases, off-by-one, normalisation, false positives/negatives |
| test | coverage gaps, missing scenarios, untested error paths, weak assertions, flakiness |
| security | auth, PII handling, crypto, injection, IDOR, leaked secrets |
| data | query/DAO correctness, N+1, missing indexes, transaction boundaries |
| types | `any`, unsafe casts (`as`), non-null assertions (`!`), weak typing (language-appropriate) |
| perf | N+1, unbounded data, connection leaks, work inside loops |

**Conditional reviewers** (added in `auto` mode when the diff shows the trigger):

| Reviewer | Trigger |
| --- | --- |
| architecture | new modules, dependency or layer changes, boundary violations |
| big-o | algorithms over collections, sorting, searching, graph traversal |
| complexity | deep nesting, large functions, tight coupling |
| integration | feature flags, events, external APIs, config, deployment changes |
| migration | database migrations, schema changes |
| docs | README changes, breaking changes, new public APIs |
| dedup | many similar files, or a large diff (>300 changed lines) |
| adr | an Architecture Decision Record is added or affected |

**Presets:** `security` = security+data+types+logic · `architecture` = architecture+complexity+big-o+dedup · `data` = data+migration+perf · `docs` = docs+adr · `thorough` = all.

## Execution rules (MUST)

1. Run every bash block for real. Don't simulate.
2. No caching: every invocation is a fresh run, even if you reviewed this PR earlier in the conversation. The code may have changed.
3. No skipping (except steps guarded by a flag the user didn't set).
4. No assumptions: run the command and read the result.
5. Follow the command's gates, not your own.
6. Show real data: tables and reports come from actual output, never placeholders.
7. **No selective filtering at presentation.** After consolidation (Step 4) you present EVERY surviving finding; the user decides what to post in Step 6. (Consolidation's dedup/drop rules are the only removals, and they happen in Step 4, not by hiding findings in Step 5.)
8. **Never ask whether to run.** Invoking `/playbook:deep-review` IS the instruction to run; start immediately.

## Voice rules

Invoke the `grounding-review` and `playbook:writing-style` skills before drafting any finding (same discipline as `/playbook:quick-review`). Non-negotiables for anything posted to GitHub: Conventional Comments label in **plain text** and bare (`issue:`, `suggestion:`, `nitpick:`, `question:`, never bold, no `(blocking)`/`(non-blocking)` decoration on the posted body; that split stays in the local summary); plain, jargon-free language; findings kept short (two sentences by default, the problem then what breaks, a third only when the mechanism is non-obvious); one pragmatic fix, not a menu; no hedging; no meta-justification; no em or en dashes.

## Step 1: Resolve PR and gather context

```bash
ARGS="$ARGUMENTS"
PR_ARG=$(echo "$ARGS" | tr ' ' '\n' | grep -E '^#?[0-9]+$' | head -1 | tr -d '#')

if [[ -n "$PR_ARG" ]]; then
  # Integer or #N: explicit PR number
  PR_NUMBER="$PR_ARG"
else
  PR_ARG=$(echo "$ARGS" | tr ' ' '\n' | grep -vE '^--|^$' | grep -vE '^#?[0-9]+$' | head -1)
  if [[ -n "$PR_ARG" ]]; then
    # Branch name: resolve to its open PR number
    PR_NUMBER=$(gh pr list --head "$PR_ARG" --json number -q '.[0].number' 2>/dev/null)
    [[ -n "$PR_NUMBER" ]] || { echo "error: no open PR for branch $PR_ARG" >&2; exit 1; }
  else
    # Empty: current branch's PR
    PR_NUMBER=$(gh pr view --json number -q .number 2>/dev/null) || { echo "error: no PR for current branch; pass a PR number" >&2; exit 1; }
  fi
fi

REPO=$(gh repo view --json nameWithOwner -q .nameWithOwner)
HEAD_SHA=$(gh pr view "$PR_NUMBER" --json headRefOid -q .headRefOid)
PR_AUTHOR=$(gh pr view "$PR_NUMBER" --json author -q .author.login)
ME=$(gh api /user -q .login)
SELF_REVIEW=$([ "$PR_AUTHOR" = "$ME" ] && echo true || echo false)

REVIEW_JSON="/tmp/$REPO/deep-review-$PR_NUMBER.json"
mkdir -p "$(dirname "$REVIEW_JSON")"

echo "PR: $REPO#$PR_NUMBER  head: $HEAD_SHA  self-review: $SELF_REVIEW"
echo "Review JSON: $REVIEW_JSON"

gh pr view "$PR_NUMBER"
gh pr diff "$PR_NUMBER"

# Decide: review in-place or via isolated worktree
LOCAL_HEAD=$(git rev-parse HEAD 2>/dev/null)
DIRTY=$(git status --porcelain --untracked-files=no 2>/dev/null)
if [[ "$LOCAL_HEAD" == "$HEAD_SHA" && -z "$DIRTY" ]]; then
  WT=""
  WT_CREATED=false
  echo "Mode: in-place (HEAD matches, tree clean)"
else
  if [[ ! -r "$HOME/.claude/shell/review-worktree.sh" ]]; then
    echo "error: review-worktree.sh not found under \$HOME/.claude/shell/; install the repo's shell/ directory" >&2
    exit 1
  fi
  WT_ERR="$(mktemp)"
  WT="$(bash "$HOME/.claude/shell/review-worktree.sh" setup "$PR_NUMBER" "$HEAD_SHA" 2>"$WT_ERR")"
  if [[ $? -ne 0 || -z "$WT" ]]; then
    echo "error: worktree setup failed: $(cat "$WT_ERR")" >&2
    rm -f "$WT_ERR"
    exit 1
  fi
  rm -f "$WT_ERR"
  WT_CREATED=true
  echo "Mode: worktree at $WT"
fi
```

Capture `REPO`, `PR_NUMBER`, `HEAD_SHA`, `SELF_REVIEW`, `REVIEW_JSON`. `--self` forces self-review mode (never posts), regardless of authorship.

In worktree mode, `WT` holds the absolute path to the isolated checkout and `WT_CREATED=true`. In in-place mode, both are empty/false. Subagents use `$WT` for all reads; if empty, they read from the local working tree.

## Step 2: Select reviewers

- `--quick` → core reviewers only.
- `--all` → every core + conditional reviewer.
- `--preset <name>` → that preset's set.
- otherwise (`auto`, default) → all core reviewers, plus each conditional reviewer whose trigger appears in the diff from Step 1 (grep the diff for migration dirs, schema files, feature flags, new modules, ADR files, >300 changed lines, etc.). Report which reviewers you selected and why.

## Step 2b: Run checks in the worktree (best-effort, worktree mode only)

Skip this step entirely when `WT_CREATED` is false (in-place mode).

In the worktree (`cd "$WT"`), detect the toolchain and run the full check suite once. Capture stdout+stderr:

```bash
cd "$WT"
CHECK_OUTPUT=""

if [[ -f package.json ]]; then
  npm install --prefer-offline 2>&1 | tail -5
  CHECK_OUTPUT=$(npm run typecheck 2>&1; npm run lint 2>&1; npm test 2>&1) || true
elif [[ -f pyproject.toml ]] || [[ -f setup.py ]]; then
  pip install -e . -q 2>&1 | tail -3
  CHECK_OUTPUT=$(python -m mypy . 2>&1; python -m pytest 2>&1) || true
elif [[ -f go.mod ]]; then
  CHECK_OUTPUT=$(go vet ./... 2>&1; go test ./... 2>&1) || true
elif [[ -f Cargo.toml ]]; then
  CHECK_OUTPUT=$(cargo check 2>&1; cargo test 2>&1) || true
else
  CHECK_OUTPUT="[no recognised toolchain; checks skipped]"
fi

# Print so the orchestrating session can read it and embed it in subagent prompts
printf '%s\n' "$CHECK_OUTPUT"
```

If install or run fails, log the error in `CHECK_OUTPUT` and continue: never block the review. The `printf` at the end makes the output visible in the tool result so Step 3 can embed it verbatim in each subagent prompt.

## Step 3: Spawn the reviewer swarm (parallel reviewer subagents)

Spawn the selected reviewers **in parallel** (one message, multiple `Agent` calls), each as a `reviewer` subagent (`subagent_type: reviewer`). The `reviewer` agent is structurally read-only (Read/Grep/Glob only, no Edit/Write/Bash) and pins its own model tier and the `grounding-review` + `grounding-research` discipline, so the orchestrator no longer sets `model` per call. Each reviewer prompt MUST include: its focus area (from the table), the PR diff and `HEAD_SHA`, the `grounding-review` + `grounding-research` discipline, the absolute `$WT` path (or a note that the tree is in-place if `WT` is empty) with the instruction "Read and grep files under <WT>; do not install or build anything.", and the `CHECK_OUTPUT` captured in Step 2b verbatim under a heading "Check suite output (from orchestrator)".

In worktree mode, each reviewer prompt includes the absolute `$WT` path with the instruction "read and grep files under $WT; do not install or build." Subagents are read-only. The orchestrator has already run the checks once in Step 2b; subagents use the captured output as context, not as a trigger to re-run.

Instruct each to:

- Read every file it cites at `HEAD_SHA` (diff context alone is insufficient); quote exact code with `file:line`; tag anything unconfirmed `[unverified]`.
- Stay within its focus; don't report issues another reviewer owns.
- Return findings as a JSON array, one object per finding:

```json
{"file": "...", "line": N, "side": "RIGHT", "label": "issue", "decoration": "blocking",
 "category": "security", "confidence": "HIGH", "evidence": "<exact code>", "body": "<short plain finding; the problem then what breaks, 2 sentences by default, a third only when the mechanism is non-obvious>"}
```

Collect every reviewer's JSON. If a reviewer returns nothing, record it ran with zero findings (not a failure).

**Close each reviewer the moment it returns (MUST).** Spawn each with a stable `name` (e.g. `dr-<focus>`: `dr-security`, `dr-logic`). As soon as a reviewer returns its JSON, `TaskStop` it. The swarm is one-shot, so a returned reviewer is never reused; a spawned agent stays idle-alive for `SendMessage` follow-ups, so leaving it unstopped keeps a subagent running in the background. Track the spawned names so Step 8 can sweep any that didn't return.

## Step 4: Consolidate and fact-check

Merge all findings, then (this is where removals happen):

- **Dedup across reviewers:** same file + nearby lines + same concern → keep the one with the stronger label, drop the rest.
- **Merge same-line findings:** two+ findings within 3 lines → one comment at the highest severity, combining the points.
- **Drop out-of-scope:** a finding on a file not in the PR diff is dropped UNLESS the PR's change breaks it (typecheck failure, runtime error, broken import). Preexisting-style nits outside the diff are dropped.
- **Filter already-addressed:** fetch existing review comments (`gh api --paginate /repos/$REPO/pulls/$PR_NUMBER/comments`), drop findings that duplicate one, and attribute by name ("already raised by @user").
- **Fact-check (orchestrator):** for each surviving finding, read the file at `HEAD_SHA` and confirm the evidence appears at the cited line; resolve or remove `[unverified]` tags; correct drifted line numbers; drop fabricated findings.
- **Drop non-actionable:** positive observations or asides with no concrete "do X" → always drop; `nitpick` → drop from an APPROVE review unless asked.
- **Verdict + confidence:** APPROVE / REQUEST_CHANGES / COMMENT / INCONCLUSIVE. **INCONCLUSIVE (never APPROVE)** if the swarm failed to run; say why. Confidence HIGH/MEDIUM/LOW.

## Step 5: Present the consolidated report

Present ALL surviving findings (rule 7). Render the `grounding-review` Review Report Format exactly, INCLUDING the `### Reviewers` line (which reviewers ran, findings per reviewer, e.g. "security 2 · logic 1 · perf 0"). Each finding carries its `Post:` block (the exact GitHub comment), or `Report-only: not on a changed line, no inline draft.` when the evidence is not on a changed diff line.

## Step 6: Orchestrate posting

If `--self` (or self-review with nothing postable), stop here: the report IS the deliverable, no GitHub posting.

Otherwise ask **one question at a time**:

- **Q1:** "Post which findings as a pending review? all / none / a subset (numbers)." If `none`, stop.
- Build the payload at `$REVIEW_JSON` (`{"commit_id": "<HEAD_SHA>", "comments": [{"path","line","side","body"}, ...]}`, body starting with the plain-text bare Conventional Comment label (`issue:`, no `(blocking)`/`(non-blocking)` decoration), no review `body`). Build each inline comment's `body` from that finding's `Post:` block verbatim, anchored to the finding's `file:line`. What the user read in the report is exactly what posts. Skip any finding marked `Report-only`. Then create the pending review:

```bash
gh api -X POST /repos/$REPO/pulls/$PR_NUMBER/reviews --input "$REVIEW_JSON" --jq '{id, state, html_url}'
```

- **Pre-post verification (MUST):** before this call, re-read each selected finding's file at `HEAD_SHA`, confirm the evidence is at the cited line (correct silently if it drifted, drop if absent), and confirm the PR is still OPEN and not CONFLICTING (`gh pr view "$PR_NUMBER" --json state,mergeable`). Don't post on a merged/closed/conflicting PR.
- **Q2:** "Submit verb? approve / comment / request-changes / skip." Self-review offers only `comment` / `skip` (GitHub rejects approve/request-changes from the author). On `skip`, leave it PENDING. Otherwise:

```bash
gh api -X POST /repos/$REPO/pulls/$PR_NUMBER/reviews/$REVIEW_ID/events -f event=<APPROVE|COMMENT|REQUEST_CHANGES>
```

Never fabricate URLs; use the `html_url` the API returns.

## Step 7: Capture and wrap up

- If a project store is present at `~/.claude/memory/<owner>/<repo>/` (derive `<owner>/<repo>` from `git remote get-url origin`), persist each POSTED blocking/non-blocking finding as a project memory fact (`type: project`, tag it a review gotcha, `anchors:` to the file), deduping against existing memory first. Skip suggestions/nitpicks and anything not posted; if there's no project store, skip silently.
- If non-self-review and the PR has unaddressed review threads, offer to run `/playbook:address-pr-comments $PR_NUMBER`.
- Final message: one line per outcome (pending review id + count, or submitted verb + timestamp).

## Step 8: Teardown (MUST run, even on failure, abort, or skip)

**Stop every reviewer subagent first.** `TaskStop` each reviewer spawned in Step 3 that is still alive (any you didn't already close on return). Use `TaskList` to confirm none from this swarm are still running before you finish. A returned agent stays idle-alive for follow-ups and this review never sends any, so an unstopped reviewer lingers as a background process. Do this whether the review completed, failed, was skipped, or aborted mid-swarm.

Then, if `WT_CREATED` is true, always run:

```bash
bash "$HOME/.claude/shell/review-worktree.sh" teardown "$WT"
```

Run this whether the review completed, failed, was skipped, or was aborted mid-swarm. It's a no-op if the worktree is already gone.

## Anti-patterns to refuse

1. Presenting a padded review: dedup, merge, and drop per Step 4, but never hide a surviving finding from the user in Step 5.
2. APPROVE when the swarm didn't actually run. Zero findings from a broken swarm is INCONCLUSIVE, not LGTM.
3. Auto-submitting without the two-question orchestration.
4. Posting findings in the review body instead of inline.
5. Fabricated evidence, line numbers, or URLs. Verify against the file at HEAD; use API-returned URLs.
