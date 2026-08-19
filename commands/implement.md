---
description: Execute an approved plan or ADR blueprint (from /playbook:scope or /playbook:adr) on Sonnet, delegating edits to subagents and committing each Work Unit. Delivers the plan as PR-sized Segments (savepoint commits, one small pull request per Segment; stacked by default), asking the delivery strategy up front. Then runs one refinement pass (self quick-review + SOLID/DRY/KISS/YAGNI simplify, re-planned and executed autonomously) and an adversarial review. Execute-only; it does not design new scope.
allowed-tools: Bash, Read, Grep, Glob, Write, Edit, Agent, Skill
argument-hint: "[plan | adr-blueprint | #issue | KEY-123 | ./spec.md | text] [--auto] [--no-tdd] [--force] [--pr-strategy=<stacked|independent|single>] [--boundary=<savepoint|pause>] [--help]"
model: sonnet
effort: high
---

# Implement: Execute a Verified Plan

Execute an approved implementation plan or ADR blueprint. **This command is execute-only: it does NOT design or plan new scope.** Produce the plan with `/playbook:scope` (or `/playbook:adr` for an architectural decision) first, then implement it here. The one exception is the Step 8 refinement pass, which re-plans and applies behaviour-preserving cleanups to the code it just wrote (never new features).

**Incremental delivery.** `/playbook:implement` delivers the plan as PR-sized **Segments**, not one big change: it executes Segment by Segment, commits each Work Unit as a savepoint, and opens one small pull request per Segment (stacked by default). Before executing, it asks how to deliver (PR topology and Segment-boundary behaviour) and recommends an option based on the plan's scope; under `--auto` it self-selects the recommended options and records them as assumptions. This follows `playbook:engineering-standards`: PRs under 500 lines, one concern each, "ship a sequence of small PRs".

Invoked as `/playbook:implement`. The remaining arguments are the task reference and flags.

## Help

If the arguments contain `--help`, print this and stop:

```
/playbook:implement - Execute an approved plan or ADR blueprint

USAGE:
  /playbook:implement                  List saved plans and pick one to execute
  /playbook:implement <task-reference> [options]

TASK SOURCES:
  Plan file      /playbook:implement .claude/plans/user-avatar-upload.md
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
  --no-tdd   Write tests alongside implementation instead of red/green/refactor
  --force    In --auto mode, override quality-gate FAILs (logged)
  --pr-strategy=<stacked|independent|single>
             Preset the PR topology and skip that question (default: ask)
  --boundary=<savepoint|pause>
             Preset the Segment-boundary behaviour and skip that question
             (default: ask; recommended: savepoint)

DELIVERY: /playbook:implement splits the plan into PR-sized Segments and, before
executing, asks two things (unless preset by flag or running --auto):
  - PR topology: stacked (default) | independent | single
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

## Step 1: Resolve the Task Reference

**No task reference given (empty, or only flags)?** Run the Plan Picker:

```bash
ROOT=$(git rev-parse --show-toplevel 2>/dev/null || pwd)
found=0
for f in "$ROOT"/.claude/plans/*.md "$ROOT"/docs/adr/*-blueprint.md; do
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

- **Plan/blueprint file** (`.claude/plans/*.md` or `docs/adr/*-blueprint.md`, or any path ending `.md`): Read it. This is the plan.
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
- If a memory store is present, load it: check whether `~/.claude/memory/MEMORY.md` exists and, if so, read it (cross-project preferences, corrections, conventions); check whether `~/.claude/memory/<owner>/<repo>/MEMORY.md` exists (`<owner>/<repo>` derived from `git remote get-url origin`) and, if so, read it, loading the relevant fact files for conventions, gotchas, and prior decisions. Honor the typed edges: a project fact that contradicts a global one wins for this repo, and surface any conflict bearing on the work rather than silently choosing. If neither store is present, skip this step silently and proceed on the codebase and the plan alone.
- **Cost baseline:** find the most recently written `telemetry.jsonl` under `~/.claude/runtime/` (one per session, populated by `statusline.sh` on each render), read its last line, and record the `cost_usd` field as this run's starting cost. No file yet (statusline hasn't rendered this session) means no baseline: Step 7 then reports the cost as unavailable rather than a delta.
- Read every file the plan references before changing it (grounding).
- **Detect the stack** to know the verify commands: check `tsconfig.json` / `package.json` (TS/JS), `pyproject.toml` / `setup.py` (Python), `go.mod` (Go), `Cargo.toml` (Rust). Derive the type-check / lint / test commands from what you find.

**Knowledge capture:** when you discover a durable convention or gotcha, write it as a project memory fact only if a project store is present at `~/.claude/memory/<owner>/<repo>/`; otherwise skip silently.

## Step 4: Quality Gate (conditional)

If the plan came from `/playbook:scope` or `/playbook:adr` it already has a companion `*-quality.md` report; trust it and skip to Step 5. Otherwise (a file/issue/ticket spec), run the inlined 3-phase gate before executing:

1. **Fact-Check** (`fact-checker` agent): every referenced path exists, signatures/imports match, downstream consumers identified, test infra present.
2. **Adversarial Review** (`critic` agent, focus `pre-exec`, + the fact-check report): simpler alternatives, scope creep, missing error paths, blast radius.
3. **Test Review** (`test-reviewer` agent): regression-pinning, flakiness, independence, mock quality, assertion strength.

Max 3 iterations per phase; revise on FAIL. A FAIL blocks execution unless the user explicitly overrides (or `--auto --force`). If a project store is present at `~/.claude/memory/<owner>/<repo>/`, record gotchas and rejected alternatives as memory facts; otherwise skip silently.

## Step 4.5: Delivery Strategy Gate (MUST, before executing)

`/playbook:implement` delivers the plan as PR-sized **Segments** (one concern, one pull request each), not one monolithic change. Settle the delivery shape now, before any code is written.

**1. Resolve Segments.**

- If the plan has a **Segments** table (a `/playbook:scope` plan does), use it: each Segment is an ordered group of Work Units, and the WU table's `Segment` column assigns every WU.
- If it does NOT (an older plan, an issue, or a spec), **derive** Segments here: pack the Work Units into groups targeting under 500 changed lines (never over the 1500 hard limit), in dependency order, one concern per group where the WU titles make the boundary obvious. A tiny plan may be a single Segment.

Either way, confirm the Segment ordering respects the WU `Requires` graph (no forward cross-Segment dependency) before continuing; reorder if needed.

**2. Choose the delivery strategy.** In interactive mode you MUST ask the user before any code is written (this is the "always ask before implementing" gate). Do this by **calling the `AskUserQuestion` tool** with the two questions below in a single call, each option's recommended choice listed first and labelled, recommended per the plan's scope. Do NOT infer the answers, and do NOT start executing Step 5 until the user has answered. The two questions:

- **PR topology:**
  - **Stacked** (default recommendation): each Segment branches off the previous one; PR N targets Segment N-1's branch. Recommend when Segments form a dependency chain (the common case).
  - **Independent off the default branch:** recommend only when the Segments have disjoint files and no cross-Segment `Requires`.
  - **Single PR:** the current whole-plan behaviour. Recommend for a one-Segment or tiny plan (escape hatch).
- **Segment-boundary behaviour** (always recommend **Savepoints**):
  - **Savepoint commits, PRs at end** (default recommendation, and the norm under `--auto`): implement every Segment as savepoint commits on their (unpushed) branches, run Steps 7-9 once over the full diff, then open the PR set at the end. Because nothing is pushed until then, Step 8's stack rebase stays local and each PR-open push is a first push, not a force-push.
  - **Pause after each PR:** finish a Segment, run Steps 7-9 scoped to just that Segment, open its PR, then stop for the user before the next Segment. No cross-Segment rebase happens (earlier PRs are already open), so a fix implicating an already-delivered Segment becomes a follow-up, not a rebased commit.

**Flag presets.** `--pr-strategy=<stacked|independent|single>` and `--boundary=<savepoint|pause>` preset a choice and skip its question. Absent (and not `--auto`) means ask; this preserves "ask every time" as the default. **Single** topology opens its one PR at the end regardless of boundary (it has a single PR, so "pause after each" is moot).

**3. `--auto`:** do NOT ask. Self-select the recommended options (stacked topology, or independent when the Segments are disjoint; savepoints) and record them in the run's assumptions, surfaced in the final report / PR follow-ups. Flag presets still win over the auto default. `--force` still overrides quality-gate FAILs.

Record the resolved Segments and the chosen strategy in the progress ledger (Step 5) so a resumed run continues with the same shape.

## Step 5: Execute (delegated subagents, reviewed)

**Delegation (MUST):** this command runs on Sonnet. The orchestrating session reads the plan and delegates each implementation chunk to a subagent via the Agent tool, then reviews the result. Delegation keeps each chunk in a fresh, isolated context (no bleed between cycles); the orchestrator spends its turn reviewing, not editing. Independent Work Units run in parallel by default, each isolated in its own git worktree, with the model tier set per role (see the scheduler below). The deep design reasoning already happened in `/playbook:scope` or `/playbook:adr`, so execution doesn't need Opus.

Every Agent prompt MUST include: the full plan content, the specific cycle/step/Work Unit, its Gherkin scenarios, the test-structure rules below, the verify command, the design principles below, and grounding rules ("read files before modifying, match existing style, verify imports resolve, don't guess types, apply SOLID/DRY/KISS/YAGNI").

**Design principles (MUST).** Every change, and every Agent prompt, applies:
- **SOLID:** one responsibility per unit, small focused interfaces, depend on abstractions only at real seams (no abstraction without a second caller).
- **DRY:** factor out genuine duplication once it recurs (rule of three); don't couple unrelated code that only looks alike.
- **KISS:** the simplest design that passes the tests and reads clearly; fewer moving parts wins.
- **YAGNI:** build only what the plan requires now. No speculative hooks, flags, config, or generality.

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
3. **Dispatch the wave concurrently.** Issue the Agent calls in a single message so they run at once, one worktree per WU (see Worktree isolation). A wave of one runs in the main tree with no worktree. Give each Agent a stable `name`; the moment it returns its result, call `TaskStop` on it. A spawned agent stays idle-alive for `SendMessage` follow-ups and this flow never reuses a finished one, so leaving it unstopped keeps it running in the background.
4. **Integrate, then recompute.** After the wave returns, integrate (below), append to the ledger, then recompute the ready set for the next wave.

Scope each WU's verify command to its own test files (the full suite runs in Step 7) so an in-progress sibling can't trip another's tests.

**Worktree isolation (parallel waves).** For each WU in a multi-WU wave:

- `git worktree add "$ROOT/.claude/worktrees/<plan-slug>/<wu-id>" HEAD` off the current branch, one per WU.
- Dispatch the implementer to work in that worktree path (absolute paths in its brief). It implements, runs its scoped verify, and commits inside the worktree. It does NOT push.
- **Integrate:** cherry-pick each WU's commit onto the current Segment branch (the run's branch, never the default branch) in dependency order. Disjoint files make this conflict-free. If a cherry-pick conflicts, the safety test was violated: STOP, keep the worktrees, and report. Then `git worktree remove` each.
- Single-WU waves skip the worktree and run in the main tree.

**File-based handoff.** Keep the orchestrator's context clean over long runs:

- Write each WU's brief (its `Files`, `Changes`, `Test scenarios`, `Done When`, the worktree path, and the scoped verify command) to `.claude/implement/<plan-slug>/<wu-id>.brief.md` and point the subagent at that file, instead of pasting the whole plan into every prompt.
- The implementer writes its full report to `<wu-id>.report.md` and returns ONLY: a status (`DONE`, `DONE_WITH_CONCERNS`, `BLOCKED`, or `NEEDS_CONTEXT`), its commit SHAs, and a one-line test result.

**Read the report file (MUST, per `playbook:delegating-subagents`).** `<wu-id>.report.md` is the deliverable; the returned status is a courtesy. The moment a WU's agent finishes, goes idle, or is given up on, **Read `<wu-id>.report.md` before doing anything else with that WU**, including before deciding it produced nothing. Agent-tool spawns frequently complete their work and return no result at all, so a silent agent is not an empty one. If the file is missing, say so explicitly rather than inferring what it would have said.

This is not optional when the commit looks fine. The report is the only place a WU records what it deliberately did NOT do: divergences it preserved on purpose, edges it left untested, files it declined to touch as out of scope. On 2026-08-16 a WU report stated plainly that `Command::Init` was still wired to nothing, and that a shell harness would break once a registry file was deleted. Both were blocking, both were rediscovered by hand a day later, and both were sitting in a file the orchestrator never opened while every test passed.

**Verify-by-diff (MUST).** Never take the subagent's word. After a WU returns, confirm the work from git (`git show --stat <sha>`, review the diff against the brief) and the scoped verify. A `DONE` the diff doesn't support is a failure: re-dispatch or stop per Error handling. Git tells you whether the work landed; only the report tells you what the agent observed, so do both.

**Progress ledger.** Record progress in `.claude/implement/<plan-slug>.progress.md` (gitignored). At the top, record the resolved Segments and the chosen delivery strategy (topology + boundary) from Step 4.5. Per Segment, record: Segment id, branch, its WU rows (WU id, status, commit range), whether it was re-split, and its PR URL once opened. On a fresh run or after compaction, read the ledger first and skip WUs already recorded done and Segments already delivered (a paused run resumes from the first Segment without a PR URL). First use in a repo, create and ignore the dir:

```bash
ROOT=$(git rev-parse --show-toplevel)
mkdir -p "$ROOT/.claude/implement"
for d in .claude/implement/ .claude/worktrees/; do
  grep -qxF "$d" "$ROOT/.gitignore" 2>/dev/null || printf '%s\n' "$d" >> "$ROOT/.gitignore"
done
```

**Model tiering (MUST, never omit `model`).** Implementer Tasks spawn the `implementer` agent, which pins `model: sonnet` itself, so you don't set the model on those calls. Mechanical bits (brief extraction, ledger writes) run `model: "haiku"`. Verification and the adversarial review (Step 9) run the capable tier. An omitted `model` on a non-typed call silently inherits the priciest default, so always set it there.

**Test structure (from `playbook:engineering-standards`):** every test follows Arrange-Act-Assert with `// Arrange` / `// Act` / `// Assert` comments mapping to the scenario's Given/When/Then; one action per test; use parameterised tests (`test.each`, `pytest.mark.parametrize`, table-driven) when scenarios share AAA structure but differ in data.

**With TDD (default).** For each cycle within a Work Unit (in dependency order):

1. **RED** - `implementer` subagent (`subagent_type: implementer`): "Write ONLY the failing tests encoding this Gherkin scenario, AAA-structured. Don't touch production code." Then run the verify command: tests MUST fail (if they pass, the test proves nothing - fix it).
2. **GREEN** - `implementer` subagent: "Write ONLY the minimal implementation to pass." Run verify: tests MUST pass (on failure, spawn a follow-up `implementer` with the error output).
3. **REFACTOR** - `implementer` subagent: "Clean up without changing behaviour; tests stay green." Run verify.
4. **Orchestrator review:** read the modified files; confirm changes match the plan, doc comments explain WHY, no unplanned side effects.

**Without TDD (`--no-tdd`).** For each logical file group: one `implementer` subagent (`subagent_type: implementer`) implements code + tests together (tests still encode the Gherkin scenarios); run verify; the orchestrator reviews as above.

**Commit per Work Unit (MUST): small commits.** One coherent commit per WU.

- **Parallel wave (worktree):** the implementer commits its WU inside its worktree, staging exactly its `Files` (it does not push). During integration the orchestrator cherry-picks each commit onto the current Segment branch in dependency order (never the default branch). The Segment's branch stays local until its PR opens (Step 9, `/playbook:create-pull-request` handles the push); under the pause boundary that push happens per Segment mid-run, under savepoint at the end. Confirm each WU's files landed (`git log --stat`).
- **Single-WU wave (main tree):** after the orchestrator review passes, stage exactly that WU's files (`git add <the WU's Files list>`) and run `/playbook:commit-and-push`. Confirm the files are committed (they no longer appear in `git status --porcelain`).

If a commit or cherry-pick fails, retry once, then stop and report.

## Step 6: Autonomous Mode (`--auto`)

`--auto` executes the Segments in dependency order, committing each Work Unit as a savepoint, then opens the PR set (Step 9). It self-selects the delivery strategy (Step 4.5: stacked topology, or independent when the Segments are disjoint; savepoints), records it as an assumption, and runs without pausing, so:

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
4. Commit and integrate per the Step 5 commit rules: implementers commit inside their worktrees, the orchestrator cherry-picks in dependency order and pushes; single-WU waves commit in the main tree.
5. Mark each WU's "Done When" checkboxes in the plan file.

**Error handling:** when a WU's verify fails or its output is wrong, apply the `playbook:systematic-debugging` skill before retrying: find the root cause first, don't stack blind fix attempts. If it still fails (verify fails, wrong output, or commit fails) after 3 fix retries, **stop**. Don't continue to dependent WUs. Report the failed WU, the root cause found so far, and the remaining WUs.

## Step 7: Validate

Run the project's checks (from Step 3 detection), e.g. type-check, lint, and tests. In `--auto`, run the full suite (not just affected) and, on failure, apply the `playbook:systematic-debugging` skill to find the root cause, then spawn an `implementer` subagent (`subagent_type: implementer`) to fix the responsible WU on its Segment branch, then amend via `/playbook:commit-and-push -a` (max 3 attempts; if still failing, stop and do NOT open any PRs).

- Fix and re-validate until green.
- **Doc audit:** every new/modified function has a doc comment explaining WHY; add any that are missing.
- **Update status:** change the plan's `Status: Proposed` to `Status: Implemented`.
- **Memory capture (MUST):** if a project store is present at `~/.claude/memory/<owner>/<repo>/`, write every durable fact this run produced: notable errors and their fixes, conventions or gotchas discovered, and decisions made under ambiguity, the same categories the system prompt's Memory section scopes to feedback and project facts. This is required, not conditional on the model remembering to do it: `/playbook:implement` is the one capture path in this design that a command executes and checks, rather than a hook that only prompts. No project store means skip silently. The graph rebuilds automatically on fact save via the PostToolUse hook.
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

This reviews the IMPLEMENTED work, not the plan: Step 4's adversarial review ran before execution against the plan; this one runs after, against the diff. Dispatch it as a swarm of lens-specialized reviewers in parallel (each reads the diff, none writes, so parallel is always safe): issue one Agent call per lens in a single message, each a `reviewer` agent (`subagent_type: reviewer`), with its lens as the focus, the full branch diff, the plan, and the refinement notes. Each lens tries to break the work, not bless it:

- **Correctness:** bugs, off-by-one, unhandled errors, regressions the tests miss.
- **Behaviour drift:** did any simplification or refactor change observable behaviour?
- **Principles:** remaining SOLID/DRY/KISS/YAGNI violations, leftover speculative code, needless abstraction.
- **Scope:** anything built beyond the plan; anything the plan required but is missing.
- **Tests:** weak assertions, missing boundary or regression coverage, flakiness.

Give each reviewer Task a stable `name` and call `TaskStop` on it the moment it returns its findings. Reviewer agents stay idle-alive after returning; this flow never reuses them, so stop each one immediately.

**The `reviewer` agent is structurally read-only, so a lens can only deliver by returning, and that channel is unreliable** (`playbook:delegating-subagents`). It holds Read, Grep, Glob and Skill; `playbook agents check` forbids `Write` and `Bash` for that tier by design, so there is no file to fall back on. **A lens that returned nothing did NOT run.** Never count it as a clean lens, and never let a swarm with missing lenses read as "no findings": that is how a review swarm silently becomes a no-op while looking thorough. Name the missing lenses in the final report. Start your own pass on the riskiest part of the diff while the swarm runs, so lost lenses cost latency rather than coverage.

Each lens gives severity-classified findings with `file:line` evidence and a fix per finding. **Consolidate:** merge the files, dedup overlapping findings, drop anything already addressed, and fact-check each surviving finding against the file at HEAD before acting (discard stale or hallucinated ones). Then apply the fixes you hold with HIGH confidence plus every blocking correctness/security finding, routing each per Step 8's multi-Segment routing (savepoint: fix the owning branch and locally rebase the stack before any push; pause: fix the current Segment only, earlier-Segment fixes become follow-ups), and re-run the Step 7 validation checks. Surface the rest as known follow-ups: don't silently drop them, and don't start a second refinement loop.

**Open the PR set (MUST).** Once the Step 7 checks are green after the adversarial fixes, deliver the Segments as pull requests per the Step 4.5 topology, one PR per Segment, via the `/playbook:create-pull-request` skill (never raw `gh pr create`; it applies pre-flight checks, the conventional-commit title, the team template, and the `playbook:writing-style` voice). Each PR body names the Segment's concern and lists that Segment's slice of the unresolved follow-ups under a "Follow-ups" heading.

- **Stacked:** for each Segment **in order**, `/playbook:create-pull-request --base <prev-Segment-branch>` on its branch (Segment 1 uses the default branch). Opening in order matters: `/playbook:create-pull-request` pushes the current branch, so each Segment's branch is on origin by the time the next Segment names it as `--base`. This yields the stacked chain (PR #1 targets the default branch, PR #2 targets Segment 1's branch, and so on). Record each PR URL in the ledger.
- **Independent:** `/playbook:create-pull-request` per Segment branch with the default branch as base.
- **Single:** in `--auto`, one `/playbook:create-pull-request` for the whole branch; interactively, leave PR creation to the user (the pre-existing behaviour). If a hard-limit re-split fired (Step 5), the plan now has a shared branch plus one `s<N>b` follow-up branch: open both (follow-up `--base` the shared branch), or in interactive mode point the user at both.

**Boundary behaviour.** With **savepoint** (the default, and `--auto`), open the whole PR set here at the end. With **pause after each PR**, Step 9 has already run per Segment (its scoped review before the PR), so this step opens that one Segment's PR and stops for the user before the next Segment.

**Finish.** Report the applied fixes, the opened PRs (with URLs, bases, and draft state), any re-splits, and the unresolved follow-ups. In interactive mode with the **single** topology chosen, leave PR creation to the user as before; every other topology opens the PRs as above.

## Decision Rules

| Scenario | Action |
| --- | --- |
| Reference isn't a ready plan | STOP; tell the user to run `/playbook:scope` or `/playbook:adr` first (Step 2) |
| Task is ambiguous | Ask the user before executing |
| Plan needs new dependencies | List them and ask for approval (vet maintenance/license/CVEs) |
| Touches auth/security code | Flag for extra review; be conservative |
| Requires a DB migration | Execute the migration as its own unit with a rollback note |
| Validation fails | Fix and re-validate before reporting done |
| `--auto`: WU fails after 3 retries | Stop; report the failed WU and the remaining WUs |
| `--auto`: validation fails after 3 fixes (including Step 8/9 re-validations) | Stop; report; do NOT open any PRs |
| `--auto`: on the default branch | Create a feature branch before the first commit |
| `--auto`: commit/push fails | Stop; report (auth, hooks, etc.) |
| Plan has no Segments (old plan/issue/spec) | Derive Segments targeting under 500 lines (never over 1500) in Step 4.5 |
| Segment's real diff exceeds the 1500-line hard limit | Re-split at WU boundaries into a new trailing Segment/PR; note it (Step 5) |
| Interactive, not `--auto`, no strategy flags | Ask the two Step 4.5 questions (topology + boundary), recommend per scope |
| `--auto` or a strategy flag set | Skip that question; self-select recommended (stacked + savepoint) and record as assumption |
| Pause boundary chosen | Run Steps 7-9 scoped per Segment, open its PR, stop for the user before the next (Step 9) |
| Refinement/adversarial fix, savepoint boundary | Commit on the owning Segment branch, locally rebase later branches before any push (Step 8) |
| Refinement/adversarial fix implicating an already-open PR (pause) | Record as a follow-up; don't rebase an open PR (Step 8) |
| Refinement (Step 8) implies new feature scope | Record as a follow-up; don't build it (YAGNI) |
| Adversarial review (Step 9) finds a blocking issue | Fix it, re-validate, then finish; report non-blocking findings as follow-ups |
| Refinement or adversarial fix would change behaviour | Don't fold it into the refactor; treat it as a separate fix and re-validate |

## Teardown (MUST run, even on failure or abort)

`TaskStop` every subagent spawned in this flow that is still alive: implementer Tasks from each wave, quality-gate agents from Step 4, and adversarial reviewer Tasks from Step 9. Confirm via `TaskList` that no tasks from this run remain before finishing.
