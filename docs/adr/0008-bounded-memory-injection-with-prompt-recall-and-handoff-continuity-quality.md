# ADR-0008 Quality Gate Report

- **Parent ADR:** docs/adr/0008-bounded-memory-injection-with-prompt-recall-and-handoff-continuity.md
- **Blueprint:** docs/adr/0008-bounded-memory-injection-with-prompt-recall-and-handoff-continuity-blueprint.md
- **Date:** 2026-08-24

## Process note

All three phases were run self-verified by the orchestrating session rather
than through the intended subagent pipeline. Five consecutive background
dispatches (one `playbook:fact-checker`, one retry of the same, three
`general-purpose` with explicit "return a final report, do not end on a tool
call" instructions) each went idle with no report ever delivered, despite the
same mechanism working reliably for four earlier research agents in this same
session. Each stuck dispatch was stopped via `TaskStop` rather than left
running. The gate criteria below were still applied in full; nothing was
skipped or assumed to pass.

## Quality Gate Result

**Fact-Check:** PASS (1 WARN, fixed)
**Adversarial Review:** PASS (1 significant finding, fixed; 1 minor finding, fixed)
**Test Review:** PASS (4 missing scenarios, added)

## Phase 1: Fact-Check

Verified directly against source: every cited file:line in both documents
(`project_slug`, `logical_cwd`, `emit_prompt_context`, `Payload::field`, the
`session-clean-exit` dual-event registration in `wire.rs:167-169,183-185`,
the `session_init.rs:246` cap, `memory_anchors.rs`'s matching functions,
`edits.jsonl` handling, the `Node` struct fields), the acyclic dependency
graph and the disjoint-files claim behind the WU-1/WU-2 parallel group, the
existence of both referenced test files, and the absence of a reusable
Payload-construction test helper (confirmed: existing tests build payloads as
inline JSON literals, which the new scenarios can follow directly, no gap).

**Discrepancy found and fixed:** the blueprint's System Snapshot claimed "no
skill-validation tooling exists in this repo." `shell/plugin-e2e.sh:62-67`
does check that every `skills/*/SKILL.md` declares a `description` field.
Corrected to "no *behavioral* validation", since that check cannot verify
WU-2's write-path logic; the conclusion (WU-2 needs manual verification)
still holds.

Cross-checked against project memory
(`~/.claude/memory/pragmatic-engineer/playbook/`): three facts bear on this
topic (`graph-first-memory-decision`, `session-slice-is-uncapped-and-outgrew-its-budget`,
`uninstall-strands-settings-hook-entries`), all written earlier the same day
and already consistent with both documents. No missed gotcha.

Confidence: HIGH. Every citation checked against the live file, not against
the document's own text.

## Phase 2: Adversarial Review

Ran the seven-question adversarial pass from the original review brief
(scope creep, missing error paths, blast radius, internal contradictions,
dedup soundness, the deferred-trim honesty question, unearned claims)
directly against both documents.

**Significant finding, fixed:** WU-3's handoff file had no staleness
backstop. `to-learn` (the pattern WU-3 mirrors) has one:
`session_init.rs:289-314`, age-pruned at 14 days, specifically because a
read-once-then-delete file can survive its own delete step failing. WU-3
copied the read-once behavior but dropped the safety net. Without it, a
handoff whose delete silently failed (permission error, process killed
between read and delete) would re-inject into every future session in that
worktree indefinitely. Fixed: WU-3 now age-prunes at 14 days, matching
`to-learn` exactly, with its own regression-pinning test scenario.

**Minor finding, fixed:** the new file read in WU-3 (the hottest path in the
plugin, every session start) did not explicitly state the non-panicking
requirement `session_init.rs`'s own module doc already establishes as its
governing invariant. Made explicit rather than left implicit.

**Checked and cleared, not findings:**
- Whether Alternative C is "secretly B plus more": the Decision text already
  discloses this honestly, B is rejected "as the sole mechanism," not
  wholesale; the shared component (prompt-time recall) is named as shared,
  not hidden.
- Dedup soundness across a session resume: correct by construction, since
  `session_dir(payload)` is keyed by `session_id`, which a resume reuses, so
  dedup state persists exactly when it should.
- Whether the deferred description-trim inflates a claimed benefit: it does
  not; WU-1's code-level cap earns the stated "growth gets a ceiling" claim
  on its own, independent of any manual content edit.
- Scope bundling (four components in one ADR): has direct precedent in this
  repo (ADR 0004 bundled four mechanisms under one decision), and each Work
  Unit is independently committable, so the bundling does not create a
  big-bang delivery risk.

Confidence: MEDIUM-HIGH. Grounded in direct rereading of both documents plus
the cited source files; not cross-checked by an independent second reviewer,
since every dispatched reviewer failed to deliver.

## Phase 3: Test Review

Verified the Gherkin scenarios against `tests/hooks_session.rs`'s actual
harness (subprocess invocation of the real binary, scratch `HOME`/cwd,
`additional_context()` string assertions) and
`skills/engineering-standards/SKILL.md`'s testing requirements.

**Confirmed non-vacuous, checked closely:**
- WU-0's dedup scenario: not vacuous only because it asserts presence on
  turn 1 AND a distinct new match on turn 2, not just absence. Absence alone
  would pass identically on unfixed code, since nothing injects there either.
- WU-0's no-match scenario: not a regression pin (can't distinguish old code
  from new, both emit nothing), but a legitimate boundary test for a
  different failure mode (empty or malformed block), matching the existing
  `session_init_no_memory_store_emits_no_memory_block` pattern in this exact
  file.

**Four missing scenarios, added:**
1. WU-0: a missing or corrupted anchor index at prompt time must degrade to
   silence, not a crash, mirroring the module's documented invariant.
2. WU-0: a fact whose `file` path has been deleted since the graph was last
   rebuilt must be skipped, not block a real match from injecting.
3. WU-1: an exact-boundary test (16000 chars untouched, 16001 truncated to
   16000), closing the gap where only "clearly over" and "clearly under"
   were covered, exactly where an off-by-one would hide.
4. WU-3: the new age-prune backstop (added during Phase 2) needed its own
   regression-pinning test, added alongside it.

Confidence: HIGH. Checked against the real test harness file, not assumed
from the blueprint's prose alone.

## Post-gate amendment: WU-4

Added after the three-phase gate passed, in response to a direct user
question ("can we automate the session handoff before compacting?"), before
implementation started. Verified rather than assumed: `PreCompact` cannot
instruct the model, confirmed live against `precompact_warn.rs:5-6,60-72`,
which only calls `emit_system_message`. The buildable alternative reuses the
existing `capture-due` threshold trigger (`statusline.sh:339-343`) and the
existing `Stop`-block pattern (`memory_capture.rs`), extending its reason
text rather than adding a new marker, threshold, or hook event. Self-reviewed
inline for the same risk this session already flagged once (ADR 0004's own
threshold-tuning note: "too high and it races auto-compact at 90%"); the
existing threshold is left untouched rather than adjusted. Not re-run through
the three-phase subagent gate, given the session's 5/5 dispatch failure rate
that day; reviewed with the same rigor directly instead.

## Structural Checks

- [x] Every Considered Alternatives entry has effort and trade-off detail.
- [x] The Decision section explains why each rejected alternative was rejected.
- [x] All work units have file plans with real, verified paths.
- [x] All verification commands are literal (`cargo test --test hooks_graph_reader`, etc.), no placeholders.
- [x] No unresolved questions remain unaddressed; the two open items in the
      blueprint (payload event-detection signal, `Node.file` path form) are
      explicitly named as implementer-verify-first items, not silent gaps.
