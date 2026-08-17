# ADR 0007 test mapping: old shell suites to new Rust tests

- **Parent ADR:** `docs/adr/0007-rust-binary-for-hooks-and-launcher.md`
- **Blueprint:** `docs/adr/0007-rust-binary-for-hooks-and-launcher-blueprint.md`
- **Started:** 2026-08-18
- **Status: INCOMPLETE. Suite-level mapping is done and measured. Per-scenario
  rows are still outstanding for most suites, and WU-14 must not delete a file
  whose rows are blank.**

## What this file is for

WU-14 deletes the old python runtime and its shell suites. Its acceptance rule
is that **a row with no new-test counterpart blocks the deletion of that file**,
because "checked case by case" is not verifiable on its own.

This file was started early, well before WU-14 runs, for one reason: the mapping
is far cheaper to write while each port is fresh than to reconstruct later across
15 suites and 214 scenarios. Anything left blank here is work WU-14 still owes.

## Read this before comparing any counts

**A lower Rust test count does NOT mean lost coverage, and several suites look
alarming on a naive count.** The shell suites are one scenario per assertion;
the Rust tests are table-driven, so one `#[test]` often covers a whole fixture
matrix.

Worked example, `tests/init_merge.rs`, measured:

| Measure | Value |
|---|---|
| `#[test]` functions | 8 |
| `const FIXTURES: [Fixture; 9]` | 9 cases in ONE test |
| `const INVALID_INPUT_CASES: [InvalidInputCase; 8]` | 8 cases in ONE test |

So 8 test functions carry at least 17 distinct cases there. Comparing "19 old
scenarios to 8 new tests" and concluding coverage halved would be wrong.

**Therefore the per-scenario rows below are the actual acceptance evidence, not
the totals.** Do not sign off a suite on totals alone.

## Suite-level mapping, measured 2026-08-18

Old counts come from running each suite and reading its own summary line. New
counts come from `cargo test --test <name>`.

| Old suite | Old cases | New Rust home | New tests | Per-scenario rows |
|---|---|---|---|---|
| `hooks/search-counter.test.sh` | 6 | `tests/hooks_counter.rs` | 7 (shared) | TODO |
| `hooks/post-edit-track.test.sh` | 7 | `tests/hooks_counter.rs` | 7 (shared) | TODO |
| `hooks/preread-edit-check.test.sh` | 6 | `tests/hooks_preread.rs` | 26 (shared) | TODO |
| `hooks/preread-size-check.test.sh` | 7 | `tests/hooks_preread.rs` | 26 (shared) | TODO |
| `hooks/auto-model-detect.test.sh` | 6 | `tests/hooks_turn.rs` | 24 (shared) | TODO |
| `hooks/precompact-warn.test.sh` | 7 | `tests/hooks_turn.rs` | 24 (shared) | TODO |
| `hooks/memory-capture.test.sh` | 19 | `tests/hooks_turn.rs` | 24 (shared) | TODO |
| `hooks/rebuild-memory-graph.test.sh` | 61 | `tests/hooks_graph_writer.rs` | 24 | **TODO, highest risk** |
| `hooks/memory-anchors.test.sh` | 15 | `tests/hooks_graph_reader.rs` | 8 | TODO |
| `hooks/session-init.test.sh` | 13 | `tests/hooks_session.rs` | 16 (shared) | TODO |
| `hooks/session-clean-exit.test.sh` | 6 | `tests/hooks_session.rs` | 16 (shared) | TODO |
| `hooks/lib/common.test.sh` | 25 | `src/common/*` unit tests | 50 (shared) | TODO |
| `hooks/incr-counter.test.sh` | 7 | `src/common/counter.rs` unit tests | 50 (shared) | TODO |
| `shell/merge-settings.test.sh` | 19 | `tests/init_merge.rs` | 8 | TODO |
| `shell/gen-shared-settings.test.sh` | 10 | `tests/settings_gen.rs` | 10 | TODO |

**Totals: 214 old scenarios. 141 Rust integration tests plus 50 unit tests, 191
in all.** Again, these totals prove nothing on their own; see the caveat above.

`hooks/rebuild-memory-graph.test.sh` at 61 old scenarios against 24 Rust tests
is the widest gap and the one to fill in first. It is also the suite whose port
already shipped a real defect (inline `anchors:` silently dropped), found by
differential testing against the live memory store rather than by fixtures, so
its per-scenario rows deserve the most care.

## Suites with NO old counterpart

New behaviour, so nothing to map, and nothing blocks on them:

| New tests | Why there is no old suite |
|---|---|
| `tests/init_wire.rs` (8) | WU-8's hook wiring is new; `hooks/hooks.json` was a static registry with no test suite |
| `tests/init_shim.rs` (10) | Shim and statusline placement was inline in `shell/setup-local.sh` with no dedicated suite |

## Suites that must NOT be deleted, and when they can be

- `shell/gen-shared-settings.test.sh` and `shell/check-shared-settings.test.sh`:
  their python scripts are the differential oracles for `tests/settings_gen.rs`
  and the WU-21 port. Both deletions moved to WU-14 (blueprint amendment four).
- Every `hooks/*.test.sh` above: the python hook it tests is still live, because
  `hooks/hooks.json` still routes all 11 functional hooks at python. Deleting a
  suite while its subject is in production removes the only check on it.

## How to fill a per-scenario row

One row per assertion in the old suite:

| Old case (verbatim label from the suite) | New test | How it is covered |
|---|---|---|
| e.g. `s15: contested key freezes old base value` | `tests/init_merge.rs::mandatory_and_ported_fixtures_rust_and_python_mergers_agree` | `FIXTURES[1]`, plus a direct `NEWBASE_OUT["k"]` assertion |

Name the specific fixture or table index when a test is table-driven. "Covered
by the differential test" is not a row; it is the thing the row has to prove.
