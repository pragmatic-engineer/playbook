# ADR 0007 test mapping: old shell suites to new Rust tests

- **Parent ADR:** `docs/adr/0007-rust-binary-for-hooks-and-launcher.md`
- **Blueprint:** `docs/adr/0007-rust-binary-for-hooks-and-launcher-blueprint.md`
- **Started:** 2026-08-18
- **Status: PARTIAL. Suite-level mapping is done and measured for all 15 suites.
  Per-scenario rows are COMPLETE for 8 of 15 and outstanding for the other 7. Done:
  `rebuild-memory-graph`, `memory-anchors`, `session-init`, `session-clean-exit`,
  `search-counter`, `post-edit-track`, `preread-edit-check`, `preread-size-check`.
  WU-14 must not delete a file whose rows are blank, so today it may delete exactly
  those eight suites.**

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
| `hooks/search-counter.test.sh` | 6 | `tests/hooks_counter.rs` | 7 (shared) | **DONE, see below** |
| `hooks/post-edit-track.test.sh` | 7 | `tests/hooks_counter.rs` | 7 (shared) | **DONE, see below** |
| `hooks/preread-edit-check.test.sh` | 6 | `tests/hooks_preread.rs` | 26 (shared) | **DONE, see below** |
| `hooks/preread-size-check.test.sh` | 7 | `tests/hooks_preread.rs` | 26 (shared) | **DONE, see below** |
| `hooks/auto-model-detect.test.sh` | 6 | `tests/hooks_turn.rs` | 24 (shared) | TODO |
| `hooks/precompact-warn.test.sh` | 7 | `tests/hooks_turn.rs` | 24 (shared) | TODO |
| `hooks/memory-capture.test.sh` | 19 | `tests/hooks_turn.rs` | 24 (shared) | TODO |
| `hooks/rebuild-memory-graph.test.sh` | 61 | `tests/hooks_graph_writer.rs` | 24 | **DONE, see below** |
| `hooks/memory-anchors.test.sh` | 15 | `tests/hooks_graph_reader.rs` | 8 | **DONE, see below** |
| `hooks/session-init.test.sh` | 13 | `tests/hooks_session.rs` | 16 (shared) | **DONE, see below** |
| `hooks/session-clean-exit.test.sh` | 6 | `tests/hooks_session.rs` | 16 (shared) | **DONE, see below** |
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

## Per-scenario rows: the session pair

**COMPLETE for both. 13 + 6 = 19 old scenarios accounted for, zero blank rows, so
WU-14 may delete both suites.** Both map into `tests/hooks_session.rs`.

### `hooks/session-init.test.sh` (13)

| Old scenarios | New test | How it is covered |
|---|---|---|
| slice injected: exits 0 / valid JSON / additionalContext contains the fact name (3) | `session_init_injects_the_graph_backed_slice` | the graph-backed slice path |
| fallback to index: exits 0 / valid JSON / contains the index line (3) | `session_init_falls_back_to_the_legacy_memory_index` | the legacy MEMORY.md fallback |
| no store: exits 0 / valid JSON / no memory block emitted (3) | `session_init_no_memory_store_emits_no_memory_block` | absent store |
| not a git repo: exits 0 / valid JSON / no memory block / slice fact absent (4) | `session_init_outside_a_git_repo_emits_no_memory_block` | no repo slug, so no slice |

### `hooks/session-clean-exit.test.sh` (6)

| Old scenarios | New test | How it is covered |
|---|---|---|
| last-clean-ts refreshed / reason 'other' writes no clean-exit marker (2) | `session_clean_exit_reason_other_refreshes_ts_without_marker` | both assertions sit in the suite's reason-'other' block, verified by reading lines 44-48, so they map here and NOT to the reason-absent test |
| clean-exit marker holds the reason (1) | `session_clean_exit_real_reason_writes_the_marker` | direct |
| auto-learn flag queued with repo_root/edits/session_id/ts (1) | `session_clean_exit_queues_auto_learn_flag_with_expected_shape` | flag shape asserted |
| below threshold queues no flag (1) | `session_clean_exit_below_threshold_queues_no_flag` | direct |
| AUTO_LEARN_NUDGE=0 disables the queue (1) | `session_clean_exit_auto_learn_nudge_disabled_skips_queue` | direct |

### Rust tests with no old counterpart (7)

| New test | What it adds |
|---|---|
| `session_init_zeroes_exactly_the_five_counter_files` | Counter reset, and that it is exactly five files |
| `session_init_drift_warning_fires_only_on_resume` | Config-drift warning gated on resume |
| `session_init_resume_with_matching_hash_emits_no_drift_warning` | The negative case for that gate |
| `session_init_degrades_quietly_when_both_shell_outs_are_unreachable` | Both shell-outs failing must not break the hook |
| `session_clean_exit_reason_absent_refreshes_ts_without_marker` | A missing `reason` field, distinct from `other` |
| `session_clean_exit_at_default_threshold_queues_a_flag` | The boundary itself, not just below it |
| `session_clean_exit_padded_min_edits_env_var_still_parses` | Whitespace-padded env var parsing |

So 19 old scenarios map onto 9 Rust tests, and 7 more add coverage the shell
suites never had. That is the whole 16.

Extraction note: `session-init.test.sh` uses FOUR `assert_*` helpers and all 13
labels come out cleanly. `session-clean-exit.test.sh` uses the `ok`/`bad` pair
style that defeated a previous attempt on other suites; what works is grepping
the CALL SITES with line numbers and reading them, rather than trying to
regex-extract the quoted strings. Use that on `search-counter` and
`post-edit-track`.

## Per-scenario rows: the counter pair

**COMPLETE for both. 6 + 7 = 13 old scenarios accounted for, zero blank rows, so
WU-14 may delete both suites.** Both map into `tests/hooks_counter.rs`, which
splits them across `mod search_counter` and `mod post_edit_track`.

Unlike the earlier suites, **every one of the 7 Rust tests here maps to old
scenarios**, so there is no separate new-coverage table. The added coverage is
inside existing tests instead, noted per row.

### `hooks/search-counter.test.sh` (6) into `mod search_counter` (4 tests)

| Old scenarios | New test | How it is covered |
|---|---|---|
| Grep bumps search-count to 4 / tool-count tracks every call (2) | `grep_bumps_search_count_and_tool_count` | one test asserting both counters, as its name says |
| threshold nudge fires at 4 / no nudge on count 5 (2) | `threshold_nudges_fire_at_four_eight_twelve_and_then_fall_silent` | a 5-case table, (4, fires), (5, silent), (8, fires), (12, fires), (13, silent). Carries both old labels AND adds the 8, 12 and 13 thresholds the shell suite never tested |
| Read of same path counts once (1) | `read_of_same_path_counts_once` | the dedup rule |
| no session id is a silent no-op (1) | `no_session_id_is_a_silent_noop` | in `mod search_counter` |

### `hooks/post-edit-track.test.sh` (7) into `mod post_edit_track` (3 tests)

| Old scenarios | New test | How it is covered |
|---|---|---|
| Edit records a path/ts line / edit-count bumped to 1 / Write appends a second line / edit-count now 2 / NotebookEdit honours notebook_path (5) | `edit_write_notebookedit_accumulate_in_session` | 7 assertions covering all three tool shapes, the `edit-count` file, `edits.jsonl` and the `notebook_path` fallback |
| Read is a no-op (1) | `non_edit_tool_is_a_noop` | direct |
| no session id is a silent no-op (1) | `no_session_id_is_a_silent_noop` | in `mod post_edit_track`, a separate test from the search-counter one of the same name |

Extraction note: these are the two suites an earlier attempt gave up on. The
technique that works is the one recorded for `session-clean-exit`, grep the
`ok "` and `bad "` CALL SITES with line numbers and read them. Regex-extracting
the quoted strings fails here because each scenario has an `ok` and a `bad`
variant with interpolated text, so a naive extract returns roughly double the
real count. Their Rust counterparts also sit inside per-hook `mod` blocks, so
the `#[test]` functions are indented and an unindented `^fn ` grep finds none.

## Per-scenario rows: the preread pair

**COMPLETE for both. 6 + 7 = 13 old scenarios accounted for, zero blank rows, so
WU-14 may delete both suites.** Both map into `tests/hooks_preread.rs`, split
across `mod preread_edit_check` (11 tests) and `mod preread_size_check` (15).

**This pair inverts the usual warning.** Elsewhere in this file a LOWER Rust
count looked like lost coverage; here 13 old scenarios become 26 Rust tests. The
13 extra are not padding: they are exactly the boundary pinning WU-3 mandated,
the 1800s window, the 1000-line and 200KB thresholds, the 25-pattern allowlist,
and asserting the deny shape exactly. Counting tests misleads in both directions;
only the rows below settle it.

### `hooks/preread-edit-check.test.sh` (6) into `mod preread_edit_check`

| Old scenario | New test | How it is covered |
|---|---|---|
| recent edit nudges with age (1) | `recent_edit_nudges_with_age` | direct |
| emits a valid PreToolUse object (1) | `nudge_emits_a_valid_pretooluse_additional_context_object` | direct |
| edit outside window stays silent (1) | `edit_older_than_the_window_stays_silent` | direct |
| unrelated path stays silent (1) | `unrelated_path_stays_silent` | direct |
| seconds-scale age renders (1) | `seconds_scale_age_renders_as_n_seconds_ago` | direct |
| no edits file is a silent no-op (1) | `no_edits_file_is_a_silent_no_op` | direct |

Adds, with no old counterpart:

| New test | What it adds |
|---|---|
| `edit_just_inside_the_1800s_window_still_nudges` | 1800s window, inside |
| `edit_at_the_1800s_window_boundary_stays_silent` | 1800s window, exactly at the boundary |
| `float_ts_inside_window_still_nudges` | Float timestamp records |
| `string_ts_record_does_not_abandon_a_later_match` | A malformed record must not stop the scan |
| `format_ago_scale_transitions_match_the_python_original` | Every age-rendering scale transition |

### `hooks/preread-size-check.test.sh` (7) into `mod preread_size_check`

| Old scenario | New test | How it is covered |
|---|---|---|
| large file denied with counts (1) | `large_non_allowlisted_file_is_denied_with_counts` | direct |
| small file passes (1) | `small_file_passes` | direct |
| allowlisted large file passes (1) | `allowlisted_large_file_passes` | direct |
| explicit offset bypasses the guard (1) | `explicit_offset_bypasses_the_guard` | direct |
| explicit limit bypasses the guard (1) | `explicit_limit_bypasses_the_guard` | direct |
| missing file is a silent no-op (1) | `missing_file_is_a_silent_no_op` | direct |
| tsconfig.*.json glob allowlisted (1) | `tsconfig_glob_allowlist_matches` | direct |

Adds, with no old counterpart:

| New test | What it adds |
|---|---|
| `allowlisted_basenames_never_deny` | The 25-pattern allowlist as a whole |
| `exactly_1000_lines_passes` | 1000-line threshold, at |
| `exactly_1001_lines_denies` | 1000-line threshold, one over |
| `exactly_200kb_passes` | 200KB threshold, at |
| `one_byte_over_200kb_denies_even_with_few_lines` | 200KB threshold, one byte over, with line count low |
| `unreadable_small_file_is_a_silent_no_op` | Unreadable file, small |
| `large_and_unreadable_file_is_still_denied_on_byte_size` | Unreadable file, large, still denied on bytes |
| `deny_output_matches_the_python_reference_byte_for_byte` | The deny shape, byte for byte against python |

So 13 old scenarios map onto 13 Rust tests one for one, and 13 further tests
add the mandated boundary coverage. That is the whole 26.

## How to fill a per-scenario row

One row per assertion in the old suite:

| Old case (verbatim label from the suite) | New test | How it is covered |
|---|---|---|
| e.g. `s15: contested key freezes old base value` | `tests/init_merge.rs::mandatory_and_ported_fixtures_rust_and_python_mergers_agree` | `FIXTURES[1]`, plus a direct `NEWBASE_OUT["k"]` assertion |

Name the specific fixture or table index when a test is table-driven. "Covered
by the differential test" is not a row; it is the thing the row has to prove.
