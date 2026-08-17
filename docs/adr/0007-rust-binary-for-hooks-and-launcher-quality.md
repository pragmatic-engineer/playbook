# ADR 0007 Quality Gate Report

- **Parent ADR:** `docs/adr/0007-rust-binary-for-hooks-and-launcher.md`
- **Blueprint:** `docs/adr/0007-rust-binary-for-hooks-and-launcher-blueprint.md`
- **Date:** 2026-08-13

## Result

```
Fact-Check:         PASS  (7/7 checks)
Adversarial Review: PASS  (after revision; 2 critical + 2 major + 1 minor all addressed)
Test Review:        PASS  (after revision; 4 warnings all addressed)
```

## How this gate was run, and the caveat that matters

**All three phases ran inline in the orchestrating session, not in subagents.** Every one of the three gate agents (`fact-checker`, `critic`, `test-reviewer`) went idle without delivering a result, including two that were explicitly instructed to call `SendMessage` before finishing. Nine recovery attempts across four agents returned nothing. This matches the `subagent-results-lost-not-hung` memory fact, which now records that custom plugin agents are unreliable for result delivery while the built-in `Explore` agent is not.

Consequence, stated plainly: **the adversarial review was not independent.** The same session that wrote the ADR also attacked it. That is weaker than a genuine second opinion, and this line exists so nobody later reads this as independent confirmation. The fact-check phase is less affected, because it was executed as deterministic shell and python commands whose output is re-runnable rather than as a judgement call.

## Phase 1: Fact-Check, PASS

| Check | Result |
|---|---|
| 31 claimed source line counts | All correct against `wc -l` |
| Files marked `create` do not yet exist | Correct, 7/7 |
| Files marked `edit` or `delete` exist | Correct |
| Dependency graph acyclic | Yes, 20 Work Units |
| Per-WU `Requires` vs Ordering table | Agree, 20/20 |
| Mermaid graph vs `Requires` | Agree, 26 edges |
| Parallel group P1 file disjointness | Disjoint |
| Verification commands runnable | `cargo`, `shasum`, `gh` present; all 4 named test suites readable |

**Corrections applied (5).**

Three line citations were wrong, inherited from exploration agents and not independently checked at the time:

| Claimed | Actual | What is really there |
|---|---|---|
| `session-clean-exit.py:43-47` | `:35-36` | Line 43 is `try:`; the reason-not-other rule is at 35-36 |
| `config-drift.sh:39-40` | `:32-33` | Line 39 is `mkdir -p`; the ALWAYS-restamps comment is at 32-33 |
| `plugin-e2e.sh:41-42` | `:51-54` | Line 41 parses plugin.json version; the `py_compile`/`bash -n` validator is at 51-54 |

Two memory facts were not honoured and now are:

- `release-versioning` requires `.claude-plugin/plugin.json` version to match the release tag, since `claude plugin details` surfaces it. WU-10 now pins `Cargo.toml`, `plugin.json` and the tag together and fails the build on divergence.
- `hook-rename-lockstep-settings` is the fact that predicted the 28-hour silent outage. The dependency chain already enforced the safe order, but only implicitly. WU-14 now states the rule explicitly and forbids reordering.

## Phase 2: Adversarial Review, FAIL then PASS after revision

### FAIL 1 (critical): a 10x cheaper alternative met the same success criteria and was not considered

The ADR rested its case on maintainability and named its criteria as single-language `hooks/` with both shared libraries deleted. Porting the four bash guards to Python achieves single-language `hooks/` and deletes `common.sh` for about 320 lines against 3,148, with no new language, release pipeline, tap repo, signing or Windows. The ADR's own measurements remove the objection to it: guards measure 26ms against Python's 29ms cold start, and parallel hook execution means extra Python hooks barely move an event.

**Resolved.** Added as alternative E (effort S). The Decision section no longer claims maintainability as its justification; it states outright that the maintainability framing does not survive E, and rests instead on removing the interpreter from the runtime, which is the one thing no Python option can do. Four specific consequences are named, and the record says explicitly that if those four are not worth the cost, E is the better decision.

### FAIL 2 (critical): retiring `hooks.json` inverted a documented invariant without acknowledgement

`plugin-install-model` records that functional hooks ship in the plugin via `hooks/hooks.json` and that `gen-shared-settings.py` filters the seed to the guards precisely so regeneration never reintroduces functional hooks. WU-8 inverted both halves, and the ADR did not mention ADR 0002 at all.

**Resolved.** ADR 0002 added to the Amends line, and a dedicated section states the reversal: installing the plugin no longer delivers working hooks until `playbook init` runs. The cause (plugins cannot reference files outside their own directory) and the rejected alternative that would preserve standalone behaviour (committing binaries, alternative D) are both named. The two-step install now matches the `rtk` model deliberately rather than by accident.

### MAJOR 3: Windows was unjustified by anything in the record

**Resolved.** A section now records it as a deliberate scope addition at the maintainer's request rather than a requirement, states that no Windows user is evidenced anywhere in the repo, and marks Segment F separable with its own go/no-go.

### MAJOR 4: Segment D concentrated the blast radius

WU-11 rewrote `install.sh` and deleted both `setup-local.sh` and `merge-settings.py` in one unit, so a bug in `playbook init` would leave users unable to install with no fallback.

**Resolved.** Both deletions moved to WU-14, after WU-12's doctor layers prove the install. WU-11 now states why the fallback survives it.

### MINOR 5: three unhandled error paths

**Resolved.** Consequences now cover version skew between the binary and the plugin (with a `/playbook:doctor` warning requirement), preservation of hand-edited `settings.json` entries, and a rollback story pinning a previous version by tag.

## Phase 3: Test Review, WARN then PASS after revision

Assessed against `skills/engineering-standards/SKILL.md`. Regression pinning was specified per Work Unit rather than hand-waved, every hook named its existing `*.test.sh` as the source, WU-0 used a proper TDD cycle per `:82`, and WU-13's throwaway git repos satisfy the isolation requirement at `:80`.

| Warning | Resolution |
|---|---|
| Comparison tests could give false confidence, since "agree on every fixture" is satisfiable with trivial fixtures | Mandatory six-case fixture matrices added to WU-5 and WU-7, including a hand-added user hook entry to pin the clobber risk |
| WU-14's acceptance was unfalsifiable | Now requires a committed old-to-new test mapping table at `docs/adr/0007-test-mapping.md`; a blank row blocks deletion |
| Three `Done When` items were not observable | WU-1's byte-comparability now asserted per emit shape by test; WU-12's counterfactual replaced with an executable fixture test; WU-14's clean-machine claim replaced with a `debian:stable-slim` CI job |
| Gaps: malformed stdin, composed init idempotence, the parallel-execution assumption, flakiness | Malformed-stdin non-panic added to WU-1; full end-to-end double-install added to WU-11; the parallel-execution assumption must now be measured once in WU-12 rather than assumed; WU-10 and WU-19 marked CI-only so neither gates local `cargo test` |

## Structural Checks

- [x] Every Considered Alternatives entry has an effort rating and trade-off detail (6 alternatives)
- [x] The Decision section explains why each rejected alternative was rejected, including the closest call
- [x] All 20 Work Units have file plans with real paths
- [x] All verification commands are literal, no placeholders
- [x] No unresolved questions remain, or each is explicitly deferred to a named open item

## Amendment, 2026-08-15: WU-20 and WU-21 were not gated

This report records the gate that ran on 2026-08-13 against a 20 work unit blueprint. The blueprint now has 22: the "Rust unless it is not possible" amendment to the parent ADR added WU-20 (port `shell/gen-shared-settings.py`) and WU-21 (port `shell/check-shared-settings.py`, and move its CI lane off `shell-ci`).

The counts above are left as they were on purpose, because rewriting them would misrepresent what was actually checked. Gate them before executing, or accept the gap knowingly. The two carry a real design question the original gate never saw: moving the settings validator into the binary makes `shell-ci` depend on a compiled artifact, which is a coupling the current CI split deliberately avoids.

## Gate on WU-20 and WU-21, 2026-08-16: FAIL

```
Fact-Check:         FAIL          (4 defects, each reproduced by running a command)
Adversarial Review: INCONCLUSIVE  (agents delivered nothing)
Test Review:        INCONCLUSIVE  (agents delivered nothing)
```

**Do not execute WU-20 or WU-21 until the blueprint is amended.** Three of the four defects are in WU-21's acceptance criteria, which cannot pass as written.

### Phase 1: Fact-Check, FAIL

Run inline in the orchestrating session as commands whose output is re-runnable. Same independence caveat as the 2026-08-13 gate above.

| # | Defect | Evidence |
|---|---|---|
| 1 | WU-21's Done When demands `find . -name "*.py" -not -path "./target/*"` return nothing. It returns 15 files: 12 under `hooks/`, plus all three under `shell/`. Only WU-14 deletes the 12 hook scripts, and **nothing in the dependency graph makes WU-21 depend on WU-14**. WU-21 requires WU-20, which requires WU-8, so the graph permits an order where this criterion is unsatisfiable through no fault of the unit. | `find` output; the Ordering rows for WU-20 and WU-21 |
| 2 | WU-21's Done When says "`shell-ci` no longer invokes `python3` anywhere", but its file plan drops only one of two steps. | `.github/workflows/shell-ci.yml:88` is the `check-shared-settings.py` step WU-21 removes; `:77` is `git ls-files '*.py' \| xargs -r -n1 python3 -m py_compile`, which WU-21 never mentions |
| 3 | WU-20 plans to repoint the `GEN` variable at the binary, but the recipe hardcodes the interpreter, so repointing alone yields `python3 target/debug/playbook settings gen`. | `Makefile:14` `GEN := shell/gen-shared-settings.py`; `Makefile:20` `@python3 "$(GEN)" ...` |
| 4 | WU-21 plans to make `plugin-e2e.sh` call the binary instead of the script, but that script never invokes `cargo`, so no built binary exists at that point in the run. | `shell/plugin-e2e.sh:128` calls the python validator; no `cargo` anywhere in the file |

Noted, not a defect on its own: `.github/workflows/rust-ci.yml` runs fmt, clippy, test and audit with no explicit `cargo build`, while WU-21's verification command is `./target/debug/playbook settings check ...`.

### Phases 2 and 3: INCONCLUSIVE, not PASS

Four agents were dispatched across two rounds: the `critic` and `test-reviewer` plugin agents, then two built-in `general-purpose` agents after the first pair failed. Every one went idle without delivering, including after a direct request for partial results and an explicit instruction to deliver via `SendMessage`.

Recorded as INCONCLUSIVE rather than PASS because they checked nothing. A gate that did not run must never read as a pass. This is the second gate on this ADR where the review agents failed to return, so the next attempt should budget for running these phases inline.

### What must change before these units execute

1. Add `WU-14` to WU-21's `Requires`, or rewrite its "no `.py` files remain" criterion to name only the two files WU-21 actually deletes.
2. Extend WU-21's file plan to cover `shell-ci.yml:77`, or drop the "python3 nowhere" claim.
3. Add `Makefile:20` to WU-20's file plan.
4. Resolve `plugin-e2e.sh`: either it builds the binary, or it keeps calling a script, or the check moves out of that harness.
5. Re-run the adversarial and test-review phases, inline if the agents fail again.

### One finding from Segment C that these units should inherit

WU-8 shipped a criterion asserting a global property ("no `settings.json` entry may point under `~/.claude/hooks/`") that ranged over all 15 hooks while the unit's own tests only proved the 11 it touched. Executing it silently disabled four safety guards. WU-20's "produces no diff against the committed file" and WU-21's "returns nothing" have the same shape: both assert over a whole repository rather than over what the unit changes. Check the whole range before accepting either.

## Open items carried forward

- The parallel-execution assumption is inferred from transcript p50s and measured directly only in WU-12.
- Windows launcher semantics are asserted from documentation, not observation. `[unverified]`
- The settings-seed allowlist inversion (`settings-seed-allowlist-inversion`) is deliberately not fixed here, recorded so the omission is a choice.
- Notarisation is deferred; channel 3 on macOS relies on a documented `xattr` workaround.
