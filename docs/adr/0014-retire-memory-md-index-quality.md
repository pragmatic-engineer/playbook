# ADR-0014 Quality Gate Report

**Parent ADR:** `docs/adr/0014-retire-memory-md-index.md`
**Blueprint:** `docs/adr/0014-retire-memory-md-index-blueprint.md`

## Quality Gate Result

**Fact-Check:** PASS (run inline)
**Adversarial Review:** PASS (3 iterations)
**Test Review:** WARN (2 iterations; informational, non-blocking)

## Phase 1: Fact-Check (inline)

Ran mechanically against the record and blueprint before drafting the final revision:

- Every file path cited (`src/hooks/session_init.rs`, `src/hooks/rebuild_memory_graph.rs`, `shell/memory-context.sh`, `tests/hooks_session.rs`, `commands/plan.md`, `commands/adr.md`, `commands/implement.md`, `commands/deep-review.md`, `commands/learn-project.md`, `skills/playbook-usage/SKILL.md`, three docs files, `commands/doctor.md`) exists.
- Every line-number citation verified exact against real file content at time of check.
- All 10 work units confirmed file-disjoint (one parallel group, no dependency edges).
- Dependency graph trivially acyclic (no `Requires` edges among WUs).

## Phase 2: Adversarial Review (`critic`, focus `decision`)

**Iteration 1 — FAIL, 5 findings:**
1. HIGH: the five commands (`plan.md`, `adr.md`, `implement.md`, `deep-review.md`, `learn-project.md`) lost their only jq-free memory-read path once `MEMORY.md` was retired, since `shell/memory-context.sh` silently produces nothing when `jq` is missing, and nothing filled that gap.
2. MEDIUM: no way to distinguish "nothing relevant in memory" from "the script couldn't run."
3. MEDIUM: Decision Drivers/Consequences never weighed the new jq dependency for the five commands.
4. MEDIUM: WU-5's manual-verification requirement had no Done-When gate.
5. LOW: no `doctor.md` check for `jq`, a pre-existing hard dependency of several layers already.

All fixed: native jq-free fallback added to WU-1 through WU-5 (read `memory.graph.json` directly via the `Read` tool when the script produces nothing); digest now notes which source produced a result; Decision Drivers reworded to state the no-new-dependency principle applies to every reader; WU-5 got an explicit Done-When gate; new WU-8 (`doctor.md` Layer 8, jq installed).

**Iteration 2 — FAIL, 3 findings:**
1. MEDIUM: WU-8's insertion-point citation was wrong (claimed Layer 7 spans to the end of the file; it actually ends at line 260, with a separate `## Output format` section after it containing remediation-branch logic WU-8 didn't touch).
2. MEDIUM: Consequences section still framed the five commands' change as pure simplification, contradicting Decision Drivers' description of an added fallback code path.
3. LOW: a `rebuild_memory_graph.rs:697-698` citation was off by up to two lines (the real `name: None` field is at `:699`).

All fixed: WU-8's Changes corrected to the real insertion point, plus new bullets to update the two literal "seven" references and add a Layer 8 remediation branch; Consequences bullet reworded to disclose the added fallback branch as a real cost; citation corrected.

**Iteration 3 — PASS**, with one LOW finding: the `:697-698` citation fix from iteration 2 had only been applied in one of three occurrences of the same claim. Fixed (two more spots corrected to `:699`) before finalizing; explicitly noted by the reviewer as non-blocking polish, confined to rationale text with no effect on any WU's Changes or Done-When criteria.

## Phase 3: Test Review (`test-reviewer`)

**Iteration 1 — FAIL, 1 FAIL + 4 WARN:**
- FAIL: WU-0 had no test scenario for a malformed/non-JSON `memory.graph.json`, despite its Changes section committing to "never panics."
- WARN: no scenario for a graph with only code-anchor nodes and zero fact nodes.
- WARN: WU-1 through WU-4 had no manual-verification callout, unlike WU-5.
- WARN: WU-7's Done-When required an absolute zero-count grep, which would fail a correct edit documenting orphaned `MEMORY.md` files (something the ADR's own Consequences explicitly allows for).
- WARN: WU-0's cap-test scenario reused an existing fixture's character-count math without noting the native path's rendered output has no `"Facts:\n"` preamble the jq path's does.

All fixed: two new WU-0 test scenarios added (malformed JSON, code-anchor-only graph); open item added for WU-1 through WU-4's manual verification; WU-7's Done-When softened to allow one explanatory-note exception; WU-0's cap scenario now explicitly warns against copying the existing fixture's character-count assumptions.

**Iteration 2 — WARN, 2 findings (informational, passes the gate):**
- WARN: the code-anchor-only scenario's assertion was vague ("no fact-shaped line") rather than pinned to a concrete output shape.
- WARN: WU-1 through WU-5's Done-When lists didn't gate on the "note which source produced the result" instruction surviving into the final wording.

Both fixed: the code-anchor scenario's assertion now pinned to the same shape as `session_init_no_memory_store_emits_no_memory_block`; each of WU-1 through WU-5 got an added Done-When bullet requiring the source-attribution instruction to survive.

## Post-gate PR self-review (`/playbook:quick-review --self` on PR #375)

Run after the gate above had already passed and the PR was opened, as this repo's standing self-review step before promoting a PR out of draft. This is a single-pass reviewer, not the Phase 2/3 gate agents, so it's recorded separately rather than folded into either phase's iteration count.

**Findings, one BLOCKING:**
1. BLOCKING: `prompts/SYSTEM_PROMPT.md` (loaded into every session's persistent instructions, not a command file any single `/playbook:*` invocation reads) independently instructs the same MEMORY.md write convention three times (`:53`, `:61`, `:63`), and no work unit touched it. Fixed: new WU-9 added, retiring these three references; the ADR record's Context and Consequences sections updated to name this as a sixth, distinct write path.
2. ISSUE: the final repo-wide sweep grep in Confidence + open items could never return empty, since `rebuild_memory_graph.rs`'s exclusion, `memory_anchors.rs`'s doc comment, and a frozen eval fixture all deliberately keep referencing `MEMORY.md`. Fixed: the sweep instruction now names all three as expected, explaining why each is legitimate, with the real pass condition being "every match is one of these three."
3. ISSUE: the record's Consequences "Breadth" bullet didn't match the blueprint's actual file plan (omitted `doctor.md` and the system prompt, incorrectly implied `rebuild_memory_graph.rs` was modified). Fixed: reworded to list the true file set and explicitly note `rebuild_memory_graph.rs` is read and cited but not modified.
4. ISSUE: the native fallback (WU-0) pulls global and org facts via `in_promotion_scope`, but the `session_init.rs` preamble text it falls into still says "these facts apply only in this repo." Fixed: WU-0 now includes rewording that preamble, plus a matching Done-When bullet.
5. NITPICK: the `doctor.md` citation for the resolve-then-check-`-f` pattern pointed at `:123` (the MISSING report bullet), not the actual pattern at `:108-113`. Fixed.
6. NITPICK: the blueprint cited the no-active-cleanup decision at `:204-205`, which had drifted to a blank line and a section heading after earlier edits shifted the file. Corrected to the real location of that text.
7. NITPICK: `commands/doctor.md` was listed among files grep found `MEMORY.md` references in, but it has zero; it's in the WU set only for WU-8's new jq layer. Fixed: the sweep instruction now makes this explicit.

All seven fixed directly on the PR branch before merge; no further review round was run afterward, since these were all textual/planning-document corrections with no code behavior to re-verify.

## Structural Checks

- [x] Every Considered Alternatives entry (A, B, C, D) has an effort estimate and real trade-off detail.
- [x] The Decision section explains why each rejected alternative was rejected.
- [x] All 10 work units have file plans with real paths.
- [x] All verification commands are literal (`cargo test --test hooks_session <name>`, `grep -c "MEMORY.md" <file>`, `cargo build && cargo clippy --all-targets -- -D warnings`).
- [x] No unresolved questions remain unaddressed; the two remaining open items (WU-1 through WU-4's manual verification, and the repo-wide final grep sweep) are explicitly named with an owner and a verification step, not vague placeholders.
