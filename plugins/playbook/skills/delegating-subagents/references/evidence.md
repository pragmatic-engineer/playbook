# Delegation evidence

Measurements and incidents behind the rules in `SKILL.md`. Read this when the rules look too strict, or before changing them.

## Contents

- What we measured: return rates by spawn mechanism
- What does not work: failed recovery attempts
- Re-dispatching: why a second pass needs the full artifact

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


## Re-dispatching: why a second pass needs the full artifact


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
