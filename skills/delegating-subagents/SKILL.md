---
name: delegating-subagents
description: Sets the file-based handoff, roster, routing, and recovery rules for subagents. Use before dispatching any subagent, and again when one finishes, fails, or goes quiet.
---

# Delegating to Subagents

A return value can be lost. When the agent can write a file, the file is the
delivery channel and the return value is a courtesy. When it cannot, the return
value is the only channel, and silence means the agent did not run.

## The rule

1. **Every brief for an agent that can write names an output file.** Absolute
   path, stated in the prompt.
2. **The orchestrator reads that file** the moment the agent finishes, goes
   idle, or is given up on. Unconditionally, before drawing any conclusion.
3. **A missing file is a distinct outcome** from an agent that found nothing.
   Say which one happened.

## Evidence

Return rates measured in August and re-measured on 2026-10-09: every Agent-tool spawn now returns its result in headless, foreground runs, but a long interactive run that goes idle can still end with a notification and no payload. The rules stay. Waiting longer, asking via `SendMessage`, telling the agent to deliver first, and inferring from git all failed to recover a lost result. Details: [references/evidence.md](references/evidence.md).

## First check whether the agent CAN write a file

File delivery needs `Write` or `Bash`. Several agents in this repo have neither,
on purpose: `playbook agents check` enforces
`FORBIDDEN_TOOLS_STRICT="Edit Write NotebookEdit Bash"` for structurally
read-only agents, so granting `Write` to a reviewer fails CI. That property is
deliberate (ADR 0003): a code reviewer must not be able to modify the code it
reviews.

| Agent | `subagent_type` to pass the `Agent` tool | Tools | Can deliver by file? |
|---|---|---|---|
| `implementer` | `playbook:implementer` | Read, Grep, Glob, Edit, Write, Bash, Skill | Yes, has `Write`/`Bash` |
| `patch-applier` | `playbook:patch-applier` | Read, Edit, Bash | Yes, has `Bash` |
| `collector` | `playbook:collector` | Bash, Read, Grep, Glob, WebFetch, Skill | Yes, has `Bash` |
| `auditor` | `playbook:auditor` | Bash, Read, Grep, Glob, WebSearch, WebFetch | Yes, has `Bash` |
| `git` | `playbook:git` | Bash, Read, Skill | Yes, has `Bash` |
| `reviewer` | `playbook:reviewer` | Read, Grep, Glob, Skill | **No. Read, Grep, Glob, Skill only** |
| `critic` | `playbook:critic` | Read, Grep, Glob, Skill | **No. Read, Grep, Glob, Skill only** |
| `fact-checker` | `playbook:fact-checker` | Read, Grep, Glob, Skill | **No. Read, Grep, Glob, Skill only** |
| `test-reviewer` | `playbook:test-reviewer` | Read, Grep, Glob, Skill | **No. Read, Grep, Glob, Skill only** |
| `analyst` | `playbook:analyst` | Read, Grep, Glob, Skill | **No. Read, Grep, Glob, Skill only** |
| `cheap-checker` | `playbook:cheap-checker` | Read, Grep, Glob, Skill | **No. Read, Grep, Glob, Skill only** |
| `review-triage` | `playbook:review-triage` | Read, Grep, Glob, Skill | **No. Read, Grep, Glob, Skill only** |

**Always pass the `playbook:` prefix as the `subagent_type` value**, not the
bare name in the first column: these are plugin-provided agents, and a bare
`subagent_type: critic` resolves to the wrong (or no) agent, the exact class
of bug fixed repo-wide in #284. This table's own prose elsewhere ("a
`critic` pass", "the `implementer` agent") uses the bare name only as a
short-hand label when talking ABOUT the agent, never as literal invocation
syntax; the second column above is the one to copy into an actual `Agent`
tool call.

`tests/delegating_subagents_roster.rs` keeps this table in sync with `agents/*.md` and requires the `playbook:` prefix on every row.

Effort-tier variants such as `reviewer-low` and `reviewer-xhigh` are not files
and are not in this table. `ccc` and `ccd` render them from the base agent for
one session into a throwaway plugin passed to Claude Code, so they carry the
base agent's `tools`, `model` and body and differ only in `effort`. Their
`subagent_type` is `playbook-variants:<name>`: spawn the exact type
`playbook effort resolve` gives you. A session started without the launcher has the base agents only.
The tiers and the rules are in `src/agents/variants.rs`.

**For the read-only agents, variants included, the return value is the only delivery channel.** It
worked in every run of the 2026-10-09 measurement. So:

- **Delegate them.** Reviews, critiques and fact-checks run in their own
  context, and the result comes back with the agent. For purely mechanical
  checks (path existence, line counts, graph acyclicity) a few shell commands
  you run yourself are still cheaper and re-runnable.
- **If a return does not arrive, treat silence as NOT RUN.** Never as "reviewed
  clean", never as PASS. Say which lens or phase is missing, in the report and
  to the user.
- **Do not grant them `Write` to work around this.** It breaks a CI-enforced
  safety property to fix a delivery problem. Change the plan, not the guard.

## Applying it

**In the brief, always (for agents that CAN write):**

```
Write your full report to <absolute path>.
Return only: status, plus any commit SHAs, plus one line of results.
The report file is the deliverable. The return value is not.
```

**In the orchestrator, always:**

Read the file. If it is missing, state that plainly and do not substitute a
guess about what the agent would have said.

**Choosing a mechanism.** When you genuinely need a result back inline and
cannot poll a file, prefer a forked skill (`context: fork` with an `agent:` in
the command's frontmatter) over an Agent-tool spawn. That path has been
reliable in every measurement so far.

**Reading the report is not optional even when the work looks obviously fine.**
The reports that mattered most were written by agents whose commits were green,
whose tests passed, and whose diffs looked correct. What they recorded was what
they had chosen NOT to do, and why. That is exactly the information a passing
test suite cannot give you.

## Route the task first

Before you pick an agent, run `playbook route <kind> --json`. The kinds are
`mechanical`, `classify`, `check`, `review`, `implement` and `design`. For
`implement`, add `--tier low|medium|high` (low for any well specified unit,
medium for a cross-module change or an ambiguous spec, high for workflow
determinism, concurrency or subtle architecture). Add `--failures <n>` when the
task already failed. Read `model`, `effort`, `agent` and `action` from the
answer.

- `action: proceed`: dispatch the named agent.
- `action: ask`: stop and ask the user about this task first. It runs on the
  top model, uses the high tier, or has already failed twice. Do not dispatch
  until the user says yes.
- `action: downgraded`: the user set `routing.escalate` to `deny`. Dispatch the
  cheaper route the answer gives.

The `routing.escalate` key is the gate. `ask` (the default) stops and asks the
user before an escalation. `auto` proceeds without asking and logs the
assumption in the run. `deny` never escalates and takes the cheaper route.

Say the tier and a one line reason in every dispatch report so the user can
override it. Pick the tier by judgment on each dispatch. A failure does not
raise the tier by itself. The user's effort ceiling always wins, and the answer
already applies it (`cappedFrom` says when it did).

## Pick the tier

The `Agent` tool has no per-call effort setting, so effort is chosen by which
agent you name. Two rules decide it, in this order.

**1. The ceiling.** Before a spawn, run
`playbook effort resolve agents <name> --json` and read `subagentType`. It is
the agent to spawn: the base agent when nothing limits it, or a variant when
the user's ceilings (Claude Code's `maxEffortLevel`, playbook's, or the
agent's own `effort.agents.<name>`) are below the base effort. `allowed` lists
every name at or under the ceiling and `allowedTypes` the matching `subagent_type` values (a variant is an agent of a throwaway session plugin, so its type reads `playbook-variants:reviewer-low`), and `satisfied: false` means no variant is
low enough or the session has none, so the base agent runs above the ceiling:
say so in your report. Never spawn a type that is not in `allowedTypes`.

**2. The diff.** Within the ceiling, pick from the size and risk of the diff,
using the limits the repo already has rather than new ones:

| Diff | Spawn | Why |
|---|---|---|
| At most 300 changed lines (the `dedup` trigger in `/playbook:deep-review`), and no security-sensitive path | the `-low` variant, if `allowed` has it | Little to reason about, so extra effort is spend with no extra findings. |
| Anything between, or when unsure | the base agent | The default, tuned for ordinary diffs. |
| Over 60 KB (`MAX_DIFF_BYTES` in `src/pr/triage.rs`, the size past which `review-triage` never calls a PR quick), or the lens is `security`, or the diff touches auth, secrets, crypto, or untrusted input | the `-xhigh` variant, if `allowed` has it | A missed finding here is expensive and the diff is big enough to hide one. |

If the variant you want is not in `allowed`, spawn the base agent. That is the
normal case outside the launcher, and it is never an error.

Narrow `cheap-check` lenses go to `cheap-checker`, which is already pinned to
low effort. A verifier that misses a wrong claim defeats its purpose, so do
not pick a cheaper `fact-checker` variant on your own for diff size: use one
only when the ceiling requires it. Every tier keeps the base agent's `tools`,
so the read-only guarantees and the file-delivery limits in the table above
apply unchanged.

## Re-dispatching (a second pass is a new agent, not a continuation)

Every `Agent` tool call is a fresh spawn with zero memory of any prior round, even for the same `subagent_type`. When a quality-gate phase (`critic`, `test-reviewer`, `fact-checker`) FAILs, gets revised, and needs a second pass, send the COMPLETE current artifact again, not a changelist. A partial re-prompt makes the agent flag things as unaddressed that the artifact already covers. Treat a "still failing" or "new finding" from a partial re-prompt with suspicion and check the full artifact before believing it. Why: [references/evidence.md](references/evidence.md).

## Verifying delegated work

Reading the report replaces neither check below; it tells you where to aim them.

- Confirm the work from git, not from the report's claims: `git show --stat`,
  then read the diff against the brief.
- Re-run the scoped verification yourself. A status of DONE that the diff does
  not support is a failure, not a rounding error.
- When the report names a deliberate divergence or an untested edge, decide
  explicitly whether to accept it, and record the decision. Do not let it pass
  silently just because the suite is green.
