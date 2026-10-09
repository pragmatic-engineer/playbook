---
description: Use when recording a significant, hard-to-reverse architectural decision, or when the user is choosing between named options and the choice is expensive to undo. Creates a fact-checked ADR with an optional execution blueprint and saves it to docs/adr/.
allowed-tools: Bash, Read, Grep, Glob, Write, Edit, Agent, Skill
argument-hint: "<topic> [--record-only] [--list] [--auto] [--ask] [--help]"
model: opus
effort: high
---

# ADR: Decision Records with Execution Blueprints

Create a formal Architecture Decision Record (ADR) with an optional execution blueprint: an archivable decision record paired with an actionable implementation plan. The blueprint is its implementation plan.

Invoked as `/playbook:adr`. The remaining arguments are the topic and flags.

## Defaults

No external config; these are fixed:

- **Document:** Architecture Decision Record (ADR).
- **Directory:** `docs/adr/`, tracked in git, created on first use. A decision record is shared history, so it belongs in the repo, not in a git-ignored scratch dir.
- **Filename:** `NNNN-{kebab-title}.md` (zero-padded sequence + kebab title). The date lives in the record's own front matter, not the filename.
- **Companion files:** blueprint at `{base}-blueprint.md`, quality report at `{base}-quality.md`, same directory.

## Help

If the arguments contain `--help`, print this and stop:

```
/playbook:adr - Architecture Decision Records with execution blueprints

USAGE:
  /playbook:adr <topic> [options]

OPTIONS:
  --help         Show this help
  --record-only  Write the decision record without an execution blueprint
  --list         List existing ADRs in docs/adr/
  --auto         Run unattended (this command stops, it needs a person)
  --ask          Force the interactive mode

NOTE: every ADR should have a companion execution blueprint. Use --record-only
only when the blueprint comes in a follow-up session.

EXAMPLES:
  /playbook:adr "replace polling with WebSocket push"
  /playbook:adr "split monolith order service"
  /playbook:adr --record-only "deprecate v1 API"
  /playbook:adr --list

Records save to docs/adr/ (tracked) as NNNN-{kebab}.md.
```

## Step 0: Read the run mode

Do this first, after the `--help` check above. Read the mode from the CLI:

```bash
playbook mode status --json
```

If the arguments contain `--auto`, add `--flag auto`. If they contain `--ask`, add `--flag ask`. If both are present, stop with one line: "--auto and --ask conflict; pass one." The JSON has four keys: `mode` (`ask` or `auto`), `source` (where it came from), `hook_mode` and `warning`. If `warning` is not empty, print it once. If the command fails, do not stop and do not retry. Run in ask mode and print one line. When the error is `unrecognized subcommand 'mode'`, the installed binary is older than this plugin: print "playbook binary is older than the plugin, run `playbook update`."

If the mode is `auto`, stop here. Print one line and nothing else: "/playbook:adr needs a person to run it, because an ADR records a decision that a person has to make and own." Do not run any later step.

In `ask` mode, behave exactly as this file describes.

**Effort ceiling.** Also run `playbook effort resolve commands adr --json`. Its `ceiling` is the highest effort the user allows for this run (`null` means no limit). Hold your own work to it, and before you spawn an agent follow the `delegating-subagents` skill, which runs `playbook effort resolve agents <agent>` and uses the `subagentType` it returns. If the command fails, carry on with no ceiling.

## Execution Rules (MUST)

1. **Execute every bash block for real with the `Bash` tool (capital B, tool names are case-sensitive).** Don't simulate, summarise, or predict output; use the actual output to drive the next step.
2. **No caching.** Every invocation is a fresh run. Don't reuse results from prior conversations or training data.
3. **No skipping.** Execute steps in order. The only exception: steps guarded by a flag the user didn't set.
4. **No assumptions.** Don't guess file contents, command output, or environment state. Run the command and read the result.
5. **Follow the command's gates, not your own.** If a step says "ask the user", ask. If it doesn't, don't add a confirmation gate.
6. **Show real data.** Tables and reports are populated from actual command output, never placeholders.

## Flag Handling

- `--list`: list existing records in `docs/adr/` (filenames + titles). If none exist, say so. Then stop.
- `--record-only`: run Stages 1-2 but skip the execution blueprint.

If no flags match, run the full workflow with the remaining text as the topic.

## Load Writing Discipline (MUST, before any drafting)

Invoke the `playbook:writing-style` skill (voice, banned words, prose rules) and the `playbook:grounding-research` skill (evidence and citations). Every ADR title, context line, alternative, and rejection note MUST follow them. ADR-specific rules on top:

- **Data over opinion.** Support claims with file paths, metrics, query counts, or concrete scenarios.
- **Spartan and informative.** Every sentence adds information. Cut sentences that only add emphasis.

## Stage 1: Investigate (mandatory)

Build understanding before writing. Skipping this produces records that don't survive contact with the codebase.

1. **Read existing records** in `docs/adr/` (if it exists) for precedent and numbering.
2. **Explore the codebase** with Read/Glob/Grep: modules and services affected by the topic, database schemas and migration history (if relevant), test patterns, configuration and deployment.
3. **Read memory stores if present** (optional enhancement, not required): run `playbook memory context --repo <owner>/<repo>` (`<owner>/<repo>` derived from `git remote get-url origin`), then load the fact files it names on demand. Use what you find to inform Considered Alternatives (reference a named pattern where one applies) and to avoid re-proposing something already rejected. Surface any `contradicts` edge among the returned facts that bears on the decision, rather than silently choosing one side. If the command produces no output (empty store, or the `playbook` binary unavailable, indistinguishable from stdout alone), fall back to reading `~/.config/playbook/memory/memory.graph.json` directly with the Read tool and picking out nodes whose `scope` is `global`, or whose `project` matches this repo (or its owner, for `org` scope): a dependency-free shape, since this command is an LLM session and can parse JSON without shelling to `playbook`. Note in the digest which path actually produced the result (command output, direct graph read, or nothing found), so an operator can tell "nothing relevant" apart from "the command couldn't run." If nothing is found by either path, skip this step silently and proceed on the codebase alone.
4. **Summarise findings to the user:** what's relevant to the topic, which areas are affected, existing patterns/constraints, and applicable patterns from memory if a memory store was present (with brief rationale).

**Knowledge capture:** if a project store is present at `~/.config/playbook/memory/<owner>/<repo>/`, write any durable convention or gotcha revealed by exploration as a project memory fact now. If no project store is present, skip this step silently.

Wait for the user to acknowledge before Stage 2.

## Stage 2: Draft

### Determine the filename

```bash
ROOT=$(git rev-parse --show-toplevel)
DIR="$ROOT/docs/adr"
mkdir -p "$DIR"
N=$(ls "$DIR" 2>/dev/null | grep -oE '^[0-9]{4}' | sort -rn | head -1)
NEXT=$(printf '%04d' $(( 10#${N:-0} + 1 )))
echo "Next number: $NEXT   Date: $(date +%Y-%m-%d)"
```

Filename: `{NEXT}-{kebab-title}.md` in `$DIR`. Write the record directly there (never to a temp/local scratch path).

### Decision Record

Write `{DIR}/{filename}` with Status **Proposed**, using this structure:

```markdown
# ADR-{NNNN}: {Title}

- **Status:** Proposed
- **Date created:** {YYYY-MM-DD}
- **Date modified:** {YYYY-MM-DD}

## Context
{Evidence-grounded background: file paths, metrics, concrete observations from Stage 1.}

## Decision Drivers
- {driver referencing concrete evidence}

## Considered Alternatives
### {Alternative} (effort: S | M | L | XL)
- {how it works}
- Trade-offs: {pros and cons}

## Decision
{The chosen alternative and why. State explicitly why each other alternative was rejected.}

## Consequences
- {positive, negative, and follow-up consequences}

## Architecture Diagrams
{Mermaid: a current-state diagram and a proposed-state diagram, plus sequence/state/ER diagrams as warranted.}
```

Requirements:

- At least 2 alternatives beyond the status quo (3 total minimum), each with a genuine effort estimate (S/M/L/XL) and real trade-offs.
- Decision drivers reference concrete evidence from Stage 1.
- The Decision section gives the reasoning for rejecting each alternative.
- **Diagrams:** keep them readable, label nodes meaningfully, pick the right type (flowchart for components, sequence for interactions, state for lifecycles, ER for schemas). Always include current-state and proposed-state.

**Knowledge capture:** if a project store is present at `~/.config/playbook/memory/<owner>/<repo>/`, record the decision and each rejected alternative as memory facts (so future planning, including `/playbook:plan`, doesn't re-propose them). If no project store is present, skip this step silently.

Present the draft. Revise in place on feedback. Repeat until the user explicitly approves.

### Execution Blueprint (skip if `--record-only`)

After the record is approved, write `{DIR}/{base}-blueprint.md` (`{base}` = filename without extension):

```markdown
# {ADR-NNNN} Execution Blueprint

- **Parent ADR:** {path to the decision record}

## System Snapshot
{Real paths discovered in Stage 1: modules, entry points, schemas, tests.}

## Work Units
### WU-0: {name}
- Requires: {WU-x, WU-y | nothing}
- Goal: {independently verifiable outcome}
- Files: {real path | action | purpose} (production + test)
- Verification: {literal, runnable command(s)}
- Tests: {Gherkin scenarios or TDD cycles, per engineering-standards}
- Done When:
  - [ ] {observable acceptance criterion}

## Ordering
| WU | Requires | Parallel group |
|---|---|---|
| WU-0 | none | none |
| WU-1 | WU-0 | P1 |
| WU-2 | WU-0 | P1 |

## Parallel Groups
- P1 (after WU-0): WU-1 and WU-2. Disjoint files, no shared state, safe to run concurrently by separate agents.
- Sequential: WU-0 first.

## Dependency Graph
{Mermaid graph generated from the Ordering table.}

## Confidence + open items

- Confidence: HIGH | MEDIUM | LOW, <one line on what makes it that>
- Open items (verify downstream): each one MUST be stated precisely enough that whoever picks it up next knows exactly what to check or decide. If it can't be phrased that precisely yet, say so plainly instead of listing a vague placeholder that only looks actionable.
  - <blind spot or LOW-confidence premise>, <who verifies: /playbook:plan interview, /playbook:implement watch>
```

Requirements: each work unit independently verifiable and committable as one small unit; file plans reference real existing paths; verification commands are literal (no placeholders); the Ordering table shows each WU's `Requires` and `Parallel group`; the test plan follows `playbook:engineering-standards`. Mark a shared `Parallel group` only when its members have no dependency on each other, touch disjoint files, and share no mutable state or ordering-sensitive step; otherwise leave them sequential. `/playbook:implement` consumes this blueprint exactly like a `/playbook:plan` plan: one small commit per WU, parallel-safe WUs dispatched to concurrent agents.

Present the blueprint. Revise in place until the user explicitly approves.

## Stage 3: Quality Gate (MUST)

After the user approves all drafts, run the three-phase gate before finalising. Don't skip it; don't finalise until it passes or the user explicitly overrides. Criteria are inline. Run Phase 1 first (Phases 2 and 3 read its report), then dispatch Phase 2 and Phase 3 in parallel: issue both Agent calls in a single message so they run at once. Phase 2 and Phase 3 are independent, so they never run one at a time; the 1-before-(2,3) order is the only real dependency.

**All three phase agents are structurally read-only, so their only channel is the return value, and it is unreliable** (`playbook:delegating-subagents`; invoke it before dispatching). `fact-checker`, `critic` and `test-reviewer` hold Read, Grep, Glob and Skill only. They cannot write a report file: `playbook agents check` forbids `Write` and `Bash` for that tier by design, and granting them would fail CI.

**Run Phase 1 inline. It is faster and it actually finishes.** Its checks are mechanical (do these paths exist, do the claimed line numbers match, is the dependency graph acyclic, do the Ordering table and the mermaid graph agree, are the parallel groups file-disjoint). That is a handful of shell and python commands, about two minutes, with re-runnable output. Delegating it has produced nothing across repeated attempts on this repo, while the inline version caught three wrong line citations and two unhonoured memory facts.

**A phase that returned nothing is INCONCLUSIVE, never PASS.** On two separate gates for ADR 0007, every dispatched phase agent went idle without returning; recording those as passes would have published a gate that checked nothing. INCONCLUSIVE blocks finalisation exactly like FAIL. Either redo that phase inline or record the gap explicitly in the quality report, naming which phase did not run.

Spawn each delegated agent with a stable `name`. `TaskStop` it only once you have its verdict or have made one post-idle `SendMessage` attempt; stopping is destructive and unrecoverable for a read-only agent. A spawned agent stays idle-alive for follow-ups and this flow never reuses a finished one, so an unstopped agent lingers as a background process.

Throughout this stage, `<record-slug>` is `adr-{base}`, where `{base}` is the record's filename without extension (from Stage 2's Determine the filename step, e.g. `docs/adr/0016-foo-bar.md` gives `<record-slug>` of `adr-0016-foo-bar`). Every `gate record`/`gate check` call below uses `<record-slug>` directly, already carrying its `adr-` prefix; do not prefix it again. That prefix keeps this record's gate rows distinct from `/playbook:plan` and `/playbook:implement` runs sharing the same per-repo-checkout gate database, since `gate_phases`' primary key carries no command column of its own.

**Gate source has two tiers.** Before each phase's own dispatch (its initial dispatch and any retry of that specific phase), snapshot the record file's content followed by the blueprint file's content (record content alone for a `--record-only` ADR) to a phase-specific scratch file, e.g. `/tmp/<repo>/adr-<record-slug>-source.<phase-name>.md` (`<phase-name>` is `fact-check`, `adversarial`, or `test-review`). That phase's `gate record` call always points `--source` at its own snapshot, frozen at the moment that phase was dispatched, never at a sibling phase's file: Phase 2 and Phase 3 dispatch in parallel, so if Phase 2 FAILs and gets revised before Phase 3 returns, Phase 3 must still record its verdict against what it actually reviewed, not the just-revised content. A separate shared scratch file, e.g. `/tmp/<repo>/adr-<record-slug>-source.md`, stays refreshed on every revision exactly as before; this is `<gate-source-path>`, used only by this stage's final Gate Check, which needs the current, fully up-to-date record and blueprint rather than any one phase's frozen snapshot.

### Phase 1: Fact-Check

**Before Phase 1's first dispatch, and before each retry of Phase 1**, snapshot the record (and blueprint, if any) to `/tmp/<repo>/adr-<record-slug>-source.fact-check.md`, Phase 1's own snapshot, and refresh the shared `<gate-source-path>` alongside it.

Spawn a `fact-checker` agent with the record (and blueprint, if any). It verifies: file paths in the system snapshot and file plans exist; function/type signatures referenced are accurate; the plan is consistent with existing patterns; the work unit dependency graph is acyclic and each Parallel group's WUs have disjoint files with no dependency on each other; if a memory store was loaded in Stage 1, known gotchas related to the topic are accounted for. Returns a PASS/FAIL/WARN report. Phase 1 folds a Verification Summary into the report, reusing the `playbook:grounding-review` table shape:

```markdown
## Verification Summary

| Referenced path | Confirmed? | Where used |
|---|---|---|
| <path> | Yes (Read) / No (not found) | WU-N |

Confidence: HIGH | MEDIUM | LOW
```

**Record the gate:** write its full raw return text to a file, e.g. `/tmp/<repo>/adr-<record-slug>-fact-check.txt`, then run `playbook gate record <record-slug> adr fact-check <that file> --source /tmp/<repo>/adr-<record-slug>-source.fact-check.md`. Do this every time Phase 1 returns, including every retry iteration below, not just the final one: `gate record` upserts on `(plan_slug, phase)`, so a stale FAIL from an earlier iteration is overwritten once a later iteration passes, and only the last recording before this stage's Gate Check matters.

After it returns, if a project store is present at `~/.config/playbook/memory/<owner>/<repo>/`, persist any durable gotcha as a memory fact, locked append as in Stage 1; otherwise skip that step silently. **FAIL → revise and re-run (max 3).** A revised record or blueprint means re-writing this phase's own snapshot, and the shared `<gate-source-path>`, before this phase re-runs.

### Phase 2: Adversarial Review

**Before Phase 2's own dispatch, and before each retry of Phase 2**, snapshot the record (and blueprint, if any) to `/tmp/<repo>/adr-<record-slug>-source.adversarial.md`, Phase 2's own snapshot, and refresh the shared `<gate-source-path>` alongside it.

Spawn a `critic` agent with focus `decision`, given the record, blueprint, and the Phase 1 report. It challenges the decision: simpler alternatives, scope creep, over-engineering, missing error paths, blast radius, contradictions with the fact-check.

**Record the gate:** write its full raw return text to a file, e.g. `/tmp/<repo>/adr-<record-slug>-adversarial.txt`, then run `playbook gate record <record-slug> adr adversarial <that file> --source /tmp/<repo>/adr-<record-slug>-source.adversarial.md`. Do this every time Phase 2 returns, including every retry iteration below, not just the final one: `gate record` upserts on `(plan_slug, phase)`, so only the last recording before this stage's Gate Check matters.

After it returns, if a project store is present at `~/.config/playbook/memory/<owner>/<repo>/`, record any rejected simpler alternative (with reasoning) as a memory fact, locked append as in Stage 1; otherwise skip that step silently. **FAIL → revise and re-run (max 3).** A revised record or blueprint means re-writing this phase's own snapshot, and the shared `<gate-source-path>`, before this phase re-runs.

### Phase 3: Test Review

**Before Phase 3's own dispatch, and before each retry of Phase 3**, snapshot the blueprint's test plan (and record, if any) to `/tmp/<repo>/adr-<record-slug>-source.test-review.md`, Phase 3's own snapshot, and refresh the shared `<gate-source-path>` alongside it.

Spawn a `test-reviewer` agent with the blueprint's test plan and the Phase 1 report (it runs in parallel with Phase 2). For a `--record-only` ADR with no blueprint tests, this is typically `PASS: N/A (no test plan)`. For a blueprint with Gherkin scenarios or TDD cycles, evaluate them against `playbook:engineering-standards`: regression-pinning, flakiness, boundary coverage, test independence, mock quality, assertion strength.

**Record the gate:** write its full raw return text to a file, e.g. `/tmp/<repo>/adr-<record-slug>-test-review.txt`, then run `playbook gate record <record-slug> adr test-review <that file> --source /tmp/<repo>/adr-<record-slug>-source.test-review.md`. Do this every time Phase 3 returns, including every retry iteration below, not just the final one: `gate record` upserts on `(plan_slug, phase)`, so only the last recording before this stage's Gate Check matters. **FAIL → revise the test plan and re-run (max 3).** A revised record or blueprint means re-writing this phase's own snapshot, and the shared `<gate-source-path>`, before this phase re-runs.

### Structural Checks

- [ ] Every Considered Alternatives entry has effort and trade-off detail.
- [ ] The Decision section explains why each rejected alternative was rejected.
- [ ] All work units (if a blueprint exists) have file plans with real paths.
- [ ] All verification commands are literal (no `<placeholders>`).
- [ ] No unresolved questions remain (or each is explicitly deferred to a named work unit).

### Gate Result

**Gate check (MUST, before presenting the result):** run `playbook gate check <record-slug> adr fact-check adversarial test-review --source <gate-source-path>`, using the shared gate source file in its current, fully refreshed state (never a phase's per-phase snapshot). Everything above, the phase reports and the Structural Checks, is for the human reading it; this command's exit code is what actually decides whether Stage 3 may finalise. Exit 0: proceed to present the result below and continue to Stage 4. Non-zero exit: do NOT present a "gate passed" result and do NOT proceed to Stage 4; report the command's own output verbatim, since it already names exactly which phase is MISSING, FAIL, STALE, or INCONCLUSIVE, never re-narrate it in your own words. For a FAIL or INCONCLUSIVE phase, revise and re-run it on that phase's own retry loop above. For a STALE phase, re-run that same phase against the current source without revising anything: STALE only means its recorded verdict's hash no longer matches the current draft, not that the phase's own review was wrong, and revising content in response would itself cascade staleness onto sibling phases.

A non-zero exit blocks finalisation unless the user explicitly overrides: the override never changes or fakes `gate check`'s result, it is an explicit, recorded decision to proceed despite a real, honestly reported non-zero exit, not a claim that the gate actually passed. Present the result, now backed by the recorded verdicts above rather than only the in-session agent responses. FAILs block finalisation; WARNs are informational. If the user explicitly overrides a FAIL, record the override in the quality report file: `Quality gate override: proceeding despite FAIL on <check> because <reason>`.

```
## Quality Gate Result

**Fact-Check:**        PASS (N/N)
**Adversarial Review:** PASS (N/N)
**Test Review:**       PASS (N/N)  [or N/A]
[or: BLOCKED: N FAILs, M WARNs]
```

- **INCONCLUSIVE**: a phase returns INCONCLUSIVE, not PASS, when it couldn't actually perform its check: the agent failed to run or returned nothing, the target files were unreadable, or its confidence is LOW and blind spots dominate so a PASS would be unsupported. INCONCLUSIVE blocks finalise exactly like FAIL and re-runs on the same max-3 loop; it's labeled distinctly so the cause reads as "couldn't verify," not "found a problem." A gate that checked nothing MUST NOT read PASS.

## Stage 4: Finalise

After the gate passes (or is overridden):

1. **Update the record:** Status Proposed → Accepted, and bump Date modified to today.
2. **Verify the blueprint** (if any): its Parent ADR reference points to the correct record path.
3. **Save the quality report** to `{DIR}/{base}-quality.md`.
4. **Memory graph:** if a project store is present at `~/.config/playbook/memory/<owner>/<repo>/` and you wrote any memory facts for this decision, the graph rebuilds automatically on fact save via the PostToolUse hook. Otherwise skip silently.
5. **Report** the final file paths to the user.

### Kebab Title Convention

Lowercase all words, replace spaces and special characters with hyphens, collapse consecutive hyphens. "Replace Polling with WebSocket Push" → `replace-polling-with-websocket-push`.

## Implementing the Blueprint

The blueprint is a self-contained implementation plan. Run `/clear` first, then implement it with a clean context: `/playbook:implement {DIR}/{base}-blueprint.md`. The record-drafting and quality-gate back-and-forth in this session is not something the execution phase needs to inherit.

## Teardown (MUST run, even on failure or abort)

`TaskStop` every subagent spawned in this flow that is still alive, then confirm via `TaskList` that none from this run remain before finishing.
