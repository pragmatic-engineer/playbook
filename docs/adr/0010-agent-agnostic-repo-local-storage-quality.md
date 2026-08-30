# ADR-0010 Quality Gate Report

- **Parent ADR:** docs/adr/0010-agent-agnostic-repo-local-storage.md
- **Blueprint:** docs/adr/0010-agent-agnostic-repo-local-storage-blueprint.md
- **Date:** 2026-08-30

## Result

**Fact-Check:** PASS (13/13)
**Adversarial Review:** PASS (iteration 3 of 3)
**Test Review:** PASS (verification pass, requested by the user beyond the automatic cap, run after a direct manual fix)

## Phase 1: Fact-Check (run inline)

All 13 file:line citations across the record and blueprint verified against the live repo (Read, not assumed). Dependency graph acyclic. Parallel group P1 = {WU-0, WU-2, WU-4} confirmed file-disjoint. Memory gotchas (`gate-check-db-shipped`'s schema key, `commands-md-files-are-executable`, `gitignore-deny-all-hides-new-root-files`) all correctly accounted for or judged not applicable. Confidence: HIGH.

## Phase 2: Adversarial Review (focus: decision)

- **Iteration 1: FAIL.** 1 HIGH finding: the original WU-2 design migrated each markdown-convention store only at its own save-time bootstrap block. Several read sites (`commands/implement.md`'s Plan Picker glob, its progress-ledger resume read, `commands/scope.md`'s Design Doc Handoff, `commands/brainstorm.md`'s prior-design scan) ran before that store's migration would ever fire, risking a paused `/playbook:implement` run's ledger becoming invisible and already-`DONE` Work Units being re-dispatched. Also flagged: the migration snippet was unlocked (TOCTOU race) and the verification script never exercised the real read paths. Non-findings: the ADR's core decision (rename to `.playbook/`, reject the literally-floated shared `~/.config/playbook/`) and its supporting evidence (the `plan_slug` collision risk, the empirically-verified per-worktree isolation) were confirmed sound.
- **Iteration 2: FAIL.** 1 HIGH finding: the revised fix moved migration to one shared step, but the `commands/implement.md` insertion point still sat inside the "no task reference given" conditional bash block, so it never ran on the common resume-by-plan-reference path. `commands/scope.md`'s and `commands/brainstorm.md`'s placements were independently confirmed already correct.
- **Iteration 3: PASS.** Verified against live file content that the corrected `commands/implement.md` insertion point (immediately after the Step 1 heading, before the conditional branch) now runs unconditionally on every path through Step 1. Lock-mandatory change and byte-identity marker approach sanity-checked, no new issues.

## Phase 3: Test Review

- **Iteration 1: FAIL.** 2 FAIL + 3 WARN: no regression assertion that the old `.claude/state.db` path stays untouched; WU-2's migration verification was a one-off script never wired into the repo's `*.test.sh` CI convention despite that costing nothing extra; missing boundary cases (new path exists as a non-directory, both old and new already exist); unlocked TOCTOU race untested; empty-old-dir case untested; ambiguous whether the four per-store checks shared scratch state.
- **Iteration 2: WARN.** 5 of 6 prior findings resolved (regression assertion added to both integration fixtures; a permanent, CI-auto-discovered `shell/repo-local-storage-migration.test.sh` added with per-store isolated scratch repos and the two missing boundary cases). 2 new/reopened WARNs: the lock only tried for ~1s then silently proceeded unlocked, narrowing rather than closing the race; the migration snippet was hand-duplicated across three files with nothing enforcing they stay identical.
- **Iteration 3: FAIL.** 2 FAIL, both narrow: the lock was made mandatory (closing the prior WARN) and a byte-identity test case 8 was added, but the canonical snippet shown in the blueprint didn't actually contain the `# BEGIN`/`# END` markers case 8 depends on, and case 8's comparison had no non-empty check, so it would pass vacuously on three empty extracts.
- **Iteration 3 exhausted the automatic max-3 re-run cap.** The orchestrator applied a direct, mechanical fix rather than spinning a 4th automated round: added the marker comment lines to the canonical snippet itself, required a non-empty assertion before the identity comparison in case 8, and updated all three Files bullets to state the markers are part of what gets inserted.
- **Verification pass (user-requested, beyond the cap): PASS.** An independent `test-reviewer` dispatch confirmed both findings resolved against the live file, and a sanity pass over the rest of WU-2 (snippet, Files list, cases 1-7, Done When) found no regressions from the edit.

## Structural Checks

- [x] Every Considered Alternatives entry has effort and trade-off detail (A: S, B: L, C: M, D: XL).
- [x] The Decision section explains why each rejected alternative was rejected.
- [x] All work units have file plans with real, verified paths.
- [x] All verification commands are literal (no placeholders).
- [x] No unresolved questions remain; the two Open Items in Confidence + open items are explicitly named future-verification notes, not blocking gaps.

## Notes for future readers

This is the one ADR in this repo's `docs/adr/` where the Adversarial Review and Test Review phases each independently caught a real, converging bug (the migration-ordering gap) across their first two iterations, then each caught a distinct residual issue in iteration 3. If revisiting this blueprint, read `commands/implement.md`'s Step 1 structure directly before assuming the migration snippet's placement is still correct: that exact spot is what iteration 2 got wrong.
