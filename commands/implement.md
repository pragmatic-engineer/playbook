---
description: Execute an approved plan or ADR blueprint (from /playbook:scope or /playbook:adr) on Sonnet, delegating edits to subagents and committing each Work Unit. Delivers the plan as PR-sized Segments (savepoint commits, one small pull request per Segment; independent off the default branch when Segments are disjoint, stacked only when they truly depend on each other), asking the delivery strategy up front. Then runs one refinement pass (self quick-review + SOLID/DRY/KISS/YAGNI simplify, re-planned and executed autonomously) and an adversarial review. Execute-only; it does not design new scope.
allowed-tools: Bash, Read, Grep, Glob, Write, Edit, Agent, Skill
argument-hint: "[plan | adr-blueprint | #issue | KEY-123 | ./spec.md | text] [--auto] [--no-tdd] [--force] [--pr-strategy=<stacked|independent|single>] [--boundary=<savepoint|pause>] [--all-lenses] [--help]"
model: sonnet
effort: high
---

# Implement: Execute a Verified Plan

Execute an approved implementation plan or ADR blueprint. **This command is execute-only: it does NOT design or plan new scope.** Produce the plan with `/playbook:scope` (or `/playbook:adr` for an architectural decision) first, then implement it here. The one exception is the Step 8 refinement pass, which re-plans and applies behaviour-preserving cleanups to the code it just wrote (never new features).

**Incremental delivery.** `/playbook:implement` delivers the plan as PR-sized **Segments**, not one big change: it executes Segment by Segment, commits each Work Unit as a savepoint, and opens one small pull request per Segment (independent off the default branch when Segments are disjoint, stacked only when they truly depend on each other). Before executing, it asks how to deliver (PR topology and Segment-boundary behaviour) and recommends an option based on the plan's scope; under `--auto` it self-selects the recommended options and records them as assumptions. This follows `playbook:engineering-standards`: PRs under 500 lines, one concern each, "ship a sequence of small PRs".

Invoked as `/playbook:implement`. The remaining arguments are the task reference and flags.

## Help

If the arguments contain `--help`, print this and stop:

```
/playbook:implement - Execute an approved plan or ADR blueprint

USAGE:
  /playbook:implement                  List saved plans and pick one to execute
  /playbook:implement <task-reference> [options]

TASK SOURCES:
  Plan file      /playbook:implement $(playbook path plans)/user-avatar-upload.md
  ADR blueprint  /playbook:implement docs/adr/0001-websocket-push-blueprint.md
  GitHub issue   /playbook:implement #42   |   /playbook:implement https://github.com/org/repo/issues/42
  Jira ticket    /playbook:implement PROJ-123   (via Atlassian MCP/acli, when reachable)
  File spec      /playbook:implement ./tasks/feature-spec.md
  Plain text     /playbook:implement "Add user avatar upload"

OPTIONS:
  --help     Show this help
  --auto     Autonomous: execute Segments in dependency order, each Work Unit
             committed as a savepoint, then open the PR set (no prompts;
             self-selects the recommended delivery strategy)
  --no-tdd   Write tests alongside implementation instead of red/green/refactor;
             also presets the TDD-approach question and skips it (default: ask)
  --force    In --auto mode, override quality-gate FAILs (logged)
  --pr-strategy=<stacked|independent|single>
             Preset the PR topology and skip that question (default: ask)
  --boundary=<savepoint|pause>
             Preset the Segment-boundary behaviour and skip that question
             (default: ask; recommended: savepoint)
  --all-lenses
             Skip the Step 9 triage dispatch entirely: run all 5 lenses
             (correctness, behaviour drift, principles, scope, tests) at
             full-lens

DELIVERY: /playbook:implement splits the plan into PR-sized Segments and, before
executing, asks three things (unless preset by flag or running --auto):
  - PR topology: independent (default, disjoint Segments) | stacked (dependency chain) | single
  - Boundary:    savepoint (default) | pause
It honors the plan's Segments but re-splits any whose real diff exceeds the
1500-line hard limit (Segments target under 500). Each Segment becomes one small
pull request.

PLANNING: /playbook:implement never designs. If the reference isn't a ready plan, it
stops and tells you to run /playbook:scope or /playbook:adr first.

REFINEMENT: after implementing, /playbook:implement runs one pass (self quick-review +
SOLID/DRY/KISS/YAGNI simplify, executed autonomously) then an adversarial
review, before opening the PR set (or finishing, per the chosen boundary).
```

## Execution Rules (MUST)

1. **Execute every bash block for real.** Don't simulate or predict output; drive the next step from real output.
2. **No caching.** Every invocation is a fresh run. Don't reuse results from prior conversations or training data.
3. **No skipping.** Execute steps in order. The only exception: steps guarded by a flag the user didn't set.
4. **No assumptions.** Don't guess file contents, command output, or environment state. Run it and read the result.
5. **Follow the command's gates, not your own.** If a step says "ask the user", ask. If it doesn't, don't add a gate.
6. **Show real data.** Tables and reports come from actual command output, never placeholders.
7. **Ground with the right tool for the search's size.** This command's own `allowed-tools` grants `Read`, `Grep`, and `Glob` directly, so a narrow, already-located lookup (a known file, a known symbol, confirming one pattern in one or two calls) MUST use them directly: an ad hoc `Agent` dispatch adds a round trip and returns through the unreliable channel `playbook:delegating-subagents` documents, for no gain when the answer is already cheap to fetch. A broad or open-ended search (the location isn't known, or it fans across many files) is the opposite case: doing that with `Read`/`Grep` yourself pulls raw, unfiltered output into this session's context, where an isolated `Explore` dispatch does the fan-out in its own context and returns only a short digest, keeping this session lean. Delegate that shape instead. Beyond this exploratory use, `Agent` is reserved for this command's named roles (`implementer`, `reviewer`, `critic`, `fact-checker`, `test-reviewer`, `review-triage`, `cheap-checker`) plus the Step 5 haiku brief-drafting call, each with a defined prompt shape and delivery path an improvised dispatch lacks.

## Step 1: Resolve the Task Reference

**No task reference given (empty, or only flags)?** Run the Plan Picker:

```bash
ROOT=$(git rev-parse --show-toplevel 2>/dev/null || pwd)
PLANS_DIR=$(playbook path plans)
found=0
for f in "$PLANS_DIR"/*.md "$ROOT"/docs/adr/*-blueprint.md; do
  [ -f "$f" ] || continue
  case "$f" in *-quality.md) continue;; esac
  found=1
  title=$(grep -m1 '^#\{1,\} ' "$f" | sed 's/^#\{1,\} *//')
  st=$(grep -m1 -iE 'status' "$f" | grep -ioE 'proposed|accepted|implemented' | head -1)
  printf '%s\t[%s]\t%s\n' "${f#"$ROOT"/}" "${st:-?}" "${title:-untitled}"
done
[ "$found" = 0 ] && echo "NO_PLANS"
```

Present the rows as a numbered menu (index, status, title, path), listing unexecuted entries (`Proposed`/`Accepted`) first and `Implemented` last. Ask the user to pick a number, or to preview one first (Read it, show the summary, then re-ask). If the output is `NO_PLANS`, stop and tell the user to run `/playbook:scope` or `/playbook:adr` to create one. Use the chosen file as the task reference, then continue below. Any flags passed (e.g. `--auto`) still apply to the chosen plan.

Otherwise, resolve `$ARGUMENTS` (minus flags) by format:

- **Plan/blueprint file** (a file under `playbook path plans`, `docs/adr/*-blueprint.md`, or any path ending `.md`): Read it. This is the plan.
- **GitHub issue** (`#N`, or a github.com issue URL): `gh issue view <N> --json title,body,url,labels,state`.
- **Jira ticket** (`ABC-123`): use `mcp__atlassian__*` tools if available, else `acli` if present, else ask the user to paste the ticket. (Same availability rule as `/playbook:learn-project`.)
- **Other file path:** Read it.
- **Plain text:** the text is the task statement.

If a fetch fails, ask the user to paste the task content.

## Step 2: Execute-Only Gate (MUST)

Decide whether the resolved reference is a **ready, executable plan**: it names concrete files to change, an ordered set of steps or Work Units, acceptance criteria, and a test plan (Gherkin scenarios or TDD cycles).

- A `/playbook:scope` plan or `/playbook:adr` blueprint → ready. Proceed.
- A spec/issue/ticket detailed enough (explicit files, steps, acceptance criteria, tests) → ready. Proceed.
- **Anything else** (raw text, a thin issue/ticket, a vague request) → **STOP.** Tell the user: "This isn't a ready plan. Run `/playbook:scope` (or `/playbook:adr` for an architectural decision) to produce one, then `/playbook:implement` it." Do NOT generate a plan inline; planning is `/playbook:scope` and `/playbook:adr`'s job. (The Step 8 refinement pass is the sole exception, and only to re-plan refactors of code already written, never new scope.)

If the plan or ADR blueprint ends with a "Confidence + open items" trailer, read it and carry the open items as a watch list through execution and the refinement/adversarial review: treat them as the spots most likely to be wrong, and confirm or resolve each before claiming the work done.

## Step 3: Load Standards and Context

- Invoke the `playbook:engineering-standards` skill (testing requirements, mocking, PR readiness, deployment), the `playbook:grounding-research` skill (verify before asserting), `playbook:delegating-subagents` (every dispatch names an output file and the orchestrator reads it; this command delegates every Work Unit, so it governs the whole run), and `playbook:writing-style` (for any prose, e.g. commit messages and the PR body).
- If a memory store is present, load it: check whether `~/.config/playbook/memory/MEMORY.md` exists and, if so, read it (cross-project preferences, corrections, conventions); check whether `~/.config/playbook/memory/<owner>/<repo>/MEMORY.md` exists (`<owner>/<repo>` derived from `git remote get-url origin`) and, if so, read it, loading the relevant fact files for conventions, gotchas, and prior decisions. Honor the typed edges: a project fact that contradicts a global one wins for this repo, and surface any conflict bearing on the work rather than silently choosing. If neither store is present, skip this step silently and proceed on the codebase and the plan alone.
- **Cost baseline:** find the most recently written `telemetry.jsonl` under `~/.config/playbook/runtime/` (one per session, populated by `statusline.sh` on each render), read its last line, and record the `cost_usd` field as this run's starting cost. No file yet (statusline hasn't rendered this session) means no baseline: Step 7 then reports the cost as unavailable rather than a delta.
- Read every file the plan references before changing it (grounding).
- **Detect the stack** to know the verify commands: check `tsconfig.json` / `package.json` (TS/JS), `pyproject.toml` / `setup.py` (Python), `go.mod` (Go), `Cargo.toml` (Rust). Derive the type-check / lint / test commands from what you find.

**Knowledge capture:** when you discover a durable convention or gotcha, write it as a project memory fact only if a project store is present at `~/.config/playbook/memory/<owner>/<repo>/`; otherwise skip silently.

**Locked index append (MUST, every time this doc writes a `MEMORY.md` index line).** Two `cc` sessions in the same repo can each persist a fact around the same moment; a plain check-then-append can silently drop one of the two lines. Append with the same mkdir-based advisory lock the Rust hooks use (`src/common/atomic.rs`'s `with_dir_lock`): briefly wait for the lock, append regardless of whether it was acquired (never block indefinitely on a stuck lock), remove the lock directory only if this run created it.

```bash
MEMORY_MD=~/.config/playbook/memory/<owner>/<repo>/MEMORY.md
LOCK="$MEMORY_MD.lock"
ACQUIRED=0
for _ in $(seq 1 20); do
  mkdir "$LOCK" 2>/dev/null && { ACQUIRED=1; break; }
  sleep 0.05
done
printf '%s\n' "- [<kebab-title>](<file>.md): <one-line hook>" >> "$MEMORY_MD"
[ "$ACQUIRED" = 1 ] && rmdir "$LOCK" 2>/dev/null
```

## Step 4: Quality Gate (conditional)

If the plan came from `/playbook:scope` or `/playbook:adr` it already has a companion `*-quality.md` report; trust it and skip to Step 5. Otherwise (a file/issue/ticket spec), run the inlined 3-phase gate before executing:

1. **Fact-Check** (`fact-checker` agent): every referenced path exists, signatures/imports match, downstream consumers identified, test infra present. After it returns, write its full raw return text to a file, e.g. `/tmp/<repo>/implement-<plan-slug>-fact-check.txt`, then run `playbook gate record <plan-slug> implement fact-check <that-file>`.
2. **Adversarial Review** (`critic` agent, focus `pre-exec`, + the fact-check report): simpler alternatives, scope creep, missing error paths, blast radius. After it returns, write its full raw return text to a file, e.g. `/tmp/<repo>/implement-<plan-slug>-adversarial.txt`, then run `playbook gate record <plan-slug> implement adversarial <that-file>`.
3. **Test Review** (`test-reviewer` agent): regression-pinning, flakiness, independence, mock quality, assertion strength. After it returns, write its full raw return text to a file, e.g. `/tmp/<repo>/implement-<plan-slug>-test-review.txt`, then run `playbook gate record <plan-slug> implement test-review <that-file>`.

Max 3 iterations per phase; revise on FAIL, recording again after every retry's return so a later PASS overwrites an earlier FAIL (`gate record` upserts on `(plan_slug, phase)`; only the last recorded value before the check below matters).

Before proceeding past this gate, run `playbook gate check <plan-slug> implement fact-check adversarial test-review`. Only continue to Step 4.5 if it exits 0; on a non-zero exit, report exactly which phase(s) are missing or failed, per `gate check`'s own output (copy it verbatim rather than re-narrating it). A FAIL blocks execution unless the user explicitly overrides (or `--auto --force`): the override never changes or fakes `gate check`'s result, it is an explicit, recorded decision to proceed despite a real, honestly reported non-zero exit, not a claim that the gate actually passed. `--force` overrides the block; it must never write a fake PASS into the database. If a project store is present at `~/.config/playbook/memory/<owner>/<repo>/`, record gotchas and rejected alternatives as memory facts, locked append as in Step 3; otherwise skip silently.

## Step 4.5: Delivery Strategy Gate (MUST, before executing)

`/playbook:implement` delivers the plan as PR-sized **Segments** (one concern, one pull request each), not one monolithic change. Settle the delivery shape now, before any code is written.

**1. Resolve Segments.**

- If the plan has a **Segments** table (a `/playbook:scope` plan does), use it: each Segment is an ordered group of Work Units, and the WU table's `Segment` column assigns every WU.
- If it does NOT (an older plan, an issue, or a spec), **derive** Segments here: pack the Work Units into groups targeting under 500 changed lines (never over the 1500 hard limit), in dependency order, one concern per group where the WU titles make the boundary obvious. A tiny plan may be a single Segment.

Either way, confirm the Segment ordering respects the WU `Requires` graph (no forward cross-Segment dependency) before continuing; reorder if needed.

**2. Choose the delivery strategy.** In interactive mode you MUST ask the user before any code is written (this is the "always ask before implementing" gate). Do this by **calling the `AskUserQuestion` tool** with the three questions below in a single call, each option's recommended choice listed first and labelled, recommended per the plan's scope. Do NOT infer the answers, and do NOT start executing Step 5 until the user has answered. The three questions:

- **PR topology:**
  - **Independent off the default branch** (default recommendation): each Segment branches directly off the default branch and opens its own PR against it, no PR based on another PR. Recommend whenever the Segments have disjoint files and no cross-Segment `Requires`.
  - **Stacked:** each Segment branches off the previous one; PR N targets Segment N-1's branch. Recommend only when Segments form a genuine dependency chain (Segment N's code doesn't exist without Segment N-1). Caveat, confirmed 2026-08-26: GitHub's native PR-stack detection routes a base-chained PR set through its async merge API, which (unlike the classic merge API) has no admin/bypass-override parameter. On a repo whose branch protection requires a review the author can't self-grant, an agent can merge every other topology via bypass but cannot merge a stacked chain at all; it needs a human to click through each PR in order. Recommend stacked only when the dependency is real and the user is ready to merge by hand, or knows an independent equivalent isn't practical for this plan.
  - **Single PR:** the current whole-plan behaviour. Recommend for a one-Segment or tiny plan (escape hatch).
- **Segment-boundary behaviour** (always recommend **Savepoints**):
  - **Savepoint commits, PRs at end** (default recommendation, and the norm under `--auto`): implement every Segment as savepoint commits on their (unpushed) branches, run Steps 7-9 once over the full diff, then open the PR set at the end. Because nothing is pushed until then, Step 8's stack rebase stays local and each PR-open push is a first push, not a force-push.
  - **Pause after each PR:** finish a Segment, run Steps 7-9 scoped to just that Segment, open its PR, then stop for the user before the next Segment. No cross-Segment rebase happens (earlier PRs are already open), so a fix implicating an already-delivered Segment becomes a follow-up, not a rebased commit.
- **TDD approach** (always recommend **red/green/refactor**):
  - **Red/green/refactor** (default recommendation): each test scenario gets its own failing-test, minimal-implementation, and refactor dispatch, per Step 5's TDD flow below. Recommend whenever the plan's Work Units carry Gherkin scenarios (the common case).
  - **Tests alongside implementation:** one dispatch per logical file group writes code and tests together (still encoding the plan's scenarios), skipping the three-dispatch cycle. Recommend for a plan that is mostly deletions/mechanical edits with little new logic (TDD adds dispatch overhead without adding rigor there).

**Flag presets.** `--pr-strategy=<stacked|independent|single>`, `--boundary=<savepoint|pause>`, and `--no-tdd` each preset a choice and skip its question. Absent (and not `--auto`) means ask; this preserves "ask every time" as the default. **Single** topology opens its one PR at the end regardless of boundary (it has a single PR, so "pause after each" is moot).

**3. `--auto`:** do NOT ask. Self-select the recommended options (independent topology when the Segments are disjoint, stacked otherwise; savepoints; red/green/refactor unless `--no-tdd` presets tests-alongside) and record them in the run's assumptions, surfaced in the final report / PR follow-ups. Flag presets still win over the auto default. `--force` still overrides quality-gate FAILs.

Record the resolved Segments and the chosen strategy in the progress ledger (Step 5) so a resumed run continues with the same shape.

## Step 5: Execute (delegated subagents, reviewed)

**Delegation (MUST):** this command runs on Sonnet. The orchestrating session reads the plan and delegates each implementation chunk to a subagent via the Agent tool, then reviews the result. Delegation keeps each chunk in a fresh, isolated context (no bleed between cycles); the orchestrator spends its turn reviewing, not editing. Independent Work Units run in parallel by default, each isolated in its own git worktree, with the model tier set per role (see the scheduler below). The deep design reasoning already happened in `/playbook:scope` or `/playbook:adr`, so execution doesn't need Opus.

Every Agent prompt MUST include: a pointer to its Work Unit's brief file (Step 5's File-based handoff), which carries the plan content that dispatch needs, the specific cycle/step, the test-structure rules below, the design principles below, and grounding rules ("read files before modifying, match existing style, verify imports resolve, don't guess types, apply SOLID/DRY/KISS/YAGNI"). Point at the brief; don't paste the whole multi-WU plan into every prompt.

**Design principles (MUST).** Every change, and every Agent prompt, applies:
- **SOLID:** one responsibility per unit, small focused interfaces, depend on abstractions only at real seams (no abstraction without a second caller).
- **DRY:** factor out genuine duplication once it recurs (rule of three); don't couple unrelated code that only looks alike.
- **KISS:** the simplest design that passes the tests and reads clearly; fewer moving parts wins.
- **YAGNI:** build only what the plan requires now. No speculative hooks, flags, config, or generality.
- **Self-explanatory over commented.** Clear names, small functions, obvious control flow, so a reader rarely needs a comment to follow it. No comments unless WHY is non-obvious; never restate WHAT the code already shows.
- **Composable over inherited.** Prefer composition (small, combinable functions or objects) over deep inheritance, in both OOP and functional code. Inheritance only for a genuine is-a relationship, kept shallow.
- **Model multi-step processing explicitly.** A request handler, a hook, a CLI command, or any multi-step pipeline reads as a named sequence of steps, not implicit control flow scattered across helpers. Log at each step, so a failure's exact location is visible from the logs alone, without needing to reproduce it locally first.
- **Idempotent and retryable where the operation allows it.** Design service and API operations to be safely repeatable (idempotency keys, upserts, at-least-once-safe handlers). This is also a `playbook:grounding-review` Reliability check, so building it in avoids a review round-trip.
- **Error messages are descriptive and assertive.** State exactly what failed and why, in plain language: "order 4471 has no shipping address", not "an error occurred". Never put PII or PHI (names, emails, government IDs, health data) in an error message or a log line; reference a record by its non-sensitive identifier instead.

When SOLID's abstraction pulls against KISS/YAGNI, favour the simplest thing that meets the plan. These principles are also the lens for the refinement pass (Step 8).

**Execution unit (MUST): the plan's Work Units, grouped by Segment.** Execute **one Segment at a time** in dependency order (Step 4.5). Within a Segment, execute its Work Units with the wave scheduler below; each WU becomes one small savepoint commit. The scheduler, worktree isolation, TDD flow, and verify-by-diff are unchanged; they just run scoped to the current Segment's WUs. **A wave never mixes WUs from two Segments:** the outer Segment loop is strictly sequential relative to the inner wave loop, so the ready set is always drawn from the current Segment only.

**Per-Segment setup (MUST): branch per topology.** Before a Segment's first commit, put HEAD on the right branch (always branch-first; never commit to the default branch). `<seg-slug>` is the Segment's Title kebab-cased and truncated, the same way `<plan-slug>` derives from the topic (Step 7 of `/playbook:scope`). Capture the branch's starting ref as `<segment-base>` (used by the re-split guard):

- **Stacked:** `git switch -c <type>/<plan-slug>-s<N>-<seg-slug>` off the previous Segment's branch (Segment 1 off the default branch); `<segment-base>` is that starting ref. The base for Segment N's PR is Segment N-1's branch.
- **Independent:** each Segment branch off the default branch; `<segment-base>` is the default branch.
- **Single:** one shared branch for the whole plan (the pre-existing behaviour); `<segment-base>` is the branch tip captured at this Segment's first commit.

On a **ledger-driven resume** (Step 5 ledger, e.g. after `/clear`, a crash, or a fresh checkout), the previous Segment's branch may not exist locally: `git fetch origin <prev-Segment-branch>` (or confirm the ref exists) before branching off it. Record `Segment id -> branch -> <segment-base> -> WU commit range` in the ledger as you go.

**Re-split guard (MUST, hard limit = 1500 changed lines; Segments target under 500).** After a Segment's WUs are committed, measure its real diff against its base: `git diff --shortstat <segment-base>...HEAD`. If changed lines exceed 1500, split the Segment at WU boundaries, in git:

1. Pick the last WU that keeps the Segment at or under budget; call its commit `<split-sha>`.
2. `git branch <type>/<plan-slug>-s<N>b-<seg-slug> HEAD` to save the excess commits, then `git reset --hard <split-sha>` on the current Segment branch to drop them from it.
3. The new `s<N>b` Segment branches off the trimmed current Segment (its `<segment-base>` is `<split-sha>`; under **independent** it still branches off the default branch); its PR targets the current Segment's branch under stacked, the default branch under independent, or the current Segment's (shared) branch under single. The `b` suffix avoids colliding with a planned `s<N+1>`. **Under single topology this means the re-split adds one follow-up PR** stacked on the shared branch: single still ships one PR normally, but the 1500 hard limit is never breached, so an overflowing single plan yields the shared-branch PR plus one follow-up.
4. **Deliver `s<N>b` as the very next Segment**, before any pre-planned `s<N+1>`, then continue the outer loop. Note the re-split (new Segment id, split point) in the ledger and the final report.

The plan's Segments are the starting point; reality wins at the budget.

**Parallel-by-default scheduler (MUST).** Don't wait for the plan to pre-label groups. Build the WU dependency graph from the `Requires` column and run the acyclicity check (the procedure in Step 6) now, before executing. Then execute in waves until every WU is done:

1. **Ready set.** The WUs whose `Requires` are all complete.
2. **Form a wave.** Take the largest subset of the ready set that's safe to run together. Parallel is the default. Two ready WUs run sequentially ONLY when one of these forces it:
   - a real dependency edge between them (transitive `Requires`),
   - their `Files` lists intersect,
   - shared mutable state: both touch a denylisted shared surface (migration dirs; lockfiles `package-lock.json`, `yarn.lock`, `pnpm-lock.yaml`, `poetry.lock`, `Cargo.lock`, `go.sum`; generated, barrel, or index files; global registries; codegen outputs), or there's genuine doubt about shared state.

   A plan `Parallel group` annotation, when present, confirms safety but isn't required. A WU that clashes with the forming wave drops to a later wave.
3. **Draft the wave's briefs (haiku).** Before dispatching, issue ONE Agent call for the whole wave, `model: "haiku"`, mechanical extraction, no judgment calls: hand it the wave's WU rows from the plan (Files, Changes, Test scenarios, Done When) plus the memory slice you selected for each WU (File-based handoff, below), and have it write each WU's `.brief.md`. One call per wave, not one per WU, keeps this to a single round-trip regardless of wave size.
4. **Dispatch the wave concurrently.** Issue the Agent calls in a single message so they run at once, one worktree per WU (see Worktree isolation). A wave of one runs in the main tree with no worktree. Give each Agent a stable `name`; the moment it returns its result, call `TaskStop` on it. A spawned agent stays idle-alive for `SendMessage` follow-ups and this flow never reuses a finished one, so leaving it unstopped keeps it running in the background.
5. **Integrate, then recompute.** After the wave returns, integrate (below), append to the ledger, then recompute the ready set for the next wave.

Scope each WU's verify command to its own test files (the full suite runs in Step 7) so an in-progress sibling can't trip another's tests.

**Worktree isolation (parallel waves).** For each WU in a multi-WU wave:

- `git worktree add "$(playbook path worktrees)/<plan-slug>/<wu-id>" HEAD` off the current branch, one per WU. Record this as the WU's base SHA in the ledger (Progress ledger, below): it's the anchor `git log` resumes against.
- Dispatch the implementer to work in that worktree path (absolute paths in its brief). A Work Unit with several test scenarios means several dispatches into the same worktree (see "With TDD" below); each commits its own step there. It does NOT push.
- **Integrate:** squash the WU's step commits into one (Commit per Work Unit, below), then cherry-pick that commit onto the current Segment branch (the run's branch, never the default branch) in dependency order. Disjoint files make this conflict-free. If a cherry-pick conflicts, the safety test was violated: STOP, keep the worktrees, and report. Then `git worktree remove` each.
- Single-WU waves skip the worktree and run in the main tree, on the Segment branch directly; the WU's base SHA is that branch's tip when the WU starts. The commit and resume mechanics below are the same either way: only the tree differs.

**No lock-step across WUs in a wave.** WUs in a wave proceed independently. If WU-A's RED returns before WU-B's RED, dispatch WU-A's GREEN immediately; don't wait for sibling WUs to reach the same scenario or step.

**File-based handoff.** Keep the orchestrator's context clean over long runs:

- Each WU's brief (its `Files`, `Changes`, `Test scenarios`, `Done When`, the worktree path, and the scoped verify command) is written to `$(playbook path implement)/<plan-slug>/<wu-id>.brief.md` by the wave's haiku drafting call (the scheduler's step 3, above), which points the implementer subagent at that file instead of pasting the whole plan into every prompt. Before that call, select a memory slice per WU from what Step 3 already loaded: facts whose `anchors:` overlap the WU's `Files`, plus any fact whose `MEMORY.md` one-line hook mentions the WU's title keywords. Hand the drafting call each WU's slice to include as a "Relevant memory" section in its brief. A WU touching nothing anchored gets no section, not an empty placeholder.
- A WU dispatches once per step (RED, GREEN, REFACTOR per scenario, or once per file group under `--no-tdd`), so its report path is scoped per dispatch: `<wu-id>.<step-slug>.report.md`, e.g. `wu-3.s2-red.report.md`. A shared `<wu-id>.report.md` would let GREEN's report clobber RED's before it's ever read. Each dispatch writes its full report there and returns ONLY: a status (`DONE`, `DONE_WITH_CONCERNS`, `BLOCKED`, or `NEEDS_CONTEXT`), its commit SHA, and a one-line test result.

**Read the report file (MUST, per `playbook:delegating-subagents`).** The dispatch's report file is the deliverable; the returned status is a courtesy. The moment a dispatch finishes, goes idle, or is given up on, **read its report file before doing anything else with that WU**, including before deciding it produced nothing. Agent-tool spawns frequently complete their work and return no result at all, so a silent agent is not an empty one. If the file is missing, say so explicitly rather than inferring what it would have said.

This is not optional when the commit looks fine. The report is the only place a WU records what it deliberately did NOT do: divergences it preserved on purpose, edges it left untested, files it declined to touch as out of scope. On 2026-08-16 a WU report stated plainly that `Command::Init` was still wired to nothing, and that a shell harness would break once a registry file was deleted. Both were blocking, both were rediscovered by hand a day later, and both were sitting in a file the orchestrator never opened while every test passed.

**Verify-by-diff (MUST).** Never take the subagent's word. After a WU returns, confirm the work from git (`git show --stat <sha>`, review the diff against the brief) and the scoped verify. A `DONE` the diff doesn't support is a failure: re-dispatch or stop per Error handling. Git tells you whether the work landed; only the report tells you what the agent observed, so do both.

**Progress ledger.** Record progress in `$(playbook path implement)/<plan-slug>.progress.md`. This directory lives outside the repo checkout (`$HOME/.config/playbook/repos/<owner>/<repo>/<worktree-id>/implement/`), so there is nothing to gitignore. At the top, record the resolved Segments and the chosen delivery strategy (topology + boundary) from Step 4.5. Per Segment, record its id, branch, whether it was re-split, and its PR URL once opened. Per WU, record one of three statuses: `NOT_STARTED` (or the row absent), `IN_PROGRESS` (its base SHA, recorded the moment its first dispatch starts), or `DONE` (its commit range, after squash and integration). On a fresh run or after compaction, read the ledger first: skip WUs recorded `DONE` and Segments already delivered (a paused run resumes from the first Segment without a PR URL); a WU recorded `IN_PROGRESS` routes to the resume procedure below instead of a fresh dispatch. First use in a repo, create the dir:

```bash
mkdir -p "$(playbook path implement)" "$(playbook path worktrees)"
```

**Model tiering (MUST, never omit `model`).** Implementer Tasks spawn the `implementer` agent, which pins `model: sonnet` itself, so you don't set the model on those calls. Brief drafting (the scheduler's step 3, above) is genuine content generation delegated off the orchestrator's own turn, so it runs `model: "haiku"` explicitly on that one Agent call per wave. Ledger writes stay a direct orchestrator action: a few-line append per WU, small enough that a separate dispatch would cost more in round-trip latency than it saves in tokens. Verification and the adversarial review (Step 9) run the capable tier. An omitted `model` on a non-typed call silently inherits the priciest default, so always set it there.

**Test structure (from `playbook:engineering-standards`):** every test follows Arrange-Act-Assert with `// Arrange` / `// Act` / `// Assert` comments mapping to the scenario's Given/When/Then; one action per test; use parameterised tests (`test.each`, `pytest.mark.parametrize`, table-driven) when scenarios share AAA structure but differ in data.

**With TDD (default).** A Work Unit with N test scenarios means up to 3N dispatches: for each scenario, in dependency order:

1. **RED** - `implementer` subagent (`subagent_type: playbook:implementer`): "Write ONLY the failing tests encoding this Gherkin scenario, AAA-structured. Don't touch production code." Then run the verify command: tests MUST fail (if they pass, the test proves nothing - fix it). On success, commit: `git commit -m "wip(<wu-id>): red - <scenario-id>"`, signed and signed off, inside the WU's tree. Never pushed.
2. **GREEN** - `implementer` subagent: "Write ONLY the minimal implementation to pass." Run verify: tests MUST pass (on failure, spawn a follow-up `implementer` with the error output). On success, commit: `wip(<wu-id>): green - <scenario-id>`.
3. **REFACTOR** - `implementer` subagent: "Clean up without changing behaviour; tests stay green." Run verify. On success, commit: `wip(<wu-id>): refactor - <scenario-id>`.
4. **Orchestrator review:** read the modified files; confirm changes match the plan, doc comments explain WHY, no unplanned side effects.

Each `wip` commit is a checkpoint, not the delivered shape: see Commit per Work Unit, below, for the squash that turns them into the WU's one real commit.

**RED anti-patterns to avoid (MUST).** A test that passes for the wrong reason is worse than no test:

- **Testing through implementation details.** Mocking internals, calling a private method, or verifying via a side channel (e.g. querying a table directly) instead of the public interface the scenario describes. Test the behaviour, not how it's built.
- **A tautological test.** The expected value is computed the same way the implementation computes it, including a hand-derived snapshot, so the test can't disagree with a wrong implementation. Derive the expectation independently.
- **Writing every scenario's tests before any implementation.** The per-scenario RED/GREEN/REFACTOR loop above already prevents this structurally; don't defeat it by batching RED across scenarios.

**Without TDD (`--no-tdd`).** For each logical file group: one `implementer` subagent (`subagent_type: playbook:implementer`) implements code + tests together (tests still encode the Gherkin scenarios); run verify; commit `wip(<wu-id>): <group-name>` on success; the orchestrator reviews as above.

**Commit per Work Unit (MUST): small commits.** One coherent commit per WU, in the tree the WU used (worktree for a parallel wave, the Segment branch in the main tree for a single-WU wave). The `wip` commits above are checkpoints while the WU is in flight, not the delivered shape:

1. Once the WU's last dispatch (its last REFACTOR, or its last `--no-tdd` group) passes review, squash: `git reset --soft <wu-base-sha>` then one commit with the WU's real message, staging exactly its `Files`.
2. **Parallel wave (worktree):** during integration the orchestrator cherry-picks that one commit onto the current Segment branch in dependency order (never the default branch). The Segment's branch stays local until its PR opens (Step 9, `/playbook:create-pull-request` handles the push); under the pause boundary that push happens per Segment mid-run, under savepoint at the end. Confirm the WU's files landed (`git log --stat`), then remove its worktree.
3. **Single-WU wave (main tree):** the squash commit already sits on the Segment branch; run `/playbook:commit-and-push` to push it. Confirm the files are committed (they no longer appear in `git status --porcelain`).

If a commit, squash, or cherry-pick fails, retry once, then stop and report.

**Resuming a Work Unit whose dispatch died mid-flight (MUST).** Trigger: a dispatch returns nothing usable (idle, errored, no report file), or a ledger-driven resume lands on a WU recorded `IN_PROGRESS`.

1. Per "Read the report file" above, read the last dispatch's report first, before classifying anything.
2. If that dispatch is still alive but idle, `TaskStop` it before dispatching a replacement. Two write-capable agents in the same tree risks a corrupted tree, not a recovery.
3. Find the WU's tree (its worktree, if still present, else the Segment branch) and its base SHA from the ledger. A missing worktree only means recreate it before continuing; it does not mean there's nothing to resume; a single-WU wave never had one, and its `wip` commits live on the Segment branch itself.
4. `git log --oneline <wu-base-sha>..HEAD` in that tree lists the `wip` commits landed so far. The last one's step and scenario say what's next (none found: start the WU from scratch, first scenario, RED).
5. Re-run the scoped verify against the tree's current state before trusting that last commit (the same Verify-by-diff principle above, applied to the last checkpoint instead of the whole WU). If it doesn't hold, `git reset --hard` past it and redo that step.
6. Continue the WU's per-scenario loop from there, dispatching fresh implementers only for what remains.

This restores the work, not the agent: nothing revives a dead or unreachable dispatch (per `playbook:delegating-subagents`, `SendMessage`-based recovery is unreliable for this agent type too). The existing "3 fix retries then stop" rule (Error handling, below) is unchanged and still governs a scenario whose verify genuinely fails after a real, confirmed attempt; this procedure targets the separate case of a dispatch that died or went silent with real, uncommitted-but-checkpointed progress on disk.

This mechanism doesn't apply to Step 8's refinement Work Units: they're synthesized in-session with no plan entry and no brief file, small enough that a failed one is simply redone whole.

## Step 6: Autonomous Mode (`--auto`)

`--auto` executes the Segments in dependency order, committing each Work Unit as a savepoint, then opens the PR set (Step 9). It self-selects the delivery strategy (Step 4.5: independent topology when the Segments are disjoint, stacked otherwise; savepoints), records it as an assumption, and runs without pausing, so:

- **Branch first.** Each Segment is created on its own branch per Step 5's per-Segment setup (never commit to the default branch). Never `--no-verify`, never force-push.
- A FAIL in Step 4 blocks; `--force` overrides it (logged to the quality report).

**Cycle check (MUST, before executing, both modes).** Step 5's scheduler runs this before any wave; in `--auto` it's the same check. Verify the Work Unit dependency graph is acyclic:

1. Build the adjacency list from the plan's Work Units table (`WU-N -> [Requires]`).
2. Count incoming edges per WU; seed a queue with the zero-incoming WUs.
3. Process the queue: mark each WU resolved, decrement the count of every WU it enables, enqueue any that hit zero.
4. If any WU is unresolved, those form a cycle: name them, break the cycle (extract shared logic into a new WU, merge, or reverse an edge), and re-check.

Report the result: `Cycle check: PASS (N WUs resolve in topological order)` or how a detected cycle was fixed.

**Per Segment (in dependency order), then per Work Unit (or parallel batch) within it:** create the Segment's branch per Step 5's per-Segment setup, run its WUs as below, then apply the Step 5 re-split guard before moving to the next Segment.

1. Confirm all WUs in the "Requires" column are done.
2. Dispatch each wave with the Step 5 parallel-by-default scheduler (ready set, safety test, worktree isolation, integration). Don't gate on `Parallel group` annotations; parallelize whatever the safety test allows, sequential only when forced. Within a WU, WU-0 types first, then the RED/GREEN/REFACTOR flow per cycle (or a single subagent for `--no-tdd`), delegated to Sonnet.
3. **Post-WU review:** changes match each WU's spec and file plan; doc comments explain WHY; no files outside the file plan touched.
4. Commit and integrate per the Step 5 commit rules: each dispatch checkpoints with a `wip` commit, the orchestrator squashes the WU's checkpoints into one commit, cherry-picks it (worktree) or pushes it (main tree). A dispatch that dies mid-WU resumes per Step 5's resume procedure instead of restarting the WU, same in `--auto` as interactively.
5. Mark each WU's "Done When" checkboxes in the plan file. This is the plan's static acceptance criteria, separate from the `wip`-commit checkpoints above: one records what the WU must achieve, the other records how far a dispatch got.

**Error handling:** when a WU's verify fails or its output is wrong, apply the `playbook:systematic-debugging` skill before retrying: find the root cause first, don't stack blind fix attempts. If it still fails (verify fails, wrong output, or commit fails) after 3 fix retries, **stop**. Don't continue to dependent WUs. Report the failed WU, the root cause found so far, and the remaining WUs. This governs a scenario whose verify genuinely fails after a real attempt; a dispatch that died or went silent uses Step 5's resume procedure first, not this retry count.

## Step 7: Validate

Run the project's checks (from Step 3 detection), e.g. type-check, lint, and tests. In `--auto`, run the full suite (not just affected) and, on failure, apply the `playbook:systematic-debugging` skill to find the root cause, then spawn an `implementer` subagent (`subagent_type: playbook:implementer`) to fix the responsible WU on its Segment branch, then amend via `/playbook:commit-and-push -a` (max 3 attempts; if still failing, stop and do NOT open any PRs).

- Fix and re-validate until green.
- **Doc audit:** every new/modified function has a doc comment explaining WHY; add any that are missing.
- **Update status:** change the plan's `Status: Proposed` to `Status: Implemented`.
- **Memory capture (MUST):** if a project store is present at `~/.config/playbook/memory/<owner>/<repo>/`, write every durable fact this run produced: notable errors and their fixes, conventions or gotchas discovered, and decisions made under ambiguity, the same categories the system prompt's Memory section scopes to feedback and project facts. This is required, not conditional on the model remembering to do it: `/playbook:implement` is the one capture path in this design that a command executes and checks, rather than a hook that only prompts. No project store means skip silently. The graph rebuilds automatically on fact save via the PostToolUse hook.
- **Cost delta:** read the run's current sample from the same `telemetry.jsonl` and report `cost_usd` minus the Step 3 baseline as this run's cost; if Step 3 recorded no baseline, report the cost as unavailable rather than guess. This is a session-level delta between two telemetry samples, not per-agent token accounting: no hook payload exposes per-agent tokens, so don't present it as more precise than that.

In `--auto`, after validation passes, continue to the refinement pass (Step 8). The PR opens at the end of Step 9, after the refinement and adversarial review pass, not here.

## Step 8: Refinement Pass (one pass, autonomous)

Once the implementation is green, run ONE refinement pass over the code you just produced. It runs autonomously (no pause, in interactive and `--auto` alike). It executes once per delivery unit: once over the full diff under the **savepoint** boundary, or once per Segment (before that Segment's PR) under **pause**; re-validation never re-enters this step for the same unit. It refines existing code; it does NOT add new scope (YAGNI keeps `/playbook:implement` execute-only). Apply the same commit safety as Step 6: never commit to the default branch, never force-push.

**Multi-Segment routing (MUST).** How this pass runs depends on the Step 4.5 boundary, because you must never force-push (and never rewrite a branch whose PR is already open):

- **Savepoint boundary (default):** no Segment's PR is open yet and the branches are unpushed, so this is safe. Run a single refinement pass over the full implemented diff, commit each fix onto **the Segment branch that owns the touched file** (the earliest Segment that introduced it), then, under stacked topology, rebase the later Segment branches onto their updated bases in order (`git rebase --onto`), purely local. Re-run the Step 7 checks on the affected branches. Step 9 opening the PRs is then the first push of each branch, not a force-push. (Single and independent topologies need no rebase.)
- **Pause boundary:** earlier Segments' PRs are already open, so you cannot rebase them. Run the refinement scoped to the **current** Segment only, before its PR opens (this is why the pause flow runs Steps 7-9 per Segment). A fix that implicates an already-delivered Segment is recorded as a follow-up, not a rebased commit.

1. **Self quick-review (local).** Apply the `playbook:grounding-review` discipline to the branch diff: severity-classified findings, each with `file:line` evidence. Keep it local; don't post anything. Fix only the findings you hold with HIGH confidence (clear bug, dead code, obvious simplification). Leave low-confidence or speculative findings for the adversarial review (Step 9); don't guess.
2. **Simplify & refactor analysis.** Read the changed files through the Design principles (SOLID, DRY, KISS, YAGNI). List concrete, behaviour-preserving changes: collapse needless indirection, delete dead or speculative code, dedupe real repetition, flatten tangled control flow, tighten names. Skip anything that changes behaviour or adds abstraction with no second caller.
3. **Re-plan.** Fold the high-confidence fixes and accepted simplifications into a small set of refinement Work Units (same shape as a `/playbook:scope` plan: `Files`, `Requires`, `Done When`). Scope is limited to code already written. If a finding implies new feature work, record it as a follow-up; don't build it.
4. **Execute autonomously.** Run the refinement Work Units like `--auto`: TDD where it applies, behaviour-preserving refactors keep tests green, commit each WU with `/playbook:commit-and-push`. Then re-run the validation checks from Step 7 (type-check/lint/test only, not the status flip or the continue-to-Step-8 handoff); they MUST stay green.

Run this pass once. Don't loop: Step 9 is the backstop for whatever remains.

## Step 9: Adversarial Review (MUST)

This reviews the IMPLEMENTED work, not the plan: Step 4's adversarial review ran before execution against the plan; this one runs after, against the diff.

**Haiku triage, before the swarm.** Skip triage entirely when `--all-lenses` was passed: all 5 lenses (correctness, behaviour drift, principles, scope, tests) run `full-lens`, unchanged from today's fixed-5-lens-always-full swarm.

Otherwise, dispatch `review-triage` (`subagent_type: playbook:review-triage`) exactly once, before the swarm, scoped to Step 9's fixed 5 lenses (`correctness`, `behaviour-drift`, `principles`, `scope`, `tests`), against the implemented diff (the same full branch diff the swarm dispatch below uses), the plan, and the refinement notes. Capture the returned tier map.

Three fail-open rules apply: if the `review-triage` dispatch itself fails, times out, or returns nothing at all, every lens defaults to `full-lens`. If it returns a tier map missing one or more lenses, each missing lens individually defaults to `full-lens`, keeping the lenses present in the map at their returned tier. If a lens IS present in the map but its `tier` value is anything other than `skip`, `cheap-check`, or `full-lens` (a drifted or malformed classifier response), that lens defaults to `full-lens` too: it must never fall through the dispatch-by-tier branches below silently.

Report which lenses resolved to which tier as a one-line summary, e.g. "Triage: correctness=full-lens, tests=cheap-check, scope=skip", before the swarm dispatches.

Dispatch it as a swarm of lens-specialized reviewers in parallel (each reads the diff, none writes, so parallel is always safe): for each of the 5 lenses, read its triage tier from the tier map captured above before dispatching; a lens absent from the map defaults to `full-lens`, per the fail-open-per-lens rule above. A `full-lens` lens dispatches a `reviewer` agent (`subagent_type: playbook:reviewer`) exactly as this step already did before tiered dispatch existed, issued as one Agent call per lens in a single message, with its lens as the focus, the full branch diff, the plan, and the refinement notes; do not change this prompt shape for this tier. Each lens tries to break the work, not bless it:

- **Correctness:** bugs, off-by-one, unhandled errors, regressions the tests miss.
- **Behaviour drift:** did any simplification or refactor change observable behaviour?
- **Principles:** remaining SOLID/DRY/KISS/YAGNI violations, leftover speculative code, needless abstraction.
- **Scope:** anything built beyond the plan; anything the plan required but is missing.
- **Tests:** weak assertions, missing boundary or regression coverage, flakiness.

A `cheap-check` lens dispatches a `cheap-checker` agent (`subagent_type: playbook:cheap-checker`) instead of `reviewer`. Its prompt names the lens's narrow concern, taken from the tier map's `reason` field for that lens, the full branch diff, the plan, the refinement notes, and ONE `skills/grounding-review/references/<file>.md` path to read for criteria, per this mapping (Step 9's 5 lenses carry different names from `/playbook:deep-review`'s lenses, so they need their own mapping, written here rather than reused from that command):

| Lens | Reference file |
|---|---|
| correctness | correctness.md |
| behaviour-drift | (none, no matching category) |
| principles | (none, no matching category) |
| scope | scope-control.md |
| tests | (none, no matching category) |

This mapping was derived the same way `/playbook:deep-review`'s Step 3 mapping was: matching each lens's stated concern against the bullet content of `skills/grounding-review/SKILL.md`'s Evaluation Categories, now split into `skills/grounding-review/references/*.md`. `scope` maps to `scope-control.md` directly since the category names match exactly. `principles` (SOLID/DRY/KISS/YAGNI violations, leftover speculative code, needless abstraction) has no matching category: `maintainability.md` covers mixed concerns, magic numbers, and naming, but never speculative code or unnecessary abstraction, the YAGNI half of this lens's own stated focus, so `principles` falls back to the full `SKILL.md` like `tests` does rather than pointing at a reference file that only partially covers its concern.

`behaviour-drift` (did a refactor change observable behaviour) also has no matching category: none of the 7 reference files ask whether a simplification changed behaviour, that is a distinct concern from the bug-pattern checks `correctness.md` covers, so it falls back to the full `SKILL.md` too rather than reusing a file that only partially fits.

Path resolution and fallback follow the same single rule as `/playbook:deep-review`'s Step 3 mapping: resolve the actual value of `$CLAUDE_PLUGIN_ROOT` with a real bash step before building the string, the same way `commands/doctor.md:123` does, for example:

```bash
REF_FILE="${CLAUDE_PLUGIN_ROOT}/skills/grounding-review/references/<file>.md"
[[ -f "$REF_FILE" ]] || REF_FILE="${CLAUDE_PLUGIN_ROOT}/skills/grounding-review/SKILL.md"
```

If the lens has a mapped file, resolve it this way and confirm it exists; if a lens has no mapped file (`principles`, `behaviour-drift`, `tests`) or the resolved file doesn't exist, resolve the full `SKILL.md` path instead, one rule either way, not two. Hand `cheap-checker` the resolved ABSOLUTE path this bash step produced, never the unexpanded placeholder or a bare repo-relative string: it has no `Bash` to expand `$CLAUDE_PLUGIN_ROOT` itself. The narrow concern text, not the reference file, is what scopes the check, so falling back to the full `SKILL.md` for criteria still returns a finding scoped to just that lens's concern.

A `skip` lens dispatches nothing. Track it explicitly as skipped in the triage summary above, e.g. "Triage: correctness=full-lens, tests=cheap-check, scope=skip"; a skipped lens is never conflated with a dispatched lens that returned nothing below, since it was never dispatched at all and so has no return value to lose.

**Trust gate.** This tiered dispatch mechanism ships and functions as soon as this Work Unit lands: a `full-lens` tier still gets the exact reviewer it always did, a `cheap-check` tier gets a real narrow-scope pass from `cheap-checker`, and a `skip` tier is a real, tracked decision to run nothing. But a `skip` or `cheap-check` decision should not be treated as validated judgment yet: `shell/eval-review-triage.sh` (a later Work Unit) has not yet recorded a pass verdict against a real fixture set. Until it has, treat triage's tier choices as best-effort, not proven: a `skip` verdict is not yet evidence a lens truly had nothing to find, and a `cheap-check` narrow pass is not yet guaranteed to have caught everything the full lens would have.

Give each reviewer Task a stable `name` and call `TaskStop` on it the moment it returns its findings. Reviewer agents stay idle-alive after returning; this flow never reuses them, so stop each one immediately.

**The `reviewer` agent is structurally read-only, so a lens can only deliver by returning, and that channel is unreliable** (`playbook:delegating-subagents`). It holds Read, Grep, Glob and Skill; `playbook agents check` forbids `Write` and `Bash` for that tier by design, so there is no file to fall back on. **A lens that returned nothing did NOT run.** Never count it as a clean lens, and never let a swarm with missing lenses read as "no findings": that is how a review swarm silently becomes a no-op while looking thorough. Name the missing lenses in the final report. Start your own pass on the riskiest part of the diff while the swarm runs, so lost lenses cost latency rather than coverage.

Each lens gives severity-classified findings with `file:line` evidence and a fix per finding. **Consolidate:** merge the files, dedup overlapping findings, drop anything already addressed, and fact-check each surviving finding against the file at HEAD before acting (discard stale or hallucinated ones). Then apply the fixes you hold with HIGH confidence plus every blocking correctness/security finding, routing each per Step 8's multi-Segment routing (savepoint: fix the owning branch and locally rebase the stack before any push; pause: fix the current Segment only, earlier-Segment fixes become follow-ups), and re-run the Step 7 validation checks. Surface the rest as known follow-ups: don't silently drop them, and don't start a second refinement loop.

**Open the PR set (MUST).** Once the Step 7 checks are green after the adversarial fixes, deliver the Segments as pull requests per the Step 4.5 topology, one PR per Segment, via the `/playbook:create-pull-request` skill (never raw `gh pr create`; it applies pre-flight checks, the conventional-commit title, the team template, and the `playbook:writing-style` voice). Each PR body names the Segment's concern and lists that Segment's slice of the unresolved follow-ups under a "Follow-ups" heading.

- **Stacked:** for each Segment **in order**, `/playbook:create-pull-request --base <prev-Segment-branch>` on its branch (Segment 1 uses the default branch). Opening in order matters: `/playbook:create-pull-request` pushes the current branch, so each Segment's branch is on origin by the time the next Segment names it as `--base`. This yields the stacked chain (PR #1 targets the default branch, PR #2 targets Segment 1's branch, and so on). Record each PR URL in the ledger.
- **Independent:** `/playbook:create-pull-request` per Segment branch with the default branch as base.
- **Single:** in `--auto`, one `/playbook:create-pull-request` for the whole branch; interactively, leave PR creation to the user (the pre-existing behaviour). If a hard-limit re-split fired (Step 5), the plan now has a shared branch plus one `s<N>b` follow-up branch: open both (follow-up `--base` the shared branch), or in interactive mode point the user at both.

**Boundary behaviour.** With **savepoint** (the default, and `--auto`), open the whole PR set here at the end. With **pause after each PR**, Step 9 has already run per Segment (its scoped review before the PR), so this step opens that one Segment's PR and stops for the user before the next Segment.

**Finish.** Report the applied fixes, the opened PRs (with URLs, bases, and draft state), any re-splits, and the unresolved follow-ups, naming each of the 5 lenses' triage tier alongside its findings, the same `<lens>: <tier> (<count>)` / `<lens>: skip` shape WU-6 added to `/playbook:deep-review`'s Step 5 `### Reviewers` line: a `full-lens` or `cheap-check` lens shows `<lens>: <tier> (<count>)` (tier written as `full` or `cheap-check` for display, not the raw `full-lens`/`cheap-check` value), a `skip` lens shows `<lens>: skip` with no count, e.g. "correctness: full (1) · tests: cheap-check (0) · scope: skip". In interactive mode with the **single** topology chosen, leave PR creation to the user as before; every other topology opens the PRs as above. Starting the next feature: run `/clear` before the next `/playbook:brainstorm` or `/playbook:scope`, so this run's plan, dispatch history, and fixes don't carry into it.

## Decision Rules

| Scenario | Action |
| --- | --- |
| Reference isn't a ready plan | STOP; tell the user to run `/playbook:scope` or `/playbook:adr` first (Step 2) |
| Task is ambiguous | Ask the user before executing |
| Plan needs new dependencies | List them and ask for approval (vet maintenance/license/CVEs) |
| Touches auth/security code | Flag for extra review; be conservative |
| Requires a DB migration | Execute the migration as its own unit with a rollback note |
| Validation fails | Fix and re-validate before reporting done |
| A WU dispatch goes idle, errors, or returns no report | `TaskStop` it if still alive, then resume from its `wip` commits (Step 5), don't restart the whole WU |
| `--auto`: WU fails after 3 retries | Stop; report the failed WU and the remaining WUs |
| `--auto`: validation fails after 3 fixes (including Step 8/9 re-validations) | Stop; report; do NOT open any PRs |
| `--auto`: on the default branch | Create a feature branch before the first commit |
| `--auto`: commit/push fails | Stop; report (auth, hooks, etc.) |
| Plan has no Segments (old plan/issue/spec) | Derive Segments targeting under 500 lines (never over 1500) in Step 4.5 |
| Segment's real diff exceeds the 1500-line hard limit | Re-split at WU boundaries into a new trailing Segment/PR; note it (Step 5) |
| Interactive, not `--auto`, no strategy flags | Ask the three Step 4.5 questions (topology + boundary + TDD approach), recommend per scope |
| `--auto` or a strategy flag set | Skip that question; self-select recommended (independent when disjoint, else stacked, + savepoint) and record as assumption |
| Pause boundary chosen | Run Steps 7-9 scoped per Segment, open its PR, stop for the user before the next (Step 9) |
| Refinement/adversarial fix, savepoint boundary | Commit on the owning Segment branch, locally rebase later branches before any push (Step 8) |
| Refinement/adversarial fix implicating an already-open PR (pause) | Record as a follow-up; don't rebase an open PR (Step 8) |
| Refinement (Step 8) implies new feature scope | Record as a follow-up; don't build it (YAGNI) |
| Adversarial review (Step 9) finds a blocking issue | Fix it, re-validate, then finish; report non-blocking findings as follow-ups |
| Refinement or adversarial fix would change behaviour | Don't fold it into the refactor; treat it as a separate fix and re-validate |

## Teardown (MUST run, even on failure or abort)

`TaskStop` every subagent spawned in this flow that is still alive: implementer Tasks from each wave, quality-gate agents from Step 4, and adversarial reviewer Tasks from Step 9. Confirm via `TaskList` that no tasks from this run remain before finishing.
