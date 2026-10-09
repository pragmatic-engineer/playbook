---
name: delegating-subagents
description: Use before dispatching any subagent, and again the moment one finishes or goes quiet.
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

## What we measured

On 2026-08-16 and 17, 22 delegations in one session:

| Spawn mechanism | Returned a result inline |
|---|---|
| Skill tool with `context: fork` and an `agent:` in frontmatter | 11 of 11 |
| Agent tool, any `subagent_type`, plugin or built-in | 0 of 11 |

Re-measured on 2026-10-09 with Claude Code 2.1.293, using `claude -p` on Haiku
and one Agent tool call per run. Each read-only agent was asked to read
`Cargo.toml` and return a marker plus the package version:

- `reviewer`: 4 of 4 returned the marker.
- `critic`: 4 of 4.
- `fact-checker`: 3 of 3.
- `cheap-checker`: 3 of 3.

That is 14 of 14, at about $0.04 per run. Every Agent-tool spawn now returns its
result, so the August number no longer holds. The August failure had a real
cost: two blocking defects sat in a written report for a day, and a third
finding was never seen. A lost return is rare now, not impossible.

The limit of the new measurement: it is headless and foreground, with short
tasks. Long interactive runs that go idle can still end with a notification and
no payload, so the rules below stay.

## What does not work

- Waiting longer. The result is not in flight; there is nothing to wait for.
- `SendMessage` asking for the result. Sometimes recovers it, often does not.
  Three escalating rounds, including an explicit "call SendMessage with
  to: main", returned nothing from four agents.
- Telling the agent to deliver first, before finishing. Tried, no effect.
- Reading git to infer what happened. Commits tell you whether work LANDED.
  They never tell you what the agent OBSERVED, which is the part you delegated
  for. Divergences it chose to preserve, quirks it found, scope it deliberately
  left alone: all of that lives only in the report.

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
| `reviewer-low` | `playbook:reviewer-low` | Read, Grep, Glob, Skill | **No. Read, Grep, Glob, Skill only** |
| `reviewer-xhigh` | `playbook:reviewer-xhigh` | Read, Grep, Glob, Skill | **No. Read, Grep, Glob, Skill only** |
| `critic-low` | `playbook:critic-low` | Read, Grep, Glob, Skill | **No. Read, Grep, Glob, Skill only** |
| `critic-xhigh` | `playbook:critic-xhigh` | Read, Grep, Glob, Skill | **No. Read, Grep, Glob, Skill only** |
| `fact-checker-xhigh` | `playbook:fact-checker-xhigh` | Read, Grep, Glob, Skill | **No. Read, Grep, Glob, Skill only** |
| `analyst-low` | `playbook:analyst-low` | Read, Grep, Glob, Skill | **No. Read, Grep, Glob, Skill only** |
| `analyst-xhigh` | `playbook:analyst-xhigh` | Read, Grep, Glob, Skill | **No. Read, Grep, Glob, Skill only** |
| `implementer-low` | `playbook:implementer-low` | Read, Grep, Glob, Edit, Write, Bash, Skill | Yes, has `Write`/`Bash` |
| `implementer-xhigh` | `playbook:implementer-xhigh` | Read, Grep, Glob, Edit, Write, Bash, Skill | Yes, has `Write`/`Bash` |

**Always pass the `playbook:` prefix as the `subagent_type` value**, not the
bare name in the first column: these are plugin-provided agents, and a bare
`subagent_type: critic` resolves to the wrong (or no) agent, the exact class
of bug fixed repo-wide in #284. This table's own prose elsewhere ("a
`critic` pass", "the `implementer` agent") uses the bare name only as a
short-hand label when talking ABOUT the agent, never as literal invocation
syntax; the second column above is the one to copy into an actual `Agent`
tool call.

(Full current roster, `agents/*.md`, cross-checked against each file's own `tools:`
frontmatter, not assumed from memory: this table went stale once before, missing
half the roster after `auditor`, `cheap-checker`, `patch-applier`, and
`review-triage` were added. `tests/delegating_subagents_roster.rs` now enforces
this table stays in sync with `agents/*.md` and that every row's `subagent_type`
carries the `playbook:` prefix, so the next agent addition fails CI instead of
quietly drifting again.)

The `-low` and `-xhigh` rows are effort-tier variants, generated from the base
agent by `playbook agents gen` and identical to it except for `name`, the
`description` prefix, and `effort`. They need the same `playbook:` prefix, for
example `playbook:reviewer-xhigh`. Never edit one by hand: `playbook agents
check` fails CI when a variant is missing, stale, or edited. The set lives in
`src/agents/variants.rs`.

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

## Pick the tier

The `Agent` tool has no per-call effort setting, so effort is chosen by which
agent you name. Pick it from the size and risk of the diff, using the limits
the repo already has rather than new ones:

| Diff | Spawn | Why |
|---|---|---|
| At most 300 changed lines (the `dedup` trigger in `/playbook:deep-review`), and no security-sensitive path | the `-low` variant | Little to reason about, so extra effort is spend with no extra findings. |
| Anything between, or when unsure | the base agent | The default, tuned for ordinary diffs. |
| Over 60 KB (`MAX_DIFF_BYTES` in `src/pr/triage.rs`, the size past which `review-triage` never calls a PR quick), or the lens is `security`, or the diff touches auth, secrets, crypto, or untrusted input | the `-xhigh` variant | A missed finding here is expensive and the diff is big enough to hide one. |

Narrow `cheap-check` lenses go to `cheap-checker`, which is already pinned to
low effort, so they need no variant. `fact-checker` has an `-xhigh` variant
only: a verifier that misses a wrong claim defeats its purpose, so there is no
`-low` for it. Every tier keeps the base agent's `tools`, so the read-only
guarantees and the file-delivery limits in the table above apply unchanged.

## Re-dispatching (a second pass is a new agent, not a continuation)

Every `Agent` tool call is a fresh spawn with zero memory of any prior round,
even one run earlier in the same session and even for the exact same
`subagent_type`. When a quality-gate phase (`critic`, `test-reviewer`,
`fact-checker`) FAILs, gets revised, and needs a second pass, send the
COMPLETE current artifact again, not a "here's what changed since round 1"
diff or changelist.

**Why.** During one quality gate, round 1 of a `critic` pass found one real
blocking defect and it got fixed. Round 2 was dispatched with only a
"here's what changed" summary. It correctly re-verified the actual fix, then
flagged two unrelated things as "unaddressed" that were genuinely already
covered elsewhere in the plan, purely because the round-2 prompt never
restated them. The agent was not lying or hallucinating: it reviewed exactly
what it was shown, and what it was shown was incomplete. A third round with
the full artifact confirmed both flags were false and surfaced the one thing
that actually was new.

**How to apply.** Budget for this on every re-dispatch: resend the complete,
current version of whatever is under review, even if it feels redundant or
the change was small. Treat a "still failing" or "new finding" from a
partial re-prompt with suspicion; check whether the finding is actually
already resolved somewhere in the artifact the agent wasn't shown before
concluding it's real.

## Verifying delegated work

Reading the report replaces neither check below; it tells you where to aim them.

- Confirm the work from git, not from the report's claims: `git show --stat`,
  then read the diff against the brief.
- Re-run the scoped verification yourself. A status of DONE that the diff does
  not support is a failure, not a rounding error.
- When the report names a deliberate divergence or an untested edge, decide
  explicitly whether to accept it, and record the decision. Do not let it pass
  silently just because the suite is green.
