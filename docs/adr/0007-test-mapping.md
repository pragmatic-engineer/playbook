# ADR 0007 test mapping: old shell suites to new Rust tests

- **Parent ADR:** `docs/adr/0007-rust-binary-for-hooks-and-launcher.md`
- **Blueprint:** `docs/adr/0007-rust-binary-for-hooks-and-launcher-blueprint.md`
- **Started:** 2026-08-18
- **Status: PARTIAL. Suite-level mapping is done and measured for all 15 suites.
  Per-scenario rows are COMPLETE for 2 of 15 (`rebuild-memory-graph` and
  `memory-anchors`) and outstanding for the other 13. WU-14 must not delete a file
  whose rows are blank, so today it may delete exactly those two suites.**

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
| `hooks/rebuild-memory-graph.test.sh` | 61 | `tests/hooks_graph_writer.rs` | 24 | **DONE, see below** |
| `hooks/memory-anchors.test.sh` | 15 | `tests/hooks_graph_reader.rs` | 8 | **DONE, see below** |
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

## Per-scenario rows: `hooks/rebuild-memory-graph.test.sh`

**COMPLETE. All 61 old scenarios accounted for, zero blank rows, so WU-14 may delete this suite.**

Old labels are grouped by the suite's own naming (`scalar link: ...`), because each
group maps to exactly one Rust test. The count in brackets is how many old labels
the row covers, and they sum to 61.

| Old scenarios | New test in `tests/hooks_graph_writer.rs` | How it is covered |
|---|---|---|
| scalar link: edge count / from / to / relation (4) | `scalar_link_produces_one_edge` | one edge asserted with from, to and relation checked in the same test |
| inline list: edge count / alpha / beta / gamma edge (4) | `inline_list_produces_one_edge_per_target` | asserts one edge per target across the three named targets |
| single-element list: edge count / target id has no brackets (2) | `single_element_inline_list_has_no_brackets_in_target_id` | pins the bracket-stripping bug directly |
| quoted items: edge count / double-quoted clean / single-quoted clean (3) | `quoted_inline_items_parse_to_clean_names` | both quote styles asserted clean |
| empty list: no edges / node still written, no crash (2) | `empty_inline_list_produces_no_edges` | asserts zero edges and that the node survives |
| nested block list: edge count / item-one / item-two (3) | `nested_block_list_produces_one_edge_per_item` | one edge per block item |
| anchors: edge count / 2 code nodes / 2 edges (5) | `anchors_produce_code_nodes_and_anchors_edges` | 5 assertions, one per old label |
| dangling target: edge emitted / to id / target absent (3) | `dangling_target_still_emits_its_edge` | edge emitted, id checked, target node asserted absent |
| project scope: node id and project / global scope: node id / project target keeps prefix (3) | `project_scoped_and_global_facts_get_distinct_ids` | both scopes and the prefix asserted together |
| outside write: baseline one node / graph untouched (2) | `outside_memory_dir_write_is_a_no_op` | baseline then no-op asserted |
| malformed frontmatter: exits cleanly / valid JSON / absent falls back / unclosed falls back (4) | `malformed_or_absent_frontmatter_falls_back_to_defaults` | two fixtures, no-frontmatter and unclosed; exit 0 asserted; valid JSON implied by read_graph failing otherwise. Verified by reading the test, not by name |
| cross-scope resolve: edge count / project reaches global (2) | `project_fact_resolves_a_link_to_a_global_target` | both asserted |
| own scope wins: targets project dup / does not fall through (2) | `own_scope_wins_over_a_same_named_global_fact` | both directions asserted |
| project dangling: edge emitted / same-scope id / target absent (3) | `project_scoped_dangling_target_uses_same_scope_id` | all three asserted |
| global source unaffected: edge count / from / to (3) | `global_source_is_unaffected_by_project_scope_resolution` | all three asserted |
| anchors regression pin: edge count / code node / anchors edge / link edge (4) | `anchors_and_links_on_the_same_fact_are_independent` | independence of anchors and links on one fact |
| inline top-level anchors: edge count / 2 code nodes / 2 edges (5) | `inline_top_level_anchors_produce_code_nodes_and_anchors_edges` | the inline-anchors defect this suite exists to pin |
| inline vs block anchors: edge count / 2 dedup / 2 anchor edges (5) | `inline_and_block_style_anchors_produce_the_same_code_nodes_and_edges` | dedup asserted across both styles |
| duplicate top-level key: later scalar evicts earlier dict (1) | `later_top_level_key_redeclaration_evicts_the_earlier_shape` | direct |
| bare CR: scalar not corrupted (1) | `bare_carriage_return_does_not_corrupt_a_scalar_value` | direct |

### Rust tests with no old counterpart (coverage the shell suite never had)

| New test | What it adds |
|---|---|
| `top_level_scalar_description_flows_into_the_node` | Node payload, not just edges |
| `supersedes_chain_two_links_long_resolves_both_hops` | Multi-hop `supersedes` resolution |
| `graph_write_is_atomic_a_failed_write_cannot_truncate_the_existing_file` | Atomicity, untested in shell |
| `python_and_rust_writers_agree_on_the_same_fixture_tree` | Differential against the real python writer |

So 61 old scenarios map onto 20 Rust tests, and 4 further Rust tests add coverage
that never existed. That is the whole 24. This is why counting test functions
misleads: 61 to 24 looks like a loss and is actually a gain.

## Per-scenario rows: `hooks/memory-anchors.test.sh`

**COMPLETE. All 15 old scenarios accounted for, zero blank rows, so WU-14 may delete this suite.**

| Old scenarios | New test in `tests/hooks_graph_reader.rs` | How it is covered |
|---|---|---|
| scenario 1: anchored file names the matching fact / scenario 3: depends_on neighbour is named (2) | `anchored_file_names_matching_fact_and_depends_on_neighbour` | one test covering both, as its name says |
| scenario 2: directory anchor names the containing-directory fact (1) | `directory_anchor_names_the_containing_directory_fact` | direct |
| scenario 4: unanchored path emits nothing (1) | `unanchored_path_emits_nothing` | direct |
| scenario 5: malformed payload / missing file_path / missing graph, each asserting exits 0 and emits nothing (6) | `never_blocks_on_malformed_payload_missing_file_path_or_missing_graph` | three labelled sub-blocks 5a, 5b, 5c with 2 assertions each, 6 in total. Verified by reading the test |
| scenario 6: index file was built on first edit / index was rebuilt on the second edit instead of reused (2) | `anchor_index_is_built_once_and_reused_on_second_edit` | **read the condition, not the label.** Both old labels are inline FAILURE messages, so the second one describes the failure; the passing condition is that a planted marker SURVIVES, meaning the index was reused. It matches the Rust test rather than contradicting it |
| scenario 7: fact added after cache build is not surfaced this session (1) | `stale_cache_does_not_surface_a_fact_added_after_it_was_built` | direct, pinned staleness |
| scenario 8: additionalContext output is valid JSON / hookEventName is PreToolUse (2) | `additional_context_output_is_valid_json_with_pretooluse_event_name` | one test covering both |

Extraction note, because it nearly produced a wrong mapping: this suite uses
THREE assertion helpers (`check`, `check_contains`, `check_true`) plus two
inline `PASS=$((PASS + 1))` sites. Grepping only `check` finds 9 of 15 and
silently drops 6. Always reconcile the extracted label count against the
suite's own summary line before mapping anything.

### Rust tests with no old counterpart

| New test | What it adds |
|---|---|
| `python_and_rust_readers_agree_on_the_same_fixture` | Differential against the real python reader |

So 15 old scenarios map onto 7 Rust tests, plus 1 differential test, which is
the whole 8.

### Suites whose labels resist clean extraction

`hooks/search-counter.test.sh` and `hooks/post-edit-track.test.sh` were attempted
and deliberately NOT mapped, so nobody repeats the dead ends. Their assertions use
`ok "..."` / `bad "..."` helper pairs at varying indentation, with the label text
interpolated (`"tool-count (got $(tcount))"`), so there is no clean list to lift:
grepping the pair yields roughly double the real scenario count, and grepping only
`ok` at line start yields almost nothing. Their Rust counterparts in
`tests/hooks_counter.rs` also sit inside per-hook `mod` blocks, so the `#[test]`
functions are indented and a `^fn ` grep misses all of them.

Map those two by reading both files case by case rather than by extraction. The
measured targets are 6 and 7 old scenarios against 7 Rust tests across two mods.

## How to fill a per-scenario row

One row per assertion in the old suite:

| Old case (verbatim label from the suite) | New test | How it is covered |
|---|---|---|
| e.g. `s15: contested key freezes old base value` | `tests/init_merge.rs::mandatory_and_ported_fixtures_rust_and_python_mergers_agree` | `FIXTURES[1]`, plus a direct `NEWBASE_OUT["k"]` assertion |

Name the specific fixture or table index when a test is table-driven. "Covered
by the differential test" is not a row; it is the thing the row has to prove.
