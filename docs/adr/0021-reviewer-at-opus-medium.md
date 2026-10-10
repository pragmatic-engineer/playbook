# ADR-0021: The Reviewer Agent Ships at Opus Medium

- **Status:** Accepted
- **Date created:** 2026-10-10
- **Date modified:** 2026-10-10

## Context

The `reviewer` agent shipped on Opus at `high`. It is the most expensive agent per call and it runs once per lens in `/playbook:deep-review` and in the `/playbook:implement` review swarm. `/playbook:quick-review` already held it to `medium` through a variant. The reviewer bench (`playbook eval bench`, 10 real diffs, 7 with a seeded real finding and 3 clean, 20 runs per cell) measured:

| Config | Pass | Cost per call |
| --- | --- | --- |
| Opus high | 200/200 | $0.0155 to $0.0168 |
| Opus medium | 199/200 (recall 139/140) | $0.0125 to $0.0136 |
| Opus low | 49/50 (5 run round) | not recorded |

## Decision

1. `agents/reviewer.md` declares `effort: medium`. The model stays Opus. The decision was made by the maintainer, on the numbers above.
2. The effort policy table, the routing table row for `review` (`opus, medium`) and `quick-review` follow the file. `quick-review` still runs `playbook effort resolve agents reviewer --cap medium`, which now returns the base reviewer, or a lower variant under a lower user ceiling.
3. Escalation stays by variant. The `-xhigh` reviewer is still rendered for a session, and `playbook:delegating-subagents` still sends a security lens, a diff over 60 KB, or a diff that touches auth, secrets, crypto or untrusted input to it. There is no `-medium` variant now (a tier equal to the base renders nothing) and no `-high` variant in `auto` mode, which only adds cheaper tiers and `xhigh`.
4. `xhigh` variants are rendered only for `critic`, `implementer` and `reviewer`. `analyst` and `fact-checker` ship on Haiku and the model cap stops Haiku at `medium`, so an `xhigh` variant for them was never passed.

## Alternatives considered

- **Keep Opus high.** Rejected by the maintainer: one miss in 200 at medium against none at high does not justify 19% more cost on every lens of every review.
- **Move the reviewer to Sonnet.** Sonnet medium and high scored 200/200 at about 60% less, but the 7 seeded bugs are a small set, and a missed finding is the costly error. Not applied until a larger seeded set exists.
- **Lower to Opus low.** 49/50 on a 5 run round, too little evidence.

## Consequences

- Every base reviewer call is about 19% cheaper, and a swarm of lenses gains the most.
- Recall on bugs harder than the 7 seeded ones is unproven. Mitigations are the `-xhigh` route for risky diffs and the orchestrator sweep of every finding.
- A session started without `ccc` or `ccd` has the base reviewer only, now at medium. Ceilings only lower effort, so raising it is done with the `reviewer-xhigh` variant in a launcher session.
- A new consistency test (`tests/agents_consistency.rs`) fails when agent frontmatter, the routing doc and the routing table drift apart.
