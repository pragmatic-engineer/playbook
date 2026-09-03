# ADR-0012 Quality Gate Report

- **Parent ADR:** `docs/adr/0012-unify-state-under-config-playbook.md`
- **Blueprint:** `docs/adr/0012-unify-state-under-config-playbook-blueprint.md`
- **Gate run:** 2026-09-03

## Phase 1: Fact-Check (inline, per `commands/adr.md`'s "run this phase inline, delegation has produced nothing on this repo")

**PASS.** Every core file:line citation verified against source: `session.rs:24-44`, `cc/mod.rs:24-32`, `rebuild_memory_graph.rs:62-63,52,579`, `repo.rs:21-32`, `config_drift.rs:8,21`, `gate/record.rs:92-94`, `gate/check.rs:75-77`, `gate/db.rs:115-148`, `init/memory_migrate.rs`, `settings/check.rs:27`, plus every `init/*.rs` module and docs file referenced. One real staleness issue found and fixed: the initial blueprint draft claimed `memory_dir()` was still private in `rebuild_memory_graph.rs`, but PR #317 (merged during this same session) had already promoted it into `src/common/session.rs`. Corrected before the gate ran on it.

## Phase 2: Adversarial Review (`critic`, focus `decision`)

| Round | Verdict | Findings | Resolution |
|---|---|---|---|
| 1 | FAIL | 2 HIGH (WU-3's "dual-prefix" claim protected the wrong thing, `INSTALL_PREFIXES` only gates repo CI not live machines; WU-3 had no fail-safe/ordering invariant, unlike WU-1), 4 MEDIUM (WU-1's "incomplete" destination check undefined; WU-4 missing WU-1's fail-safe invariant; the record's own deferred nested-worktree question dropped by the blueprint; the "self-defeating" rejection of Alternative B overclaiming for the repo-local half specifically), 2 LOW (speculative `.config/` tier; ADR-0010's Status field not updated) | All 9 addressed: WU-3 corrected and given an explicit MUST invariant with enforced ordering; WU-1 given a sentinel-file completion mechanism; WU-4 given the same invariant; nested worktrees resolved (no ancestry notion in `worktree_id()`); the record's Decision section rewritten to concede an honest trade-off instead of overclaiming; `.config/` tier explicitly scoped out of test obligations; WU-5 gained an ADR-0010 Status-field Done-When item |
| 2 | FAIL | 1 MEDIUM (WU-4's fail-safe invariant claimed parity with WU-1's but had no resumption test to back it) | Added `legacy_migration_resumes_correctly_after_simulated_kill_mid_copy` to WU-4, matching WU-1's rigor |
| 3 | PASS | 1 non-blocking LOW (the Confidence section's gate-history changelog only logged iteration 1) | Updated to log all three iterations |

## Phase 3: Test Review (`test-reviewer`)

| Round | Verdict | Findings | Resolution |
|---|---|---|---|
| 1 | FAIL | 4 FAIL (WU-1's "incomplete" branch untested; no interrupted-then-rerun test; WU-4's collision test only proved path distinctness, not that the actual `plan_slug` bug is fixed; no test for `gate::record`/`check` erroring cleanly on unresolvable scoping), 2 WARN (WU-2 and WU-3 Done-When items with no matching named tests) | All 6 addressed: sentinel-based completion detection plus two named resumption tests for WU-1; WU-4's collision test rewritten to a real two-worktree `gate record` round trip with identical `plan_slug`, asserting data isolation not just path difference; error-path test added; both WARNs closed with named tests. Test-reviewer independently confirmed two real precedents in this codebase to reuse rather than inventing new test infrastructure: `tests/init_merge.rs:694-749` (chmod-based crash injection) and `tests/cc_worktree.rs:1638-1739` (real `git worktree add` harness) |
| 2 | FAIL | 1 FAIL (WU-4 only tested `gate record`'s silent-fallback path, not `gate check`'s independent one), 2 WARN (two WU-1 tests read as undifferentiated duplicates; the `cc_worktree.rs` helper-reuse note didn't flag their module privacy, a real Rust visibility mismatch that would have surfaced mid-implementation) | Added `gate_check_errors_when_worktree_scoping_cannot_resolve` alongside the `gate record` version; differentiated the two WU-1 tests with a distinct assertion (end-state completeness vs. sentinel-timing); Files note now explicitly states the `cc_worktree.rs` helpers are private to an unrelated submodule and must be hoisted before reuse |
| 3 | PASS | none | Both phases converged cleanly on the third revision |

## Structural Checks

- [x] Every Considered Alternatives entry has effort and trade-off detail.
- [x] The Decision section explains why each rejected alternative was rejected, including an honest concession where the reasoning was initially overstated (Alternative B, repo-local half).
- [x] All 6 work units have file plans with real, verified paths.
- [x] All verification commands are literal (no placeholders).
- [x] No unresolved questions remain: the nested-worktree question the record explicitly deferred to the blueprint is resolved there, not silently dropped.

## Quality Gate Result

**Fact-Check:** PASS (1/1)
**Adversarial Review:** PASS (round 3 of 3; rounds 1-2 FAIL and revised, round 3 PASS outright)
**Test Review:** PASS (round 3 of 3; rounds 1-2 FAIL and revised, round 3 PASS outright)

No override needed. Both phases converged inside the normal 3-round cap.

## Verification Summary

| Referenced path | Confirmed? | Where used |
|---|---|---|
| `src/common/session.rs` | Yes (Read, 3 rounds) | WU-0, WU-1 |
| `src/cc/mod.rs` | Yes (Read, 3 rounds) | WU-0 |
| `src/hooks/rebuild_memory_graph.rs` | Yes (Read, 2 rounds) | WU-0, WU-1 (ownership boundary) |
| `src/common/repo.rs` | Yes (Read, 2 rounds) | WU-0, WU-4 |
| `src/cc/config_drift.rs` | Yes (Read, 1 round) | WU-2 |
| `src/gate/record.rs`, `src/gate/check.rs`, `src/gate/db.rs` | Yes (Read, 3 rounds) | WU-4 |
| `src/settings/check.rs`, `src/settings/gen.rs` | Yes (Read, 2 rounds) | WU-3 (INSTALL_PREFIXES scope correction) |
| `src/main.rs` | Yes (Read, 1 round) | WU-3 |
| `.github/workflows/rust-ci.yml` | Yes (Read, 2 rounds) | WU-3 |
| `settings.shared.json` | Yes (Read, 1 round) | WU-3 |
| `commands/doctor.md` | Yes (Read, 2 rounds) | WU-3 |
| `tests/init_merge.rs` | Yes (Read, 3 rounds) | WU-1, WU-3, WU-4 (crash-injection precedent) |
| `tests/cc_worktree.rs` | Yes (Read, 2 rounds) | WU-4 (real-worktree harness precedent) |
| `src/init/memory_migrate.rs`, `src/init/run.rs` | Yes (Read/Glob, 2 rounds) | WU-1, WU-3 |
| `docs/adr/0001-package-toolkit-as-plugin.md` | Yes (Read, 2 rounds) | Context, WU-5 |
| `docs/adr/0010-agent-agnostic-repo-local-storage.md` | Yes (Read, 3 rounds) | Context, Decision, WU-5 |

Confidence: HIGH.
