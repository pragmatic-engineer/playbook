# ADR-0011 Quality Gate Result

**Fact-Check:**        PASS (done directly by the orchestrator, see note below)
**Adversarial Review:** PASS (after 3 rounds)
**Test Review:**       PASS (after 3 rounds)

## Phase 1: Fact-Check

Two `playbook:fact-checker` dispatches (`factcheck-adr0010`, `factcheck-adr0010-v2`) failed to
deliver: the first went idle for roughly 1.5 hours across three idle notifications with two
`SendMessage` follow-ups producing nothing; the second, dispatched with a named backup output
file per the established mitigation, still produced no file and no report after two more
follow-ups. Both were stopped. Fact-checking was then performed directly, citing real evidence
for every claim: `src/hooks/rebuild_memory_graph.rs:424-450`'s `Graph`/`Node`/`Edge` shapes,
`src/hooks/mod.rs`'s `HookName`/`dispatch()` pattern, `src/init/run.rs`'s `InitPaths`/`StepReport`
pattern, a live empirical probe of the `PreToolUse` `Agent` matcher against
`~/.claude/settings.json` (added, validated, triggered, reverted), and a real repo-wide grep
confirming the "30 files reference `graph.json`" claim exactly. One real correction: WU-5's test
file was wrongly assumed unconfirmed; it's `tests/hooks_turn.rs`, confirmed via grep.

Verdict: PASS. Confidence: HIGH. Caveat: this ran as direct orchestrator verification, not an
independent subagent, so it lacks the independence a separate fact-checker normally provides,
though every finding is real evidence, not assumption. This incident and its mitigation
(name the backup file at original dispatch, not reactively; treat continued idle-with-no-content
as a dead channel after one nudge) are recorded in the global `subagent-results-lost-not-hung.md`
memory fact.

## Phase 2: Adversarial Review (`critic`, focus=decision)

**Round 1** (`critic-adr0010-decision`): lost to a session interruption before delivering.

**Round 2** (`critic-adr0010-v2`): FAIL, 8 findings.
1. HIGH: `memory.signals.json`'s writer reuses `with_dir_lock` (`src/common/atomic.rs:37-52`),
   documented fail-open, but WU-1's test claimed a stronger "neither result is lost" guarantee.
2. MEDIUM: O1's auto-promotion contradicted the design doc's stated non-goal against autonomous
   behavior.
3. MEDIUM: "independently shippable" overstated what the Ordering table's real dependencies allow.
4. MEDIUM: bundling all four gaps' specific mechanisms into one hard-to-reverse ADR record, when
   only the file split is genuinely hard to reverse.
5. MEDIUM: O1's rolling-window state machine disproportionate to the one data point motivating it.
6. MEDIUM: no defined behavior for a same-repo anchor with no usable git history.
7. MEDIUM: migration didn't address concurrent sessions on different plugin versions.
8. LOW: the rejected alternative's cost wasn't quantified against the chosen one.

All 8 addressed: WU-1's test plan now separates normal-contention from the lock-exhausted path;
the design doc's non-goal was scoped to gap 3 specifically with gap 1 named as a deliberate
exception; the ADR's Decision intro was tightened; a new "What this ADR fixes versus what stays
open" paragraph scopes the hard commitment to the file split alone; a manual `pinned: true`
frontmatter override was added (WU-0 parses it, WU-3 consumes it) alongside the automatic path;
WU-4's `check_staleness` now defines `None`-handling explicitly; a new "Accepted limitation"
Consequence documents the cross-version window; the rejected alternative's trade-off now leads
with failure-domain mixing, not an unquantified merge-cost claim.

**Round 3** (`critic-adr0010-v3`): FAIL, but only because of one new, small finding: the ADR's
own gap-1 bullet described the manual override as "a manual `promoted: true` frontmatter
override," the wrong field name, colliding with the automatic path's field name the blueprint
deliberately kept distinct. All 8 round-2 findings independently re-verified as resolved. Fixed
directly (one line) and confirmed via direct grep, not a fourth agent dispatch, given the fix was
small, precisely located, and trivially verifiable.

Verdict: PASS.

## Phase 3: Test Review

**Round 1** (`testreview-adr0010`): lost to a session interruption before delivering.

**Round 2** (`testreview-adr0010-v2`): FAIL, 3/9 checks passed.
- FAIL: WU-2's Jaccard-overlap tests missed the threshold boundary and the empty-body edge case.
- FAIL: WU-3's hit-counter tests missed the exact-threshold boundary and same-turn double-dispatch.
- FAIL: WU-4's cache-hit/miss assertions named no observable mechanism; a repo-wide grep confirmed
  every existing git-shelling call site in this codebase is an unswappable `Command::new("git")`.
- WARN: WU-1's concurrent-write test was vague; a real precedent exists at
  `tests/hooks_graph_writer.rs:1177-1210`.
- WARN: WU-6's migration tests didn't specify per-scenario fixture isolation.
- WARN: WU-5's threshold-boundary gap, same class as WU-3, lower urgency given the external block.

All 6 addressed: boundary and empty-body scenarios added to WU-2; exact-threshold and
double-dispatch scenarios added to WU-3; WU-4 gained a real dependency-injection seam
(`git_lookup: &impl Fn(&Path) -> Option<DateTime>`) with a call-counting fake named in the test
plan; WU-1's test now cites the real dual-thread precedent plus the lock-exhausted scenario;
WU-6 now names per-scenario scratch-directory isolation; WU-5 carries a deferred note for when it
unblocks.

**Round 3** (`testreview-adr0010-v3`): PASS outright, all 6 re-verified resolved, including an
independent re-read confirming the `&impl Fn` injection seam is genuine dependency injection with
no type collision between the production and test call sites. One non-blocking citation nit:
WU-6 implied `tests/init_merge.rs` uses the identical helper name `scratch_home` that
`tests/init_run.rs` does; it actually uses `scratch_dir`, functionally identical, differently
named. Fixed directly and confirmed via grep.

Verdict: PASS.

## Structural Checks

- [x] Every Considered Alternatives entry has effort and trade-off detail.
- [x] The Decision section explains why each rejected alternative was rejected.
- [x] All work units have file plans with real paths.
- [x] All verification commands are literal (no `<placeholders>`).
- [x] No unresolved question remains outside a named deferral (WU-5's external block on ADR-0009;
      the settings-seed generator file location for WU-3, deferred to implementation time).
