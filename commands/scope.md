---
description: Interview-driven planning session with deep requirements gathering that produces a verified implementation plan, its Work Units grouped into suggested PR-sized Segments.
allowed-tools: Bash, Read, Grep, Glob, Write, Agent, Skill
argument-hint: "[topic | ./prompt.md | .claude/designs/*.md] [--auto] [--help]"
model: opus
effort: xhigh
---

# Scope: Interactive Design Interview

Interview the user relentlessly about every aspect of this plan until you reach a shared understanding. Walk down each branch of the design tree, resolving dependencies between decisions one by one. The output is a verified, self-contained implementation plan, ready to run with `/playbook:implement`.

Invoked as `/playbook:scope`. The remaining arguments are an optional topic seed or file path.

## Help

If the arguments contain `--help`, print this and stop:

```
/playbook:scope - Interview-driven planning that produces an implementation plan

USAGE:
  /playbook:scope [topic]              Start interactive planning
  /playbook:scope "user auth system"   Start with a topic seed
  /playbook:scope ./prompt.md          Load topic seed from a file
  /playbook:scope .claude/designs/x.md Plan from a /playbook:brainstorm design doc

ARGUMENTS:
  If the argument is a file path (starts with ./ or / and the file exists),
  read the file content and use it as the topic seed. Prepare detailed
  prompts in a file instead of typing them inline.

OPTIONS:
  --auto   Autonomous: skip the interview, self-answer every decision with the
           recommended answer (recorded as assumptions), run the quality gate,
           and save the plan without pauses. Stops after saving; run /playbook:implement.
  --help   Show this help

Asks one question at a time with a recommended answer. Explores the codebase
and the memory store (`~/.claude/memory/` for global facts, `~/.claude/memory/<owner>/<repo>/` for project facts, `<owner>/<repo>` derived from the git remote) when it exists to answer
questions itself before asking you. Memory is optional: when neither store is present, it relies on the codebase alone. Walks decision trees, runs a 3-phase
quality gate, and produces a verified, self-contained plan broken into small
Work Units (some flagged parallel-safe), grouped into ordered PR-sized Segments
(one concern each, one PR each) that you can run with /playbook:implement.
```

## Core Rules (MUST)

**Autonomous mode (`--auto`) replaces the interview.** When `--auto` is set, ask the user nothing: resolve every branch yourself by taking the answer you'd otherwise recommend, record it in an **Assumptions** list, and run straight through to saving the plan without the Step 3 and Step 6 confirmation pauses. Rules 4 and 6 still hold (walk the full decision tree; never write code), and the quality gate (Step 5) still runs. See **Autonomous Mode (`--auto`)** below. The rules below otherwise describe the default interactive mode.

1. **Ask ONE question at a time.** Not two, not a batch. One question, wait for the answer, then the next. The only exception: the very first message, where you present initial context and the first question.
2. **Provide your recommended answer with every question.** Format: "Question? **I'd recommend X** because Y." The user can accept, reject, or modify. This keeps the conversation moving instead of stalling on open-ended questions.
3. **If a question can be answered by exploring the codebase, explore instead of asking.** Read the files, grep for patterns, check the config and the project memory store if one is present. Only ask the user about decisions, preferences, and constraints that aren't in the code.
4. **Walk the decision tree.** Each answer may open new branches. Track which branches are resolved and which are still open. Don't jump to unrelated topics while a branch has unresolved dependencies.
5. **Do NOT produce the implementation plan until all branches are resolved.** The user invoked `/playbook:scope` because they want thorough design, not a quick answer.
6. **Do NOT write any code.** This is a planning session. The output is a plan file, not implementation.

## Argument Resolution

If the argument looks like a file path (starts with `./`, `../`, `/`, or `~`, or ends with `.md`, `.txt`, `.yaml`, `.yml`), check whether the file exists with the Read tool:

- **File exists:** read its content and use it as the topic seed.
- **File does not exist:** treat the argument as a plain-text topic seed.

This happens before Step 1. The loaded content replaces the raw argument as the topic seed.

**When a file path is provided, the file IS the context (MUST).** Do NOT explore the repo beyond what the file explicitly references. Skip the general repo exploration in Step 1 (no git log, no TODO scan, no manifest scan). If a memory store is present, read it (global `~/.claude/memory/` and/or project `~/.claude/memory/<owner>/<repo>/`); when neither exists, skip this and proceed with the file content alone. Beyond memory, read ONLY the file, then go straight to your first question based on its content. If the file references specific source files, modules, or APIs by name, you may read those, but do NOT go looking for things the file does not mention.

**Ignore `.gitignore`d files (MUST).** Don't read files matched by `.gitignore` (PDFs, build artefacts, binaries, vendor dirs, `.env`), even if the prompt file mentions them. Only read tracked source files.

## Autonomous Mode (`--auto`)

Enable when `--auto` appears in the arguments; strip it (like `--help`) before resolving the topic seed. `--auto` runs the entire scope without the interview, then stops after saving the plan. It never writes code (run `/playbook:implement` to build it). Concretely:

- **No questions.** For every decision Step 2 would ask, take the answer you would have recommended ("I'd recommend X because Y") and proceed. Still do the Step 1 research first: explore the codebase and, if a memory store is present, read it too, since a preference or convention there may override your default choice. When no memory store exists, skip that step silently.
- **Record assumptions.** Every self-made decision goes into an **Assumptions** list with its rationale, so the user can audit what was chosen for them. When you're genuinely split on a decision, record it as an `OPEN` assumption (with the leading option and why) rather than silently picking.
- **Skip the confirmation gates.** Do not pause at Step 3 ("Does this capture everything?") or Step 6 ("Does this plan look right?"). Fold the Design Summary and the Assumptions list into the saved plan instead.
- **Quality gate still runs (Step 5).** It needs no user input. If a phase still FAILs after its 3 iterations, STOP: do not save; report the failing checks and the assumptions made. No user is present to override a FAIL in `--auto`.
- **Save and report (Step 7).** On a passing gate, save the plan and quality report, then tell the user the paths, the assumptions made (flag any `OPEN` ones), and to run `/playbook:implement` when ready. If a project store is present at `~/.claude/memory/<owner>/<repo>/`, persist the accepted decisions there; otherwise skip.

## Design Doc Handoff (from `/playbook:brainstorm`)

`/playbook:brainstorm` produces a design doc under `.claude/designs/` and can chain straight into `/playbook:scope`. Before Step 1, check for one:

- **Explicit:** if the argument resolves (per Argument Resolution) to a file under `.claude/designs/`, that file IS the design doc.
- **Implicit:** if no topic or file argument was given and `.claude/designs/` has entries, take the newest and ask once: "Base this plan on the design doc `<path>` (from `<date>`)? **I'd recommend yes** because it captures the agreed direction." If the user declines, proceed with a normal topic seed.

When a design doc is in play, it's the authoritative context, not a raw seed:

- Treat its **Decision**, chosen approach, **success criteria**, and **non-goals** as already settled. Do NOT re-litigate them in the interview.
- The interview covers only what the doc leaves open: its **Risks and open questions**, plus the planning detail the doc didn't decide (exact files, Work Unit boundaries, test strategy).
- Still gather the codebase context you need and explore whatever the plan requires. Still run the Step 5 quality gate and the Step 6 approval.
- Reference the design doc path in the saved plan's Architecture section so the lineage is traceable.

With no design doc, `/playbook:scope` behaves exactly as before.

## How It Works

### Step 1: Initial Context Gathering

**Skip this step if a file path was provided** (see Argument Resolution), except check for memory stores as described in the first bullet below when they exist. Then go to Step 2 with the file content as your context.

With a plain-text topic seed, silently research before asking anything:

- If a memory store is present, read it: check for the global store at `~/.claude/memory/MEMORY.md` (cross-project preferences, corrections, conventions) and the project store at `~/.claude/memory/<owner>/<repo>/MEMORY.md` (`<owner>/<repo>` derived from `git remote get-url origin`). Load the relevant fact files from whichever stores exist. This is the durable knowledge: architecture, conventions, decisions, gotchas. Honor the typed edges; when a project fact contradicts a global one it wins for this repo, and surface any conflict bearing on the plan rather than silently choosing. When neither store exists, skip this bullet and proceed on the codebase alone.
- Check recent git log for context.
- If the topic seed mentions files, modules, or features, read them.
- Read the README and whatever build manifest exists (`package.json`, `pyproject.toml`, `go.mod`, `Cargo.toml`, etc.) for project context.
- Read `TODO.md` (or an equivalent) if it exists.

Then present what you found and ask the first question:

```
I looked at [what you explored] and here's what I understand so far:
- [key finding 1]
- [key finding 2]

First question: [question]? **I'd recommend [X]** because [reason from codebase].
```

**In `--auto`:** gather the same context but ask nothing: go straight to Step 2 and self-resolve the decision tree, recording each choice as an assumption.

### Step 2: Decision Tree Interview

If a `/playbook:brainstorm` design doc with a "Confidence + open items" trailer exists for this work, read its open items and aim the interview at them first; those are the premises brainstorm couldn't fully verify.

Ask questions one at a time. Each question should follow from the previous answer (don't jump topics), include your recommended answer with reasoning, and resolve a specific branch.

Types of questions, in roughly this order (adapt):

- **Goal clarification:** What does "done" look like? Who is this for?
- **Scope boundaries:** What's explicitly out of scope? What should this NOT do?
- **Existing patterns:** "I found [pattern] in the codebase. Follow it or break from it? **I'd recommend following it** because [reason]."
- **Architecture decisions:** "Two approaches: A does [X] but [downside]. B does [Y] but [cost]. **I'd recommend A** because [reason]."
- **Dependencies:** "This depends on [system]. How should it interact? **I'd recommend [approach]** because I see [evidence in code]."
- **Edge cases:** "What happens when [failure scenario]? **I'd recommend [handling]** because [reason]."
- **Migration/rollout:** "How do we get from current state to the new state? **I'd recommend [strategy]** because [reason]."

Between questions, explore the codebase if the answer reveals new areas. Report what you found before asking the next question.

**Knowledge capture (memory).** When exploration reveals a durable convention or gotcha about the codebase (true regardless of this plan), and a project store is present at `~/.claude/memory/<owner>/<repo>/`, persist it as a project memory fact right then: a kebab-case file in `~/.claude/memory/<owner>/<repo>/` with `name`/`description`/`type: project`/`links:`/`anchors:` (to the files), plus its `MEMORY.md` index line. When no project store is present, skip this step silently. Track each confirmed *plan decision* in the running plan draft; those are persisted at Step 7, not now.

### Step 3: Confirm Understanding

When all branches are resolved, summarise:

```
## Design Summary

**Goal:** [one sentence]

### Decisions Made
1. [Decision]: [what was chosen] (because [reason])
2. ...

### Out of Scope
- [what was explicitly excluded]

### Open Risks
- [anything flagged but accepted]
```

Ask: **"Does this capture everything? Anything to change?"** Do NOT proceed until confirmed. **In `--auto`, skip this gate:** fold the summary and the Assumptions list into the plan and proceed.

### Step 4: Generate Implementation Plan

Produce a self-contained plan that `/playbook:implement` can consume directly:

```
## Implementation Plan

**Status:** Proposed | **Date:** <YYYY-MM-DD>

### Architecture
[High-level design. Reference specific files and functions found during research.]

### Files to Create/Modify
| File | Action | Purpose |
|------|--------|---------|
| path/to/file.ts | modify | [what changes and why] |
| path/to/new.ts  | create | [what it does] |

### Segments (suggested PRs)
Ordered, PR-sized groups of Work Units. One concern each; each Segment becomes one pull request.
`/playbook:implement` honors these but may re-split a Segment whose real diff exceeds the 1500-line hard limit.
| Seg | Title | Work Units | Requires | Concern | Est. lines |
|-----|-------|-----------|----------|---------|-----------|
| S1 | [title] | WU-0, WU-1, WU-2 | none | [schema + types] | ~180 |
| S2 | [title] | WU-3 | S1 | [wire-up] | ~90 |

### Deliverables (Work Units)
Smallest independently-committable units, in dependency order. One WU = one small commit. Each WU
belongs to exactly one Segment.
| WU | Title | Files | Requires | Segment | Parallel group | Done When |
|----|-------|-------|----------|---------|----------------|-----------|
| WU-0 | [title] | path/to/types.ts | none | S1 | none | [observable acceptance] |
| WU-1 | [title] | path/to/a.ts, path/to/a.test.ts | WU-0 | S1 | P1 | ... |
| WU-2 | [title] | path/to/b.ts, path/to/b.test.ts | WU-0 | S1 | P1 | ... |
| WU-3 | [title] | path/to/index.ts | WU-1, WU-2 | S2 | none | ... |

### Parallel Groups
- **P1** (after WU-0): WU-1 and WU-2. Disjoint files, no shared state, safe to run concurrently by separate agents.
- **Sequential:** WU-0 first; WU-3 last (needs WU-1 and WU-2).

### Per-Work-Unit Detail
For each WU, in dependency order:

#### WU-N: [title]
- **Requires:** [WU-x, WU-y | nothing]
- **Files:** [exact paths, production + test]
- **Changes:** [concrete what-to-do]
- **Test scenarios:** [Gherkin Given/When/Then or TDD cycles this WU satisfies]
- **Done When:**
  - [ ] [observable acceptance criterion]

### Testing Strategy
- [What to test, which test files, what assertions]

## Confidence + open items

- Confidence: HIGH | MEDIUM | LOW, <one line on what makes it that>
- Open items (verify downstream):
  - <blind spot or LOW-confidence premise>, <who verifies: /playbook:scope interview, /playbook:implement watch>
```

The Testing Strategy MUST follow the `playbook:engineering-standards` skill (test types, isolation, TDD red/green/refactor, no coverage decrease).

**Work Unit sizing (MUST).** Each WU is one coherent commit: small enough to review on its own, following the `playbook:engineering-standards` size limits and incremental-delivery guidance. Prefer more, smaller WUs over a few large ones; `/playbook:implement` commits each separately. The `Files` column lists production and test files. The `Requires` column is the dependency edge `/playbook:implement` topologically orders and cycle-checks. The `Segment` column names the PR-sized group each WU belongs to (see below).

**Segment sizing (MUST).** A Segment groups Work Units into one PR-sized, reviewable increment. Each Segment becomes one pull request, so:
1. **One concern per Segment.** Data layer, service layer, wire-up, and docs are separate Segments, not one, per `playbook:engineering-standards` "one concern per PR".
2. **Budget (three tiers, from `playbook:engineering-standards`).** Target under 500 changed lines per Segment (soft). A Segment estimated over 1000 (enforced) needs explicit justification in the plan; prefer to split it. Never plan a Segment estimated over 1500 (hard). `/playbook:implement` re-splits at the 1500 hard limit if reality exceeds the estimate.
3. **Ordering respects WU dependencies.** A Segment's WUs may only `Require` WUs in the same or an earlier Segment; no forward cross-Segment dependency. Default to a **linear** Segment chain (`S1 -> S2 -> S3`), which `/playbook:implement` maps to stacked PRs.
4. **Coverage.** Every WU belongs to exactly one Segment; no WU is left out and none appears in two.
5. **Suggestions, not law.** These are `/playbook:implement`'s starting point; note in the plan that it may re-split a Segment whose real diff blows the budget. Mark two Segments as parallel-safe only when their file sets are disjoint and neither `Requires` the other.

**Parallel-safety (MUST mark explicitly).** Assign a shared `Parallel group` label only when every member of the group:
1. has no dependency on another member (none appears in another's `Requires`),
2. touches a disjoint set of files (no file in two members), and
3. shares no mutable runtime state and no ordering-sensitive step (e.g. sequential DB migrations).

If any condition fails, leave the WUs ungrouped (they run sequentially). When unsure, leave sequential: a wrong parallel flag makes `/playbook:implement` run concurrent agents over the same files and corrupt the working tree. `/playbook:implement` re-verifies the flags before dispatching, but the plan should not assert parallelism it can't justify.

### Step 5: Quality Gate (MUST)

After producing the plan, run the three-phase gate. Do NOT skip it. Do NOT ask the user to verify it. Do NOT proceed to Step 6 until it passes. The criteria are inline below. Run Phase 1 first (Phases 2 and 3 read its report), then dispatch Phase 2 and Phase 3 in parallel: issue both Agent calls in a single message so they run at once. The 1-before-(2,3) order is the only real dependency here; Phase 2 and Phase 3 are independent, so they never run one at a time. Consolidate all three at the Quality Gate Result.

**All three phase agents are structurally read-only, so the return value is their only channel and it is unreliable** (`playbook:delegating-subagents`; invoke it before dispatching). `fact-checker`, `critic` and `test-reviewer` hold Read, Grep, Glob and Skill only, and cannot write a report file: `shell/check-agents.sh` forbids `Write` and `Bash` for that tier by design. **Run Phase 1 inline**, since its checks are mechanical and finish in about two minutes with re-runnable output. A phase that returned nothing is **INCONCLUSIVE, never PASS**, and INCONCLUSIVE blocks exactly like FAIL: redo it inline or record the gap and name the phase that did not run. A gate that checked nothing must never read as a pass.

#### Phase 1: Fact-Check

Spawn a `fact-checker` agent (`subagent_type: fact-checker`) with the full plan and these criteria:

- Every referenced file path exists.
- Function/type signatures and imports in the plan match the real code.
- The plan is consistent with existing patterns and conventions.
- Downstream consumers of changed code are identified.
- The test infrastructure the plan assumes actually exists.
- The Work Unit dependency graph is acyclic, and each Parallel group's WUs have disjoint files with no dependency on each other (the parallel-safe flags are accurate).
- **Segments are well-formed:** every WU maps to exactly one Segment (full coverage, no WU in two); Segment order respects WU `Requires` (no forward cross-Segment dependency); each Segment's estimate is within budget (FAIL if a planned Segment exceeds the 1500 hard limit, WARN if it exceeds 1000 without justification or exceeds the 500 soft limit); any parallel-marked Segments have disjoint files and no mutual `Requires`.

Returns a structured PASS / FAIL / WARN report. Phase 1 folds a Verification Summary into the report, reusing the `playbook:grounding-review` table shape:

```markdown
## Verification Summary

| Referenced path | Confirmed? | Where used |
|---|---|---|
| <path> | Yes (Read) / No (not found) | WU-N |

Confidence: HIGH | MEDIUM | LOW
```

Spawn it with a stable `name`; the moment it returns, `TaskStop` it: a spawned agent stays idle-alive for `SendMessage` follow-ups and this flow never reuses a finished one, so leaving it unstopped keeps it running in the background. **After it returns**, if a project store is present at `~/.claude/memory/<owner>/<repo>/`, persist any durable gotcha it found as a memory fact; otherwise skip. **If any FAILs:** revise the plan and re-run Phase 1 (max 3 iterations). Don't proceed until it passes.

#### Phase 2: Adversarial Review

Spawn a `critic` agent (`subagent_type: critic`, focus `plan`) with the full plan and the Phase 1 report. It challenges the design:

- Is there a simpler alternative that meets the goal?
- Scope creep or over-engineering?
- Missing error paths or failure handling?
- Blast radius: what could this break?
- Contradictions with the fact-check report.

Returns a structured report. Spawn it with a stable `name`; the moment it returns, `TaskStop` it: a spawned agent stays idle-alive for `SendMessage` follow-ups and this flow never reuses a finished one, so leaving it unstopped keeps it running in the background. **After it returns**, record any rejected simpler alternative (with reasoning) in the plan's Risks section. **If any FAILs:** revise and re-run Phase 2 (max 3 iterations).

#### Phase 3: Test Review

Spawn a `test-reviewer` agent (`subagent_type: test-reviewer`) with the plan's Testing Strategy and the Phase 1 report (it runs in parallel with Phase 2, so it doesn't wait on the adversarial findings). It evaluates the proposed tests against `playbook:engineering-standards`: regression-pinning, flakiness, boundary coverage, test independence, mock quality, assertion strength.

Returns a structured report. Spawn it with a stable `name`; the moment it returns, `TaskStop` it: a spawned agent stays idle-alive for `SendMessage` follow-ups and this flow never reuses a finished one, so leaving it unstopped keeps it running in the background. **After it returns**, if a project store is present at `~/.claude/memory/<owner>/<repo>/`, persist any durable test-quality pattern as a memory fact; otherwise skip. **If any FAILs:** revise the test plan and re-run Phase 3 (max 3 iterations).

#### Quality Gate Result

Present all three reports:

```
## Quality Gate Result

**Fact-Check:**        PASS (N/N checks passed)
**Adversarial Review:** PASS (N/N challenges passed)
**Test Review:**       PASS (N/N checks passed)
[or: BLOCKED: N FAILs, M WARNs]
```

WARNs are shown for awareness but do not block.

- **INCONCLUSIVE**: a phase returns INCONCLUSIVE, not PASS, when it couldn't actually perform its check: the agent failed to run or returned nothing, the target files were unreadable, or its confidence is LOW and blind spots dominate so a PASS would be unsupported. INCONCLUSIVE blocks the save exactly like FAIL and re-runs on the same max-3 loop; it's labeled distinctly so the cause reads as "couldn't verify," not "found a problem." A gate that checked nothing MUST NOT read PASS.

### Step 6: User Approval

**In `--auto`, skip this step:** there is no approval pause. After the gate passes, go straight to Step 7; a gate FAIL stops the run (Step 5) instead of prompting for an override. The rest of this step is the default interactive flow.

After the gate passes, present the full plan and the gate reports. Ask:

**"Quality gate passed. Does this plan look right? Anything to change before I save it?"**

If there are WARNs, highlight them: **"Note: N warnings flagged (see report). None are blocking."**

Do NOT save until the user explicitly approves. If they request changes, revise the affected parts, re-run the quality gate (Step 5), and present again.

**Override:** if the gate has FAILs and the user says to proceed anyway, record the override in the quality report file: `Quality gate override: proceeding despite FAIL on <check> because <user's reason>`.

### Step 7: Save & Next Steps

Only after the user approves. First time in this repo, create the plans dir and ignore it (same pattern as memory):

```bash
ROOT=$(git rev-parse --show-toplevel)
mkdir -p "$ROOT/.claude/plans"
grep -qxF '.claude/plans/' "$ROOT/.gitignore" 2>/dev/null || printf '.claude/plans/\n' >> "$ROOT/.gitignore"
```

Then:

1. Save the plan to `.claude/plans/<topic-slug>.md` and the gate reports to `.claude/plans/<topic-slug>-quality.md`.
2. If a project store is present at `~/.claude/memory/<owner>/<repo>/`, persist the plan's accepted key decisions as project memory facts (`type: project`, `anchors:` to the files they touch), and update `~/.claude/memory/<owner>/<repo>/MEMORY.md`. The graph rebuilds automatically on fact save via the PostToolUse hook. Otherwise skip.
3. Tell the user:
   - "Saved to `.claude/plans/<topic-slug>.md`"
   - "Implement it when ready: `/playbook:implement .claude/plans/<topic-slug>.md`."
   - "Want to capture the key decisions in an ADR (`/playbook:adr`)?" (if architectural)
   - **In `--auto`:** also list the **Assumptions** made (especially any `OPEN` ones) so the user can audit the autonomous choices before running `/playbook:implement`.

### Step 8: Teardown (MUST run, even on failure or abort)

`TaskStop` every subagent spawned in this flow that is still alive (Phase 1, 2, and 3 agents). Confirm via `TaskList` that none from this run remain before finishing.

## Adapting to Complexity

- **Simple change (1-2 files):** 3-5 questions. Don't over-interview a trivial change.
- **Medium feature (3-10 files):** 8-15 questions. Focus on integration points and edge cases.
- **Large feature (new module, architecture):** 15-25 questions. Cover dependencies, rollout, migration.
- **Bug fix:** replace design questions with diagnosis: reproduction steps, error messages, root-cause hypotheses. Still provide recommended answers.
