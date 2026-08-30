---
name: delegating-subagents
description: Use before dispatching any subagent, and again the moment one finishes or goes quiet. Agents that can write deliver by file and the orchestrator MUST read it; read-only agents have no reliable channel, so run their pass inline and never treat silence as a clean result.
---

# Delegating to Subagents

A subagent's return value is not a delivery channel. Treat it as a courtesy.
The artifact the agent writes to disk is the delivery channel.

## The rule

1. **Every brief names an output file.** Absolute path, stated in the prompt.
2. **The orchestrator reads that file** the moment the agent finishes, goes
   idle, or is given up on. Unconditionally, before drawing any conclusion.
3. **A missing file is a distinct outcome** from an agent that found nothing.
   Say which one happened.

## Why, with numbers

Measured across 22 delegations in one session on 2026-08-16 and 17:

| Spawn mechanism | Returned a result inline |
|---|---|
| Skill tool with `context: fork` and an `agent:` in frontmatter | 11 of 11 |
| Agent tool, any `subagent_type`, plugin or built-in | 0 of 11 |

The Agent-tool spawns were not hung and not idle. Four of them wrote code, ran
their tests and committed. Two wrote full reports to the paths their briefs
named. The only signal that came back was an idle notification carrying no
payload.

The cost of ignoring this: two blocking defects sat in a written report for a
day while the orchestrator rediscovered them by hand, and a third finding in
another report was never seen at all. The reports were on disk the whole time.

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

**For the seven read-only agents there is no reliable delivery channel at all.** Their
only route is the return value, and that is the route that fails. So:

- **Do the pass inline instead.** For mechanical work this is simply better, not
  a fallback: an ADR fact-check (path existence, line counts, graph acyclicity,
  table agreement) runs as a handful of shell commands in about two minutes and
  produces re-runnable output, while the delegated version returned nothing
  across three attempts.
- **If you do delegate one, treat silence as NOT RUN.** Never as "reviewed
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
reliable.

**Reading the report is not optional even when the work looks obviously fine.**
The reports that mattered most were written by agents whose commits were green,
whose tests passed, and whose diffs looked correct. What they recorded was what
they had chosen NOT to do, and why. That is exactly the information a passing
test suite cannot give you.

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
