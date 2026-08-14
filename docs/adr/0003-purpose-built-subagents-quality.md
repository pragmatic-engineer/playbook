# ADR 0003 Quality Gate Report

- **Parent ADR:** `docs/adr/0003-purpose-built-subagents.md`
- **Blueprint:** `docs/adr/0003-purpose-built-subagents-blueprint.md`
- **Date:** 2026-08-07

## Quality Gate Result

**Fact-Check:**        PASS (24/24 referenced paths exist, 9/9 new files absent, all spawn sites confirmed)
**Adversarial Review:** PASS (1 WARN)
**Test Review:**       PASS (2 WARN)

Mechanism note: the fact-check ran deterministically through the shell (path existence, spawn-site content, dependency-graph and parallel-group checks). Adversarial and test review ran as a fresh read of the just-written blueprint. Async subagents in this session reported only across turns, which would have stalled a live gate, so the gate used checks collectable in-turn. Nothing was cached; every check ran against the current files.

## Verification Summary

| Referenced path | Confirmed? | Where used |
|---|---|---|
| `agents/auditor.md`, `git.md`, `reviewer.md` | Yes (test -f) | Context, System Snapshot |
| 10 command files under `commands/` | Yes (test -f) | System Snapshot, re-point WUs |
| `shell/check-shared-settings.sh`, `check-manifest.sh` + tests | Yes (test -f) | WU-1 pattern |
| `.github/workflows/shell-ci.yml`, `hooks/no-dash-guard.test.sh` | Yes (test -f) | WU-1 CI + precedent |
| `docs/authoring/01-*.md`, `docs/index.md`, `docs/internals/02-*.md` | Yes (test -f) | WU-12, model policy |
| 5 `general-purpose` sites (brainstorm:108, scope:277, adr:215, implement:128, implement:306) | Yes (grep, content match) | WU-7..10 |
| 6 `Explore` sites (scope:251,289; adr:199,219; implement:127,129) | Yes (grep) | WU-8..10 |
| `/playbook:learn-project` untyped dispatch | Yes (grep, no subagent_type) | WU-11 |
| 9 new files (agents, lint, docs) | Absent, as expected | P1, WU-1, WU-12 |

Dependency graph acyclic (WU-0, WU-1, then P1, then P2); P1 file set (5 new agent files) and P2 file set (5 command files + 2 docs files) are disjoint. Confidence: HIGH.

## WARNs (informational, not blocking)

- Adversarial: the re-point WUs (WU-8, WU-9, WU-10) swap the agent name but leave the command's inline task instructions in place, which now overlap the new agent's baked-in system prompt. The lint checks agent files, not this cross-file overlap, so part of the maintainability goal is deferred to `/playbook:implement`. Recorded as a blueprint open item (decide the source of truth).
- Test: add an `effort` out-of-range boundary case and a positive new-agent fixture to `shell/check-agents.test.sh`. Recorded as a blueprint open item.

## Structural Checks

- [x] Every Considered Alternatives entry has effort (M, L, S) and trade-off detail.
- [x] The Decision section explains why each rejected alternative was rejected.
- [x] All work units have file plans with real paths (existing confirmed, new intended).
- [x] All verification commands are literal (no placeholders).
- [x] No unresolved questions remain; open items are deferred to named downstream owners, and the TDD site is explicitly out of scope.
