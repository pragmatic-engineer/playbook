---
description: Use for substantial, risky, or cross-cutting PRs. A swarm of specialist reviewer subagents (logic, test, security, data, types, perf, plus conditional) run in parallel, consolidated and fact-checked, then posted as a pending GitHub review. Heavier than /playbook:quick-review.
allowed-tools: Bash, Read, Grep, Glob, Write, Agent, Skill
argument-hint: "[PR number] [--all] [--preset <name>] [--self] [--this-pr] [--whole-stack] [--auto] [--ask] [--help]"
model: opus
effort: high
---

# Deep Review: Multi-Agent PR Review

Review a pull request with a swarm of specialist reviewer subagents run in parallel, each a focused `reviewer` subagent under the `playbook:grounding-review` discipline. The orchestrating session consolidates, dedups, and fact-checks their findings, then posts them as a **pending** GitHub review (same posting flow as `/playbook:quick-review`). This is heavier and slower than `/playbook:quick-review`; use it for substantial, risky, or cross-cutting PRs.

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
  --preset <name>   Named reviewer set: security | architecture | data | docs
  --self            Local self-review, never posts to GitHub (default when no
                    PR number is given, or when the PR is yours)
  --this-pr         When the PR is in a stack, review only this PR
  --whole-stack     When the PR is in a stack, review the whole stack
  --auto            Run unattended; implies --self
  --ask             Force the interactive mode

EXAMPLES:
  /playbook:deep-review               Review the current branch's PR, report only, never posts
  /playbook:deep-review 123           Review and post to PR #123 (unless #123 is your own PR)
  /playbook:deep-review 123 --all     PR #123 with every reviewer, posts (unless it's yours)
  /playbook:deep-review 123 --self    Review PR #123, report only, never posts
```

No PR number given means no posting: with nothing to disambiguate which PR you meant to publish to, the safe default is a local report, same as passing `--self` explicitly. A PR authored by you never posts either, even with a PR number given: GitHub blocks approve/request-changes from the author, and a comment-only review of your own PR has no independent reviewer behind it, so it gets the same report-only treatment as `--self`. Pass a PR number for someone else's PR to post.

## Step 0: Read the run mode

Do this first, before Step 1. Read the mode from the CLI:

```bash
playbook mode status --json
```

If the arguments contain `--auto`, add `--flag auto`. If they contain `--ask`, add `--flag ask`. If both are present, stop with one line: "--auto and --ask conflict; pass one." The JSON has four keys: `mode` (`ask` or `auto`), `source` (where it came from), `hook_mode` and `warning`. If `warning` is not empty, print it once. If the command fails, do not stop and do not retry. Run in ask mode and print one line. When the error is `unrecognized subcommand 'mode'`, the installed binary is older than this plugin: print "playbook binary is older than the plugin, run `playbook update`."

**Effort ceiling.** Also run `playbook effort resolve commands deep-review --json`. Its `ceiling` is the highest effort the user allows for this run (`null` means no limit). Hold your own work to it, and before you spawn an agent follow the `delegating-subagents` skill, which runs `playbook effort resolve agents <agent>` and uses the `subagentType` it returns. If the command fails, carry on with no ceiling.

- **`ask` mode:** behave exactly as this file describes.
- **`auto` mode:** follow the Auto path below.

### Auto path

Auto mode implies `--self`: treat the arguments as if `--self` were passed. Set `RUN_MODE=auto` at the top of the Step 1 block (use `ask` otherwise), so `SELF_MODE` is true. The review runs and reports locally, and nothing is posted to GitHub, because a posted review speaks as you.

## Reviewer Swarm

**Core reviewers** (run unless a preset narrows the set):

| Reviewer | Focus |
| --- | --- |
| logic | algorithm correctness, edge cases, off-by-one, normalisation, false positives/negatives |
| test | coverage gaps, missing scenarios, untested error paths, weak assertions, flakiness |
| security | auth, PII handling, crypto, injection, IDOR, leaked secrets |
| data | query/DAO correctness, N+1, missing indexes, transaction boundaries |
| types | `any`, unsafe casts (`as`) instead of runtime parsing at a boundary, non-null assertions (`!`), weak typing (language-appropriate) |
| perf | N+1, unbounded data, connection leaks, work inside loops |

**Conditional reviewers** (added by default when the diff shows the trigger):

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

**Presets:** `security` = security+data+types+logic · `architecture` = architecture+complexity+big-o+dedup · `data` = data+migration+perf · `docs` = docs+adr. For every reviewer regardless of diff content, use `--all` directly.

## Execution rules (MUST)

1. Run every bash block for real with the `Bash` tool (capital B, tool names are case-sensitive). Don't simulate.
2. No caching: every invocation is a fresh run, even if you reviewed this PR earlier in the conversation. The code may have changed.
3. No skipping (except steps guarded by a flag the user didn't set).
4. No assumptions: run the command and read the result.
5. Follow the command's gates, not your own.
6. Show real data: tables and reports come from actual output, never placeholders.
7. **No selective filtering at presentation.** After consolidation (Step 4) and the verification sweep (Step 4b) you present EVERY surviving finding; the user decides what to post in Step 6. (Consolidation's dedup/drop rules and the sweep's drops are the only removals, and they happen in Steps 4 and 4b, not by hiding findings in Step 5.)
8. **Never ask whether to run.** Invoking `/playbook:deep-review` IS the instruction to run; start immediately.

## Voice rules

Findings are plain: label, `file:line`, evidence, a short failure scenario, and one fix in plain words, with no comment body (same discipline as `/playbook:quick-review`). Step 5 drafts the comments after the sweep, and only when something can be posted. Non-negotiables for every finding: Conventional Comments label in **plain text** and bare (`blocking:`, `issue:`, `suggestion:`, `nitpick:`, `question:`, never bold; `blocking:` replaces `issue:` specifically for a merge-blocking finding); plain, jargon-free language; findings kept short (one sentence when possible, two at most, the problem then what breaks, a second only when the mechanism is non-obvious); one pragmatic fix, not a menu; no hedging; no meta-justification; no em or en dashes.

## Step 1: Resolve PR and gather context

```bash
playbook review prepare deep "$ARGUMENTS"
```

Add `--auto` when Step 0 read auto mode (auto implies report-only). The command resolves the PR (a number or `#N`, else a branch name, else the current branch's PR with report-only), works out `SELF_MODE`, decides in-place or worktree and sets the worktree up, creates the folder of `REVIEW_JSON`, and prints these lines: `PR`, `REPO`, `PR_NUMBER`, `HEAD_SHA`, `AUTHOR`, `SELF_REVIEW`, `SELF_MODE`, `MODE`, `WT` (empty when in place) and `REVIEW_JSON`. On a problem it prints `error: ...` and exits 1: stop and show it. Then read the PR with the number it printed:

```bash
gh pr view <PR_NUMBER>
gh pr diff <PR_NUMBER>
```

Capture `REPO`, `PR_NUMBER`, `HEAD_SHA`, `SELF_REVIEW`, `SELF_MODE`, `REVIEW_JSON`. `SELF_MODE` is true, and posting is skipped entirely, when `--self` is passed explicitly, when no PR number/branch was given in `$ARGUMENTS` at all (nothing named to post to), when the run mode is auto, or when `SELF_REVIEW` is true (the resolved PR is authored by the caller). `SELF_REVIEW` stays a separate fact purely for logging (the status line prints it independently), but it never leaves posting partially enabled on its own: once it is true, `SELF_MODE` is true too, so Step 6 never reaches the submit-verb question in the first place.

In worktree mode, `WT` holds the absolute path to the isolated checkout. In in-place mode it is empty. Subagents use `$WT` for all reads; if empty, they read from the local working tree.

## Step 1b: Stack check

Right after Step 1, run:

```bash
playbook skill ref grounding-review stack-review
```

Read the file it prints and follow it. A PR that is not in a stack gets no question and nothing changes in this file. When the PR is in a stack, that file decides the scope (this PR only, the whole stack, or the whole stack with deep review), builds one shared context, and says how Steps 2 to 4 and the posting step differ for a whole stack. Strip `--this-pr` and `--whole-stack` before reading the other arguments.

## Step 2: Select reviewers

- `--all` → every core + conditional reviewer.
- `--preset <name>` → that preset's set.
- otherwise (default) → all core reviewers, plus each conditional reviewer whose trigger appears in the diff from Step 1 (grep the diff for migration dirs, schema files, feature flags, new modules, ADR files, >300 changed lines, etc.). Report which reviewers you selected and why.

## Step 2b: Run checks in the worktree (best-effort, worktree mode only)

Skip this step entirely when `WT` is empty (in-place mode).

Run the project's own check suite once in the worktree. `playbook review checks` detects the toolchain (Node, Python, Go or Rust) and prints the combined output:

```bash
playbook review checks "$WT"
```

A failing install or check is printed as output, never an error: never block the review. Keep the output as `CHECK_OUTPUT` so Step 3 can embed it verbatim in each subagent prompt.

## Step 2c: Load memory (best-effort)

Run `playbook memory context --repo <owner>/<repo>` (`<owner>/<repo>` from `git remote get-url origin`), and load the fact files it names on demand. If the command produces no output (empty store, or the `playbook` binary unavailable, indistinguishable from stdout alone), fall back to reading `~/.config/playbook/memory/memory.graph.json` directly with the `Read` tool and picking out nodes whose `scope` is `global`, or whose `project` matches this repo (or its owner, for `org` scope). When both the command and the direct graph read produce nothing, skip this step silently; Step 3's reviewers get no memory section and that's expected, not an error. Note in the report which source produced the result (command output, direct graph read, or nothing found), so an operator can tell "nothing relevant" apart from "the command couldn't run."

## Step 2d: Haiku triage

Skip this step entirely when `--all` was passed (check `$ARGUMENTS` the same way Step 2 does): every core + conditional reviewer Step 2 selected already runs `full-lens`, matching `--all`'s existing "run every reviewer regardless of diff content" guarantee, so there is nothing left for triage to narrow.

Otherwise, dispatch `review-triage` (`subagent_type: playbook:review-triage`) exactly once per review run, regardless of how many lenses Step 2 selected. The prompt includes: the PR diff (the same diff Step 1 already captured via `gh pr diff "$PR_NUMBER"` and that Step 3's reviewer prompts also embed), `HEAD_SHA`, the absolute `$WT` path (or a note that the tree is in-place when `WT` is empty, same convention Step 3 uses), and the full set of lens names Step 2 selected (core reviewers plus any triggered conditional reviewers). Capture the returned tier map, a JSON object of `{lens: {tier, reason}}`, into context for Step 3 to read.

Three distinct fail-open rules apply, not one:

- **Total failure:** if the `review-triage` dispatch itself fails, times out, or returns nothing at all (per `playbook:delegating-subagents`, `review-triage` is structurally read-only so its return value is the only channel and can fail silently), every lens Step 2 selected defaults to `full-lens`.
- **Partial response:** if the dispatch returns a tier map missing one or more of Step 2's selected lenses, each MISSING lens individually defaults to `full-lens`; lenses present in the returned map keep their returned tier. This mirrors the same discipline Step 3's existing "Returned findings / Returned empty array / Returned nothing" tracking table already applies one layer downstream (a silent lens is never folded into "found nothing"), applied here to triage's own output instead of the swarm's findings.
- **Unrecognised tier:** if a lens IS present in the returned map but its `tier` value is anything other than `skip`, `cheap-check`, or `full-lens` (a drifted or malformed classifier response, not schema-validated on the way in), that lens defaults to `full-lens` too, the same as if it were missing. A lens present with garbage in its `tier` field must never fall through Step 3's dispatch-by-tier branches silently: that is the same "swarm becomes a no-op while looking thorough" failure this file already warns against for a lost reviewer, just triggered from triage's side instead of the swarm's.

Report which lenses resolved to which tier, a one-line summary, e.g. "Triage: security=full-lens, docs=cheap-check, perf=skip", the same way Step 2 already reports which reviewers it selected and why.

## Step 2e: Load skills and reference files

Only now, with the lenses and tiers settled, load what the run needs. The orchestrator invokes `playbook:grounding-review` (for the sweep and the report format) and loads no `playbook:writing-style` here. Each dispatched `reviewer` loads `playbook:grounding-review` plus the one reference file mapped to its lens (Step 3). A `cheap-checker` reads only that resolved file. A skipped lens loads nothing.

## Step 3: Spawn the reviewer swarm (parallel reviewer subagents)

**Concurrency cap (MUST).** Dispatch at most 8 reviewers at once. When the selected set (Step 2) is 8 or fewer, dispatch it in one wave exactly as below. When it's larger (only possible under `--all`, up to 14 lenses), split into waves of at most 8: issue the first wave's `Agent` calls in one message, wait for them to return, `TaskStop` each, then issue the remaining lenses as a second wave. This bounds concurrent spawns; it never drops a lens to stay under the cap; every selected reviewer still runs, just possibly across two waves instead of one.

For each lens in a wave, read its Step 2d tier from the captured tier map before dispatching: a lens absent from the map defaults to `full-lens`, per Step 2d's fail-open-per-lens rule (a triage dispatch that returns a partial map never silently narrows a lens's coverage). Dispatch by tier:

- **`full-lens`:** dispatch a `reviewer` subagent (`subagent_type: playbook:reviewer`, or a `reviewer-low` or `reviewer-xhigh` variant the session has, chosen by "Pick the tier" in `playbook:delegating-subagents`) as everything below through "Instruct each to" describes. It loads `playbook:grounding-review` and, when its lens has a mapped file in the `cheap-check` table below, only that reference file, resolved to an absolute path the same way. It never loads `playbook:writing-style`.
- **`cheap-check`:** dispatch a `cheap-checker` subagent (already pinned to low effort, so no tier variant applies) (`subagent_type: playbook:cheap-checker`) instead of `reviewer`. Its prompt names: the lens's narrow concern, taken from the tier map's `reason` field for that lens (that field is already a short, grounded justification from `review-triage`, so it doubles as the concern statement); the PR diff and `HEAD_SHA`; the absolute `$WT` path (or the in-place note when `WT` is empty), same conventions as the `reviewer` dispatch below; and ONE reference file path to read for criteria, per this mapping:

  | Lens | Reference file |
  |---|---|
  | security | security.md |
  | perf | performance.md |
  | data | performance.md |
  | logic | correctness.md |
  | types | reliability.md |
  | architecture | architecture.md |
  | migration | architecture.md |
  | big-o | performance.md |
  | complexity | maintainability.md |
  | dedup | maintainability.md |
  | integration | reliability.md |
  | test | (none, no matching category) |
  | docs | (none, no matching category) |
  | adr | architecture.md |

  `types` maps to `reliability.md`, not `correctness.md`: that file's "cast with `as` instead of parsed with a runtime schema validator" bullet is the one that actually matches the `types` lens's stated focus (unsafe casts, non-null assertions), and `correctness.md` has no bullet about either.

  Path resolution: resolve the actual value of `$CLAUDE_PLUGIN_ROOT` with a real bash step before building the string, inside an executed bash block, not prose that merely names the variable, for example:

  ```bash
  playbook skill ref grounding-review <file>
  ```

  It prints the absolute path of `references/<file>.md` (give `<file>` without `.md`), or of the full `SKILL.md` when the file is missing or no name is given.

  If the lens has a mapped file, resolve it to an absolute path this way and confirm that file exists. If a lens has no mapped file (`test`, `docs`), or the resolved file doesn't exist for some reason (defensive fallback), resolve the full `SKILL.md` path instead: the same fallback mechanism either way (no reference file to hand over), so it is one rule, not two. Hand `cheap-checker` the resolved ABSOLUTE path this bash step produced, never the unexpanded `${CLAUDE_PLUGIN_ROOT}` placeholder or a bare repo-relative string: `cheap-checker` has no `Bash`, so it cannot expand `$CLAUDE_PLUGIN_ROOT` itself, and a repo-relative path never resolves against the diff's own target repo (which is not this plugin's repo). The narrow concern text, not the reference file, is what scopes the check, so falling back to the full `SKILL.md` for criteria still returns a finding scoped to just that lens's concern, never the full skill's breadth.
- **`skip`:** dispatch nothing for that lens. Track it explicitly as skipped, e.g. in the same one-line summary Step 2d already reports ("Triage: security=full-lens, docs=cheap-check, perf=skip"). A skipped lens is never conflated with "returned nothing" below: it was never dispatched at all, so it has no return value to lose.

Spawn each wave **in parallel** (one message, multiple `Agent` calls). The `reviewer` agent is structurally read-only (Read/Grep/Glob only, no Edit/Write/Bash) and pins its own model tier, so the orchestrator no longer sets `model` per call; `cheap-checker` is the same shape (Read/Grep/Glob/Skill, its own pinned `haiku` model). Each reviewer prompt MUST include: its focus area (from the table), the PR diff and `HEAD_SHA`, the instruction to load `playbook:grounding-review` (plus its mapped reference file), the absolute `$WT` path (or a note that the tree is in-place if `WT` is empty) with the instruction "Read and grep files under <WT>; do not install or build anything.", the `CHECK_OUTPUT` captured in Step 2b verbatim under a heading "Check suite output (from orchestrator)", and, when Step 2c loaded anything, a memory slice: facts anchored to a file touched in the diff, or otherwise related to the lens's focus area (for example, security-tagged facts for the security lens), listed by one-line hook or short body under a heading "Relevant memory (from orchestrator)". No matching facts means no section, not an empty placeholder.

In worktree mode, each reviewer prompt includes the absolute `$WT` path with the instruction "read and grep files under $WT; do not install or build." Subagents are read-only. The orchestrator has already run the checks once in Step 2b; subagents use the captured output as context, not as a trigger to re-run.

For full-lens dispatches, instruct the reviewer to:

- Read every file it cites at `HEAD_SHA` (diff context alone is insufficient); quote exact code with `file:line`; tag anything unconfirmed `[unverified]`.
- Stay within its focus; don't report issues another reviewer owns.
- Return plain findings as a JSON array, one object per finding, with no comment body and no `Post:` block:

```json
{"file": "...", "line": N, "side": "RIGHT", "label": "blocking",
 "category": "security", "confidence": "HIGH", "evidence": "<exact code>", "body": "<short plain finding; the problem then what breaks, 1 sentence when possible, a second only when the mechanism is non-obvious>", "fix": "<one fix, in plain words>"}
```

**A silent reviewer is NOT a reviewer with zero findings.** This is the single most important rule in this command (`playbook:delegating-subagents`).

The `reviewer` agent is structurally read-only (Read, Grep, Glob, Skill), so it cannot write its findings to a file. `playbook agents check` forbids `Write` and `Bash` for that tier by design, and granting them would fail CI, so the return value is the only channel there is. In measured use, Agent-tool return values have failed outright. Treat a swarm as likely to lose lenses.

Track three outcomes per dispatched lens (whichever of `reviewer` or `cheap-checker` its tier actually sent) and carry them into Step 4's verdict:

| Outcome | Meaning |
|---|---|
| Returned findings | Ran, use them. |
| Returned an explicit empty array | Ran, found nothing. Safe. |
| Returned nothing | **NOT RUN. Not a clean lens.** |

Never fold "returned nothing" into "zero findings", and **never let a swarm with missing lenses produce an APPROVE.** Step 4 already requires INCONCLUSIVE when the swarm failed to run, and a missing security or logic lens is exactly that case, whether it was dispatched as a `reviewer` or a `cheap-checker`. Name the missing lenses in the report and to the user, alongside any lens this run skipped by design at Step 2d's `skip` tier: a skipped lens is a deliberate triage decision, not a lost one, and the report should not blur the two.

**Cover the gap while the swarm runs.** Start your own grounding pass on the highest-risk part of the diff as soon as the swarm is dispatched, rather than waiting to see what comes back. A lost swarm then costs latency instead of coverage, which is the difference between a slow review and a review that only looked thorough.

After a reviewer's idle notification fires, one `SendMessage` asking for partial results is worth a single attempt. Do not spend more than one round per lens.

**Close each reviewer once you have its findings or have given up (MUST).** Spawn each with a stable `name` (e.g. `dr-<focus>`: `dr-security`, `dr-logic`). The swarm is one-shot, so a finished reviewer is never reused; a spawned agent stays idle-alive for `SendMessage` follow-ups, so leaving it unstopped keeps a subagent running in the background. Track the spawned names so Step 8 can sweep any that never delivered. `TaskStop` is destructive and unrecoverable for a read-only agent, so do not use it until you have either taken the findings or made the one recovery attempt.

**Trust gate.** This tiered dispatch mechanism ships and functions as soon as this Work Unit lands: a `full-lens` tier still gets the exact reviewer it always did, a `cheap-check` tier gets a real narrow-scope pass from `cheap-checker`, and a `skip` tier is a real, tracked decision to run nothing. But a `skip` or `cheap-check` decision should not be treated as validated judgment yet: `playbook eval review-triage` (a later Work Unit in this plan) has not yet recorded a pass verdict against a real fixture set. Until it has, treat triage's tier choices as best-effort, not proven: a `skip` verdict is not yet evidence a lens truly had nothing to find, and a `cheap-check` narrow pass is not yet guaranteed to have caught everything the full lens would have.

## Step 4: Consolidate and fact-check

Merge all findings, then (this is where removals happen):

- **Dedup across reviewers:** same file + nearby lines + same concern → keep the one with the stronger label, drop the rest.
- **Merge same-line findings:** two+ findings within 3 lines → one comment at the highest severity, combining the points.
- **Drop out-of-scope:** a finding on a file not in the PR diff is dropped UNLESS the PR's change breaks it (typecheck failure, runtime error, broken import). Preexisting-style nits outside the diff are dropped.
- **Filter already-addressed:** fetch existing review comments (`gh api --paginate /repos/$REPO/pulls/$PR_NUMBER/comments`), drop findings that duplicate one, and attribute by name ("already raised by @user").
- **Fact-check (orchestrator):** done in Step 4b, once the list is merged, so each finding is checked once.
- **Drop non-actionable:** positive observations or asides with no concrete "do X" → always drop; `nitpick` → drop from an APPROVE review unless asked.
- **Verdict + confidence:** APPROVE / REQUEST_CHANGES / COMMENT / INCONCLUSIVE. **INCONCLUSIVE (never APPROVE)** if the swarm failed to run; say why. Confidence HIGH/MEDIUM/LOW.

## Step 4b: Verification sweep (MUST, before presenting or posting)

Run this after Step 4 and before any finding is shown to the user or a pending review is created. You do it yourself, not a reviewer: reviewers are read-only and can be wrong. Check every surviving finding against the code at `HEAD_SHA` (under `$WT` in worktree mode):

1. **True.** Re-read the cited lines. Trace or run the failure scenario where that is cheap. A finding tagged `[unverified]` is either confirmed, dropped, or kept with the `[unverified]` tag stated plainly.
2. **Label.** The label (`blocking`, `issue`, `suggestion`, `question`, `nitpick`) matches the real impact.
3. **Anchor.** The file and line are right. In a stacked or multi-PR review, the finding sits on the PR or branch that owns the code.

Drop findings that do not hold, relabel the mislabelled, and move the misplaced. Read the cited lines plus what the trace needs, never whole files. Then put a `Sweep:` line under the Overview with the counts: kept, dropped, relabelled, moved, and recompute the verdict, confidence and finding order from the swept list (Step 4 set them before the sweep). See the Verification Sweep section of `playbook:grounding-review`.

## Step 5: Present the consolidated report

Show this plain report only in `SELF_MODE` or when nothing is postable. Otherwise it is shown once, with the drafts below. Present ALL findings that survived the Step 4b sweep (rule 7), with its `Sweep:` line under the Overview. Render the `playbook:grounding-review` Review Report Format exactly, INCLUDING the `### Reviewers` line. List every lens Step 2 selected, including any Step 2d resolved to `skip`, so a reader can see what was deliberately not looked at, not just what fired. Show each lens's Step 2d tier alongside its finding count: a `full-lens` or `cheap-check` lens renders `<lens>: <tier> (<count>)` (tier written as `full` or `cheap-check`); a `skip` lens renders `<lens>: skip` with NO count, since it never ran and a count of 0 would misleadingly read the same as "ran and found nothing". For example: "security: full (2) · docs: cheap-check (0) · perf: skip". Each finding is plain: label, `file:line`, evidence, the failure, and one fix. A finding whose evidence is not on a changed diff line ends with `Report-only: not on a changed line, no inline comment.`

### Draft the comments (non-self runs only)

Skip this in `SELF_MODE`, or when nothing is postable: the plain report above is the deliverable, no comment is drafted, and Step 6 stops. Otherwise, with the sweep done, load `playbook:writing-style` now. Draft one comment body for every swept finding that is not `Report-only`, from its label, problem, consequence and fix, applying that skill's GitHub rules. Each body starts with the bare plain-text label, never bold, and carries no `file:line` prefix. It MAY hold a ```suggestion``` block when the fix is mechanical.

Present the report with each draft shown under its finding as a `Draft:` block. Each draft is the final text, in the humane `playbook:writing-style` register, not the terse operator voice: label, voice, no dashes, GitHub rules all applied. Nothing is rewritten between this preview and the post, so what the user reads is exactly what posts. If the user asks for a change to a draft, redraft it with `playbook:writing-style` and show the new preview before posting.

## Step 6: Orchestrate posting

If `SELF_MODE` (`--self` passed explicitly, no PR number/branch was given so `PR_NUMBER` came from the current-branch fallback, the run mode is auto, or the resolved PR is authored by the caller), or nothing postable, stop here: the report IS the deliverable, no GitHub posting.

Otherwise, **print every finding first (MUST).** Before asking anything, print all surviving findings in the chat, none left out and none summarized away: a code (F1, F2, and so on), the label, severity, `file:line` and a one-line description each. With zero findings, say so. The questions come only after this list, and the options refer to findings by code. A question with no list above it is a bug (#593).

Then ask **one question at a time**:

- **Q1:** "Post which findings as a pending review?" Offer exactly these six tiers, each a strict superset of the one before, blocking and questions take precedence, suggestions and nitpicks stay optional:
  - `only blockers` (`blocking` findings only)
  - `all issues` (`blocking` + `issue`)
  - `all issues + questions` (`blocking` + `issue` + `question`)
  - `all except nitpicks` (`blocking` + `issue` + `question` + `suggestion`)
  - `all findings` (everything, `nitpick` included)
  - `none` (stop, nothing posted)

  If `none`, stop.
- Build the payload at `$REVIEW_JSON` (`{"commit_id": "<HEAD_SHA>", "comments": [{"path","line","side","body"}, ...]}`, no review `body`). Build each inline comment's `body` from that finding's previewed `Draft:` block verbatim, anchored to the finding's `file:line`. Post the previewed bodies as they are, never a rewording. Skip any finding marked `Report-only`. Then create the pending review:

```bash
gh api -X POST /repos/$REPO/pulls/$PR_NUMBER/reviews --input "$REVIEW_JSON" --jq '{id, state, html_url}'
```

- **Pre-post verification (MUST):** the Step 4b sweep already settled each finding's truth, label and anchor. Before this call, only confirm the PR is still OPEN and not CONFLICTING (`gh pr view "$PR_NUMBER" --json state,mergeable`). Don't post on a merged/closed/conflicting PR.
- **Q2:** "Submit verb? approve / comment / request-changes / skip." Reaching this question already means `SELF_MODE` was false, so the PR is never self-authored here and all four verbs are always valid; GitHub's author restriction is exactly why `SELF_REVIEW` forces `SELF_MODE` earlier instead of trying to offer a narrower menu here. On `skip`, leave it PENDING. Otherwise:
- **Q3:** "Add a comment for the review?" (optional free text, blank to skip). Leave `BODY` empty on a blank answer, except: on `approve` with a blank answer, default `BODY` to `LGTM`.

```bash
if [ -n "$BODY" ]; then
  gh api -X POST /repos/$REPO/pulls/$PR_NUMBER/reviews/$REVIEW_ID/events -f event=<APPROVE|COMMENT|REQUEST_CHANGES> -f body="$BODY"
else
  gh api -X POST /repos/$REPO/pulls/$PR_NUMBER/reviews/$REVIEW_ID/events -f event=<APPROVE|COMMENT|REQUEST_CHANGES>
fi
```

Never fabricate URLs; use the `html_url` the API returns.

## Step 7: Capture and wrap up

- If a project store is present at `~/.config/playbook/memory/<owner>/<repo>/` (derive `<owner>/<repo>` from `git remote get-url origin`), persist each POSTED blocking/non-blocking finding as a project memory fact (`type: project`, tag it a review gotcha, `anchors:` to the file), deduping against existing memory first. Skip suggestions/nitpicks and anything not posted; if there's no project store, skip silently.
- If non-self-review and the PR has unaddressed review threads, offer to run `/playbook:address-pr-comments $PR_NUMBER`.
- Final message: one line per outcome (pending review id + count, or submitted verb + timestamp).

## Step 8: Teardown (MUST run, even on failure, abort, or skip)

**Stop every reviewer subagent first.** `TaskStop` each reviewer spawned in Step 3 that is still alive (any you didn't already close on return). Use `TaskList` to confirm none from this swarm are still running before you finish. A returned agent stays idle-alive for follow-ups and this review never sends any, so an unstopped reviewer lingers as a background process. Do this whether the review completed, failed, was skipped, or aborted mid-swarm.

Then, if `WT` is not empty, always run:

```bash
playbook worktree review teardown "$WT"
```

Run this whether the review completed, failed, was skipped, or was aborted mid-swarm. It's a no-op if the worktree is already gone.

## Anti-patterns to refuse

1. Presenting a padded review: dedup, merge, and drop per Step 4, but never hide a surviving finding from the user in Step 5.
2. APPROVE when the swarm didn't actually run. Zero findings from a broken swarm is INCONCLUSIVE, not LGTM.
3. Auto-submitting without the two-question orchestration.
4. Posting findings in the review body instead of inline.
5. Fabricated evidence, line numbers, or URLs. Verify against the file at HEAD; use API-returned URLs.
