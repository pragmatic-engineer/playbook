# ADR 0004 Quality Gate Report

- **Parent ADR:** `docs/adr/0004-graph-first-memory.md`
- **Blueprint:** `docs/adr/0004-graph-first-memory-blueprint.md`
- **Date:** 2026-08-10

## Quality Gate Result

**Fact-Check:**        PASS (15/15 cited paths exist, 7/7 new files absent, 4/4 line citations accurate, 6/6 helpers present)
**Adversarial Review:** PASS (2 WARN)
**Test Review:**       PASS (1 WARN)

Mechanism note, stated plainly: the fact-check ran deterministically through the shell (path existence, line-number accuracy, helper presence, a Kahn topological sort of the work unit graph, and a set-intersection check of every parallel group's file plans). The adversarial and test phases were a fresh critical read of the just-written blueprint, not agent swarms. Subagents in this session repeatedly returned nothing across two separate swarms, which would have stalled a live gate, so the gate used checks collectable in turn. This matches the precedent recorded in `docs/adr/0003-purpose-built-subagents-quality.md`. Nothing was cached; every check ran against the current files.

## Verification Summary

| Referenced path | Confirmed? | Where used |
|---|---|---|
| `hooks/rebuild-memory-graph.sh` + its test | Yes (test -e) | WU-0 |
| `hooks/session-init.sh` | Yes, line 67 accurate | WU-2 |
| `hooks/post-edit-track.sh`, `hooks/lib/common.sh` | Yes, 6/6 helpers present | WU-3, WU-5 |
| `hooks/preread-edit-check.sh` | Yes | WU-3 pattern |
| `hooks/precompact-warn.sh` | Yes, line 8 constraint accurate | Context |
| `hooks/hooks.json` | Yes, 7 events registered | WU-3, WU-5 |
| `statusline.sh` | Yes, line 297 accurate | WU-4 |
| `shell/statusline.test.sh` | Yes | WU-4 |
| `commands/implement.md`, `prompts/SYSTEM_PROMPT.md`, `docs/concepts/02-memory-system.md` | Yes | WU-6, WU-7 |
| `memory/graph.json`, `shell/worktree.sh` | Yes | Context |
| 7 new files (context reader, 2 hooks, 4 suites) | Absent, as expected | WU-1, WU-2, WU-3, WU-5 |

Cycle check: PASS, 8 work units resolve in topological order (WU-0, WU-4, WU-1, WU-7, WU-6, WU-2, WU-3, WU-5). P1, P2, and P3 each verified file-disjoint with no intra-group dependency. WU-5 and WU-3 both edit `hooks/hooks.json`, and WU-5 correctly declares WU-3 as a prerequisite so they never run concurrently. Confidence: HIGH.

## WARNs (informational, not blocking)

- **Adversarial: WU-3 puts a hook on a hot path.** A `PreToolUse` hook on `Edit|Write` fires on every single edit. The ADR argues that turns which edit nothing pay nothing, which is true, but a turn that edits 50 files now pays 50 anchor lookups against a 204 KB `graph.json`. The blueprint does not specify caching. Mitigation to apply during implementation: read and index the graph once per session into the session dir, and have the hook consult that index instead of re-reading the graph per edit. Recorded as an implementation constraint on WU-3.
- **Adversarial: the capture trigger is a prompt, not an enforcement.** Already stated in the ADR's Consequences and its open items, and it is the honest limit of the hook surface, since `PreCompact` has no channel to the model. Only WU-6 is deterministic. Restated here so the gate does not appear to have missed it.
- **Test: no scenario covers a stale anchor index.** If WU-3 caches the index per session (see the first WARN), a fact written mid-session will not appear until the cache refreshes. Add a scenario pinning the chosen behaviour, whichever way it is decided.

## Structural Checks

- [x] Every Considered Alternatives entry has effort (S, M, M, XL) and trade-off detail.
- [x] The Decision section explains why each rejected alternative was rejected, with the 8.8 KB measurement as the deciding evidence for rejecting C and D.
- [x] All work units have file plans with real paths (existing confirmed, new intended).
- [x] All verification commands are literal (no placeholders).
- [x] No unresolved questions remain; open items are deferred to named downstream owners, and the `PreCompact` limitation is explicitly recorded rather than designed around.
