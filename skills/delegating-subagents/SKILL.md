---
name: delegating-subagents
description: Handoff, roster, routing, and recovery rules for subagents. Use before dispatching any subagent, and when one finishes or goes quiet.
---

# Delegating to Subagents

A return value can be lost. When the agent can write a file, the file is the delivery channel and the return value is a courtesy. When it cannot, the return value is the only channel, and silence means the agent did not run. Measurements behind this are in [references/evidence.md](references/evidence.md); read it before loosening any rule here.

## The rule

1. **Every brief for an agent that can write names an output file**, as an absolute path in the prompt.
2. **The orchestrator reads that file** the moment the agent finishes, goes idle, or is given up on, before drawing any conclusion.
3. **A missing file is a distinct outcome** from an agent that found nothing. Say which happened, and never substitute a guess about what the agent would have said.

Brief template for agents that can write:

```
Write your full report to <absolute path>.
Return only: status, plus any commit SHAs, plus one line of results (at most about 300 words in all).
The report file is the deliverable. The return value is not.
```

Read the report even when the work looks fine. The reports that mattered most came from agents with green commits, and recorded what they chose NOT to do and why.

## Roster

File delivery needs `Write` or `Bash`. The read-only agents have neither on purpose (`playbook agents check` enforces it, ADR 0003): a reviewer must not be able to modify the code it reviews. Do not grant them `Write` to fix a delivery problem; change the plan, not the guard.

| Agent | `subagent_type` to pass the `Agent` tool | Delivery |
|---|---|---|
| `implementer` | `playbook:implementer` | File (has `Write` and `Bash`) |
| `collector` | `playbook:collector` | File (has `Bash`) |
| `auditor` | `playbook:auditor` | File (has `Bash`) |
| `git` | `playbook:git` | File (has `Bash`) |
| `reviewer` | `playbook:reviewer` | **Return value only** (Read, Grep, Glob, Skill) |
| `critic` | `playbook:critic` | **Return value only** (Read, Grep, Glob, Skill) |
| `fact-checker` | `playbook:fact-checker` | **Return value only** (Read, Grep, Glob, Skill) |
| `test-reviewer` | `playbook:test-reviewer` | **Return value only** (Read, Grep, Glob, Skill) |
| `analyst` | `playbook:analyst` | **Return value only** (Read, Grep, Glob, Skill) |
| `cheap-checker` | `playbook:cheap-checker` | **Return value only** (Read, Grep, Glob, Skill) |
| `review-triage` | `playbook:review-triage` | **Return value only** (Read, Grep, Glob, Skill) |

**Always pass the `playbook:` prefix** from the second column. These are plugin agents, and a bare `subagent_type: critic` resolves to the wrong agent or none (fixed repo-wide in #284). Elsewhere, the bare name is only a label for talking about the agent. `tests/delegating_subagents_roster.rs` keeps this table in sync with `agents/*.md`.

Effort variants such as `reviewer-low` and `reviewer-xhigh` are not files and not in the table. `ccc` and `ccd` render them for one session into a throwaway plugin, with the base agent's `tools`, `model` and body and a different `effort`. Their `subagent_type` is `playbook-variants:<name>`: spawn the exact type `playbook effort resolve` returns. A session started without the launcher has the base agents only (rules in `src/agents/variants.rs`).

**For the read-only agents, variants included:**

- **Delegate them.** Reviews, critiques and fact-checks run in their own context. For purely mechanical checks (path existence, line counts, graph acyclicity) a few shell commands you run yourself are cheaper and re-runnable.
- **Treat silence as NOT RUN.** Never as "reviewed clean" or PASS. Name the missing lens or phase in the report and to the user.

When you need a result inline and cannot poll a file, prefer a forked skill (`context: fork` with a prefixed `agent: playbook:<name>` in the command's frontmatter, since a bare name falls back to a general purpose agent on the session model) over an Agent-tool spawn. It has been reliable in every measurement.

## Fresh, not forked

An `Agent` call with no `subagent_type` forks the whole parent conversation: the child inherits the parent's history (measured: about 372K tokens on the first turn, $2.90 per fork, 117 of them in 30 days). **Always pass a `subagent_type`.** Use a `playbook:` agent from the roster, or the built-in `Explore` for read-only search and `general-purpose` for anything else. A fresh agent starts with only its brief, so write the brief to stand alone:

```
Goal: <one sentence, the question or the change>
Inputs: <absolute paths with line ranges; pasted text only when short>
Output: write your full report to <absolute path>
Constraints: <scope, files not to touch, read-only or not>
Return: status, findings as file:line, output path; at most about 300 words
```

The `Return` line caps the message, not the work: the full report goes in the output file, uncapped. Every agent's prompt carries the same cap. To keep the parent context small, read a finished agent's output file instead of calling `TaskOutput`, and when you must call it, ask once for the compact result rather than polling.

Fork only when the child needs the decisions and corrections made earlier in this session and writing them down would cost more than the inheritance. That is rare: the measured forks were research, review, edits and commit text, all of which a brief covers. Classification of every playbook spawn site is in [references/briefs.md](references/briefs.md).

## Route the task first

Run `playbook route <kind> --json` before picking an agent. Kinds: `mechanical`, `classify`, `check`, `review`, `implement`, `design`. For `implement` add `--tier low|medium|high` (low for any well specified unit, medium for a cross-module change or ambiguous spec, high for workflow determinism, concurrency or subtle architecture). Add `--failures <n>` when the task already failed. Read `model`, `effort`, `agent` and `action`.

- `action: proceed`: dispatch the named agent.
- `action: ask`: stop and ask the user first. The task runs on the top model, uses the high tier, or has failed twice. Do not dispatch until the user says yes.
- `action: downgraded`: the user set `routing.escalate` to `deny`. Dispatch the cheaper route given.

The `routing.escalate` key is the gate: `ask` (default) asks before escalating, `auto` proceeds and logs the assumption in the run, `deny` never escalates. Say the tier and a one line reason in every dispatch report. A failure does not raise the tier by itself. The user's effort ceiling always wins (`cappedFrom` says when it applied).

## Pick the tier

The `Agent` tool has no per-call effort setting, so effort is chosen by which agent you name. Two rules, in order.

**1. The ceiling.** Before a spawn, run `playbook effort resolve agents <name> --json` and read `subagentType`: the base agent when nothing limits it, or a variant when the user's ceilings (Claude Code's `maxEffortLevel`, playbook's, or `effort.agents.<name>`) are below the base effort. `allowed` lists names at or under the ceiling and `allowedTypes` the matching `subagent_type` values (a variant's type reads `playbook-variants:reviewer-low`). `satisfied: false` means no variant is low enough or the session has none, so the base agent runs above the ceiling: say so in your report. Never spawn a type not in `allowedTypes`.

**2. The diff.** Within the ceiling, use the limits the repo already has:

| Diff | Spawn | Why |
|---|---|---|
| At most 300 changed lines (the `dedup` trigger in `/playbook:deep-review`), and no security-sensitive path | the `-low` variant, if `allowed` has it | Little to reason about, so extra effort is spend with no extra findings. |
| Anything between, or when unsure | the base agent | The default, tuned for ordinary diffs. |
| Over 60 KB (`MAX_DIFF_BYTES` in `src/pr/triage.rs`, the size past which `review-triage` never calls a PR quick), or the lens is `security`, or the diff touches auth, secrets, crypto, or untrusted input | the `-xhigh` variant, if `allowed` has it | A missed finding here is expensive and the diff is big enough to hide one. |

**3. Automatic choice (only when the user turned it on).** Run `playbook effort choose <name> --base <ref> --json` (the ref is the diff's base, or pass `--lines`, `--files` and `--path` when there is no git diff) when the key `effort.auto` is `suggest` or `on`. Do nothing extra while it is `off`, the default. Spawn the `dispatch` it returns: with `on` that is the chosen variant, inside the agent's quality floor and every ceiling, and with `suggest` it is still the default. When `note` is set, print it as one line to the user, since the choice deviates from the default. An error or an unexpected answer means the default: spawn what step 1 gave. The agent marked costly to miss (the reviewer, the auditor) is only raised, never lowered. Every choice is in `playbook effort log`. This replaces the table above for that spawn, and never overrides step 1.

If the wanted variant is not in `allowed`, spawn the base agent. That is normal outside the launcher, never an error. Narrow `cheap-check` lenses go to `cheap-checker`, already pinned low. A verifier that misses a wrong claim defeats its purpose, so pick a cheaper `fact-checker` variant only when the ceiling requires it. Every tier keeps the base agent's `tools`, so the read-only and file-delivery limits above apply unchanged.

## Re-dispatching

Every `Agent` call is a fresh spawn with no memory of any prior round, even for the same `subagent_type`. When a quality-gate phase (`critic`, `test-reviewer`, `fact-checker`) FAILs, is revised, and needs a second pass, send the COMPLETE current artifact, not a changelist. A partial re-prompt makes the agent flag things as unaddressed that the artifact already covers; treat a "still failing" or "new finding" from one with suspicion. Why: [references/evidence.md](references/evidence.md).

## Verifying delegated work

Reading the report does not replace these; it tells you where to aim them.

- Confirm the work from git, not the report's claims: `git show --stat`, then read the diff against the brief.
- Re-run the scoped verification yourself. A DONE the diff does not support is a failure.
- When the report names a deliberate divergence or untested edge, decide explicitly whether to accept it and record the decision.
