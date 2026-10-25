# ADR-0055 Quality Report

- **Record:** `docs/adr/0055-watch-mode-for-github-events.md`
- **Blueprint:** `docs/adr/0055-watch-mode-for-github-events-blueprint.md`
- **Date:** 2026-10-25
- **Result:** The record stays Proposed. The gate ran five rounds, which is its cap. The fact-check and the adversarial review pass. The test review stopped at FAIL in round 5, and the fixes made after that round were not reviewed.

## Gate checklist

| Phase | Latest verdict | Round | Note |
|---|---|---|---|
| Fact-check | PASS | 5 | Run inline: paths, line numbers, repo claims, GitHub docs claims, graph. |
| Adversarial review | PASS | 5 | Four low findings after the last round were fixed afterwards. |
| Test review | FAIL (stopped at the round cap) | 5 | One blocking gap (the path that reads a reported cost) was fixed afterwards and not re-reviewed. |

The record and the blueprint changed after the last review of each phase, so a gate check reports them as stale. A maintainer can run one more test review round on the current files before accepting.

## What the rounds changed

- Round 1 (critic): reviews cannot be polled with `since`, so reviews and checks became probes. The event key carries the head SHA. `auto` on day one contradicted ADR 0025, so only `label` and fixed text comments skip the track record ladder. Tool-using actions read only trusted context. `review_pr` moved to `propose`. `approve` hands the item to the lock holder. The autonomy ceiling is the lowest value across tiers. The service became a timer that runs `watch once`. S6 to S12 became direction.
- Round 1 (tests): limit trip tests, threat control tests, state transition tests, injected clock, jitter and process seams, literal commands.
- Rounds 2 to 5: `approved` state, the Extends line for ADR 0025, keys land with their first reader, a classify argv and stdin test, a cost reading test, single outcome test lists.

## Verification summary

| Claim | How checked | Result |
|---|---|---|
| Cited paths and line numbers | Script over backticked paths, `sed -n` on each line | Match |
| The six agent workflows are gone | `git log` shows #1185, `ls .github/workflows` | Confirmed, the issue text is out of date |
| `unsafe` count and workspace lint | `grep` and `Cargo.toml:19` | 65 lines, only `undocumented_unsafe_blocks` is set |
| 304 not counted, `retry-after`, `x-ratelimit-*`, events latency | docs.github.com fetch | Confirmed |
| ETag and `since` on the three list endpoints, PR `updated_at` on push | Not checked | Open item, WU-0 |
| Cost field in the `claude -p` reply | Not checked | Open item, WU-6 |
| JSON object values in the config store | Not checked | Open item, WU-1 |
