# ADR 0007 test mapping: old shell suites to new Rust tests

- **Parent ADR:** `docs/adr/0007-rust-binary-for-hooks-and-launcher.md`
- **Blueprint:** `docs/adr/0007-rust-binary-for-hooks-and-launcher-blueprint.md`
- **Started:** 2026-08-18
- **Status: COMPLETE for all 19 suites, and the scope is stated deliberately.**
  - **The 15 ported python suites: COMPLETE.** Suite-level mapping is done and
    measured for all 15. Per-scenario rows are complete for 15 of 15, covering
    all 214 old scenarios with zero blank rows. WU-14's acceptance rule is
    satisfied for every one of those suites and both `shell/` python scripts it
    deletes.
  - **The 4 guard suites: COMPLETE, 96 of 96 scenarios mapped.** Mapped
    2026-08-22. `hooks/*.sh | delete | the 4 guards` is in WU-14's file list, so
    the same acceptance rule applies to them. The mapping opened 9 blank rows in
    `hooks/rm-workspace-guard.test.sh`; **all 9 were closed the same day** by 6
    new tests in `tests/hooks_guards.rs::rm_workspace_guard`, taking that file
    from 30 tests to 36. All four guard suites are now clear to delete.
  - **Temp-directory exemption, added 2026-08-22 after the mapping.** Paths
    inside `/tmp` and `/private/tmp` are now allowed regardless of
    `PLAYBOOK_SAFE_ROOTS`, the temp root itself still blocks. Added to BOTH
    implementations at once and to both suites: 6 scenarios to the shell suite
    (30 -> 36) and 2 tests to the Rust one (36 -> 38). Changing only the Rust
    side would have made WU-14d's deletion a silent behaviour change, since the
    shell guard is still what a pre-`init` machine actually runs.
  - **Quoted-token fix, added 2026-08-22.** A `rm` inside a quoted region now
    counts as a command only when a separator puts it in command position, so a
    commit message or PR title that merely mentions the guard stops blocking.
    Outside quotes nothing changed, so `sudo rm`, `xargs rm` and `find -exec rm`
    are all still caught. 7 scenarios to the shell suite (36 -> 43) and 2 tests
    to the Rust one (38 -> 40), again in both implementations at once.
  - **Combined: 310 old scenarios across 19 suites, zero blank rows.**

**The earlier version of this line read "COMPLETE ... for all 15 of 15 suites"
with no scope qualifier, while the four guard suites had no rows at all.** Read
quickly, it said the gate was satisfied. It was not, and the file it cleared by
implication is the one that blocks `rm` outside the workspace. Anything that
narrows this file's scope must say so in the status line itself, not only in the
section that does the narrowing.

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
| `hooks/auto-model-detect.test.sh` | 6 | `tests/hooks_turn.rs` | 24 (shared) | **DONE, see below** |
| `hooks/precompact-warn.test.sh` | 7 | `tests/hooks_turn.rs` | 24 (shared) | **DONE, see below** |
| `hooks/memory-capture.test.sh` | 19 | `tests/hooks_turn.rs` | 24 (shared) | **DONE, see below** |
| `hooks/rebuild-memory-graph.test.sh` | 61 | `tests/hooks_graph_writer.rs` | 24 | **DONE, see below** |
| `hooks/memory-anchors.test.sh` | 15 | `tests/hooks_graph_reader.rs` | 8 | **DONE, see below** |
| `hooks/session-init.test.sh` | 13 | `tests/hooks_session.rs` | 16 (shared) | **DONE, see below** |
| `hooks/session-clean-exit.test.sh` | 6 | `tests/hooks_session.rs` | 16 (shared) | **DONE, see below** |
| `hooks/lib/common.test.sh` | 25 | `src/common/*` unit tests | 50 (shared) | **DONE, see below** |
| `hooks/incr-counter.test.sh` | 7 | `src/common/counter.rs` unit tests | 50 (shared) | **DONE, see below** |
| `shell/merge-settings.test.sh` | 19 | `tests/init_merge.rs` | 8 | **DONE, see below** |
| `shell/gen-shared-settings.test.sh` | 10 | `tests/settings_gen.rs` | 10 | **DONE, see below** |

**Totals: 214 old scenarios. 141 Rust integration tests plus 50 unit tests, 191
in all.** Again, these totals prove nothing on their own; see the caveat above.

### The 4 guard suites, measured 2026-08-22

These were missing from this table entirely until 2026-08-22, which is how the
status line above came to imply a gate they had never been checked against.
WU-14 deletes them (`hooks/*.sh | delete | the 4 guards`), so they need rows on
the same terms as everything else. All four map into `tests/hooks_guards.rs`,
whose 42 tests sit in four `mod` blocks named for the four scripts.

| Old suite | Old cases | New Rust home | New tests | Per-scenario rows |
|---|---|---|---|---|
| `hooks/bg-await-guard.test.sh` | 19 | `tests/hooks_guards.rs::bg_await_guard` | 6 | **DONE, see below** |
| `hooks/no-dash-guard.test.sh` | 17 | `tests/hooks_guards.rs::no_dash_guard` | 8 | **DONE, see below** |
| `hooks/precommit-check.test.sh` | 12 | `tests/hooks_guards.rs::precommit_check` | 7 | **DONE, see below** |
| `hooks/rm-workspace-guard.test.sh` | 48 | `tests/hooks_guards.rs::rm_workspace_guard` | 21 | **DONE, see below** |

**Totals: 96 old guard scenarios against 42 Rust tests, all 96 mapped.** This is
the suite where the naive-count warning at the top of this file did *not* explain
the gap away. 19 old bg-await scenarios really do collapse into 3 table-driven
Rust tests holding the same 19 command strings verbatim, and that is honest
compression. The 9 rm-workspace blanks were not compression: the behaviour was
absent, and the count went 30 -> 36 to supply it rather than the table being
edited to look complete.

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
  **Amended 2026-08-22: this reason has expired.** `hooks/hooks.json` now holds
  one `"command"`, the `SessionStart` migration shim, and a real install wires
  16 hooks as bare `playbook hook <name>`. The python hooks are no longer in
  production, so the rule above no longer blocks anything; the suites that
  remain are gated by their mapping rows alone.
- **`hooks/rm-workspace-guard.test.sh`: was blocked on 9 unmapped scenarios, now
  cleared.** Added and resolved 2026-08-22. It was held back not because its
  subject was live but because the mapping found 9 scenarios, concentrated on
  the zero-config default-root path, that no Rust test reached. Six new tests
  closed them, each verified by mutating the guard rather than by a green run.
  See the per-scenario section near the end of this file.

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

## Per-scenario rows: the turn trio

**COMPLETE for all three. 6 + 7 + 19 = 32 old scenarios accounted for, zero blank
rows, so WU-14 may delete all three suites.** They map into `tests/hooks_turn.rs`
across three mods, 24 tests in total: 17 carry old scenarios and 7 are new.

### `hooks/auto-model-detect.test.sh` (6) into `mod auto_model_detect` (10)

| Old scenarios | New test | How it is covered |
|---|---|---|
| nudges on design intent with a UserPromptSubmit object (1) | `design_intent_nudges_with_user_prompt_submit_object` | direct |
| slash command stays silent (1) | `slash_command_stays_silent` | direct |
| short prompt stays silent (1) | `short_prompt_stays_silent` | direct |
| plain prose stays silent (1) | `plain_prose_stays_silent` | direct |
| empty prompt silent, exit 0 (1) | `empty_prompt_stays_silent_and_exits_zero` | direct |
| architecture/migration keywords trigger (1) | `architecture_and_migration_keywords_trigger` | direct |

Adds, with no old counterpart:

| New test | What it adds |
|---|---|
| `representative_sample_of_regex_branches_all_trigger` | Every regex branch, not just two keywords |
| `word_boundary_near_miss_does_not_trigger` | The negative side of the word boundary |
| `keyword_glued_to_cjk_text_does_not_trigger` | CJK text, the Unicode word-boundary defect fixed in #117-#122 |
| `keyword_glued_to_accented_text_does_not_trigger` | Accented text, same defect |

### `hooks/precompact-warn.test.sh` (7) into `mod precompact_warn` (5)

| Old scenarios | New test | How it is covered |
|---|---|---|
| emits a valid systemMessage object (1) | `emits_a_valid_system_message_object` | direct |
| interpolates the trigger (1) | `interpolates_the_trigger` | direct |
| missing trigger defaults to auto in message / log records trigger=unknown when absent (2) | `missing_trigger_defaults_to_auto_in_message_and_unknown_in_log` | one test covering both halves of the documented divergence, auto in the message but unknown in the log |
| exit 0 on empty payload (1) | `exits_zero_on_empty_payload` | direct |
| log capped at 500 lines / log cap keeps the newest window, not the oldest (2) | `log_is_capped_at_500_lines` | asserts the cap AND the direction, that the oldest surviving line starts the newest window. Verified by reading the assertions |

No new-coverage tests in this mod; the 7 old scenarios consolidate into 5.

### `hooks/memory-capture.test.sh` (19) into `mod memory_capture` (9)

| Old scenarios | New test | How it is covered |
|---|---|---|
| marker present: exits 0 / valid JSON / decision is block / reason non empty / marker cleared (5) | `marker_present_fires_once_and_clears_the_marker` | fires once and clears, all five asserted |
| second call: exits 0 / no output (2) | `second_call_with_marker_already_consumed_is_silent` | direct |
| no marker: exits 0 / no output (2) | `no_marker_at_all_is_silent` | direct |
| edited paths: valid JSON / names first path / names second path (3) | `edited_paths_are_named_in_the_reason` | direct |
| capped list: valid JSON / at most a handful listed / notes more not shown (3) | `path_list_is_capped_at_five_with_a_more_note` | the cap and the more-note |
| missing edits log: exits 0 / valid JSON / decision still block / reason still non empty (4) | `missing_edits_log_still_blocks_with_a_reason` | all four asserted |

Adds, with no old counterpart:

| New test | What it adds |
|---|---|
| `path_list_with_exactly_five_paths_has_no_more_note` | The cap boundary, at five |
| `path_list_with_exactly_six_paths_notes_one_more` | The cap boundary, one over |
| `malformed_non_object_line_does_not_discard_the_other_paths` | A bad line in edits.jsonl must not lose the good ones |

Extraction note: `memory-capture.test.sh` puts the LABEL LAST, as the third
argument to `assert_eq` and `assert_contains` and the second to
`assert_valid_json`. Grabbing the first quoted string on the line returns
interpolated values instead and yields 25 hits against a real 19. Take the last
quoted token per call site.

## Per-scenario rows: the two oracle suites

**COMPLETE for both. 19 + 10 = 29 old scenarios accounted for, zero blank rows.**
These two matter most for sequencing: their python scripts are the live
differential oracles, so these rows are what let WU-14 delete the scripts as well
as the suites.

### `shell/merge-settings.test.sh` (19) into `tests/init_merge.rs` (8 tests)

`tests/init_merge.rs` carries its own coverage map in the file header, which is
the source of truth. Transcribed here, and while checking it I found it named 18
of the 19 scenarios: **s2 was absent**. The coverage existed all along, since the
`FIXTURES` entry it cites for s6 and s15 IS s2's scenario, but an auditor
following WU-14's rule would rightly have blocked on a scenario with no row. The
test file's map now names s2 too.

| Old scenarios | New test or fixture | How it is covered |
|---|---|---|
| s1 user-unchanged key gets template value (1) | `FIXTURES` "s1" | in the table-driven differential loop |
| s2 user-changed key is preserved / s6 conflict keeps user with one skip entry / s15 contested key frozen to OLD base in NEWBASE (3) | `FIXTURES` "user key modified from base" | one fixture carrying all three, plus a direct assertion that NEWBASE holds the OLD base value. **s2 was missing from the test file's own coverage map and is added in this change** |
| s3 new template key added to output (1) | `FIXTURES` "s3" | direct |
| s4 template-dropped unchanged key absent (1) | `FIXTURES` "template key removed" | direct |
| s5 absent base gives additive fallback (1) | `n4_missing_base_becomes_empty_object_with_warning_not_hard_fail` | N4 soft-fail |
| s7 corrupt TEMPLATE / s8 missing TEMPLATE / s9 corrupt USER / s10 USER is array / s11 USER is scalar, all N2 fail closed (5) | `INVALID_INPUT_CASES` | an 8-case table asserting non-zero exit and empty stdout on both engines |
| s12 USER == {} gives output equal to template (1) | `FIXTURES` "s12" | direct |
| s13 corrupt BASE gives additive fallback plus warning (1) | `n4_invalid_base_becomes_empty_object_with_warning_not_hard_fail` | N4, invalid rather than missing |
| s14 type-mismatch on a contested key keeps the user value (1) | `FIXTURES` "s14" | direct |
| s16 C2 coincidence across three cycles (1) | `c2_coincidence_keeps_user_value_frozen_through_a_matching_template_cycle` | chained through two real merge calls per engine |
| s17 skip file is a valid JSON array / s18 merged stdout and NEWBASE are valid JSON (2) | asserted inline in the `FIXTURES` loop | every case producing a skip entry checks both |
| s19 zero withheld keys gives an empty skip array (1) | `n3_zero_withheld_keys_writes_empty_skip_array` | N3 |

### `shell/gen-shared-settings.test.sh` (10) into `tests/settings_gen.rs` (10 tests)

| Old scenarios | New test or fixture | How it is covered |
|---|---|---|
| happy path: canned perms, model stripped, personal keys dropped, passthrough (1) | `happy_path_canned_perms_model_stripped_personal_keys_dropped_passthrough` | direct |
| model absent stays absent (1) | `model_absent_in_source_stays_absent` | direct |
| model in source is stripped (1) | `model_in_source_is_stripped` | direct |
| malformed source / missing source / missing permissions, all non-zero exit with empty stdout (3) | `malformed_or_missing_inputs_guard_rejects_with_no_output` | a table covering all three input-guard shapes |
| degenerate permissions {} rejected / empty allow array rejected (2) | `malformed_or_missing_inputs_guard_rejects_with_no_output` | the same table's permissions-guard cases |
| no arguments rejected (1) | `no_arguments_guard_rejects_on_both_sides` | asserted against both engines |
| hooks reduced to the safety guards only (1) | `hooks_reduced_to_safety_guards_only_functional_hooks_dropped` | the SAFETY_REGEXP filter |

Adds, with no old counterpart:

| New test | What it adds |
|---|---|
| `non_ascii_value_diverges_from_python_named_direction` | The ensure_ascii divergence, asserted in a named direction rather than left as a comment |
| `regression_pin_rust_matches_python_oracle_and_mutation_diverges` | Byte-match against the python oracle, plus a mutation that must produce a diff |
| `settings_gen_works_from_the_cli` | The subcommand end to end, not just the library function |
| `malformed_hooks_shape_fails_in_both_engines_and_writes_no_stdout` | A malformed .hooks shape must fail loudly, not emit a hooks-less seed |

So 10 old scenarios map onto 6 Rust tests and 4 more are new, which is the whole
10. Two of those four exist because of defects found during the port itself.

## Per-scenario rows: the two `src/` unit-test suites

**COMPLETE for both. 25 + 7 = 32 old scenarios accounted for, zero blank rows.**
These are the last two, so with them the mapping is finished and WU-14's
acceptance rule is satisfied for every suite it deletes.

Unlike the other thirteen, these map to unit tests inside `src/` rather than to a
file under `tests/`. `cargo test --lib` reports 50: 46 live in `src/common/` and
4 elsewhere, `src/hooks/mod.rs` (1, malformed-stdin survival) and `src/lib.rs`
(3, the CLI surface), which belong to WU-0 and WU-1 and have no shell ancestor.

### `hooks/lib/common.test.sh` (25)

| Old scenarios | New home | How it is covered |
|---|---|---|
| field: string value / nested path / missing key returns empty / boolean true / integer number / object returns compact JSON (6) | `src/common/payload.rs` | six tests, one per label: `field_string_value`, `field_nested_path`, `field_missing_key_returns_empty_string`, `field_boolean_true`, `field_integer_number`, `field_object_returns_compact_json` |
| session_id: extracts / empty when missing (2) | `src/common/session.rs` | `session_id_extracts_value`, `session_id_empty_when_missing` |
| session_dir: empty when no session_id / returns expected path / created on demand (3) | `src/common/session.rs` | `session_dir_in_empty_when_no_session_id` plus `session_dir_in_creates_and_returns_expected_path`, which asserts both the path and the on-demand creation |
| abspath: empty input / directory resolves realpath / non-existent resolves parent plus basename (3) | `src/common/session.rs` | `abspath_empty_input_returns_empty`, `abspath_directory_resolves_realpath`, `abspath_nonexistent_file_resolves_parent_and_basename` |
| atomic_append: two lines written / first line content (2) | `src/common/atomic.rs` | `appends_two_lines_in_order` asserts order and content together |
| emit_pre_context / emit_pre_deny / emit_prompt_context / emit_system_message exact JSON (4) | `src/common/emit.rs` | the four `*_matches_shell` tests, each diffed against the shell original |
| emit_system_message: non-ASCII stays raw UTF-8 (1) | `src/common/emit.rs` | `emit_system_message_non_ascii_matches_shell` |
| incr_counter: missing file starts at 1 / second call returns 2 / lock directory removed (3) | `src/common/counter.rs` | `missing_file_starts_at_one`, `second_call_returns_two`, `lock_directory_removed_after_call` |
| repo_slug: returns owner/repo slug (1) | `src/common/repo.rs` | `repo_slug_returns_owner_repo_format_in_this_checkout` |

### `hooks/incr-counter.test.sh` (7)

| Old scenarios | New home | How it is covered |
|---|---|---|
| missing-file: file content / _INCR_RESULT (2) | `src/common/counter.rs` | `missing_file_starts_at_one` asserts both the file and the returned value |
| second-call: file content / _INCR_RESULT (2) | `src/common/counter.rs` | `second_call_returns_two` |
| pre-seeded: file content / _INCR_RESULT (2) | `src/common/counter.rs` | `pre_seeded_file_increments_from_existing_value` |
| lock-dir-absent (1) | `src/common/counter.rs` | `lock_directory_removed_after_call`. This label comes from an INLINE pass site, not a `check` call |

### Substantial new coverage in `src/common/` (22 tests, no old counterpart)

| Area | What it adds |
|---|---|
| `src/common/proc.rs` (3) | The whole module is new: subprocess timeouts, added after review found no shell-out was bounded |
| `src/common/counter.rs` concurrency and saturation (2) | `concurrent_increments_do_not_lose_a_count`, `counter_at_i64_max_saturates_instead_of_wrapping` |
| `src/common/atomic.rs` (2) | `creates_missing_parent_directories`, `concurrent_appends_do_not_lose_or_interleave_a_line` |
| `src/common/payload.rs` (5) | Insertion-order preservation, whole numbers beyond i64 max, and three parse-robustness tests including truncated JSON never panicking |
| `src/common/session.rs` (5) | Real HOME handling, leaf symlinks left unresolved, missing parent falling back unchanged |
| `src/common/emit.rs` (3) | The `decision: block` shape, an emit probe, and public emitters matching the struct builders |
| `src/common/repo.rs` (4) | URL normalisation across https, ssh shorthand, ssh scheme with trailing slash, and no-suffix |

So 32 old scenarios map onto 24 unit tests, and 22 more add coverage the shell
suites never had, which is the 46 in `src/common/`. The largest single addition,
`proc.rs`, exists because a review found that no shell-out anywhere had a
timeout while the python bounded every one.

Extraction note, the sixth convention in this file: both suites put the LABEL
FIRST, as `check "<label>" "$got" "$want"`. Taking the last quoted token returns
the expected VALUE instead. Two further traps: `common.test.sh`'s helper contains
`ok "$label"`, which a naive scan counts as a 26th case, and
`incr-counter.test.sh` has one INLINE pass site (`lock-dir-absent`) that no
`check` grep will find, so it reads as 6 against a real 7. Both were caught by
reconciling against the suites' own summary lines.

## Per-scenario rows: `hooks/bg-await-guard.test.sh`

**COMPLETE. All 19 old scenarios accounted for, zero blank rows, so WU-14 may
delete this suite.** The old suite passes `run <expect> <bg> <command>` with no
label, so the command string is the case. All 19 strings appear verbatim in
`mod bg_await_guard`, in the same order, across three table-driven tests.

### Warn: backgrounded and await-sensitive (12) into `backgrounded_await_sensitive_commands_warn`

| Old scenario | New test | How it is covered |
|---|---|---|
| `npm install` (1) | `backgrounded_await_sensitive_commands_warn` | array element 1, verbatim |
| `rm -rf node_modules package-lock.json && npm install` (1) | same | element 2, verbatim |
| `pnpm install --frozen-lockfile` (1) | same | element 3, verbatim |
| `yarn install` (1) | same | element 4, verbatim |
| `bun install` (1) | same | element 5, verbatim |
| `npm ci` (1) | same | element 6, verbatim |
| `npm run build` (1) | same | element 7, verbatim |
| `tsc && tsc-alias` (1) | same | element 8, verbatim |
| `make build` (1) | same | element 9, verbatim |
| `cargo build --release` (1) | same | element 10, verbatim |
| `pip install -r requirements.txt` (1) | same | element 11, verbatim |
| `rm -rf node_modules` (1) | same | element 12, verbatim |

### Quiet: same commands in the foreground (3) into `the_same_commands_in_the_foreground_stay_quiet`

| Old scenario | New test | How it is covered |
|---|---|---|
| `npm install` foreground (1) | `the_same_commands_in_the_foreground_stay_quiet` | element 1, `background = false` |
| `npm run build` foreground (1) | same | element 2 |
| `rm -rf node_modules && npm install` foreground (1) | same | element 3 |

### Quiet: legitimately long-running watches (4) into `long_running_watches_are_left_alone`

| Old scenario | New test | How it is covered |
|---|---|---|
| `npm run dev` backgrounded (1) | `long_running_watches_are_left_alone` | element 1 |
| `vite --host` backgrounded (1) | same | element 2 |
| `tail -f /var/log/app.log` backgrounded (1) | same | element 3 |
| `node server.js` backgrounded (1) | same | element 4 |

Adds, with no old counterpart:

| New test | What it adds |
|---|---|
| `build_tools_only_match_at_a_command_start` | `echo tsc` and `grep make Makefile` must not trip; `a; tsc` must, so the anchor is a separator and not a blanket substring |
| `the_env_switch_disables_the_guard` | `BG_AWAIT_GUARD=0` silences it |
| `malformed_payloads_exit_silently` | Empty, `{`, `null`, no `command`, and a payload with no `run_in_background` key |

## Per-scenario rows: `hooks/no-dash-guard.test.sh`

**COMPLETE. All 17 old scenarios accounted for, zero blank rows, so WU-14 may
delete this suite.** Note the old suite's clean-body-file pair maps into the
*blocking* Rust test rather than the allow test: `a_dash_inside_a_referenced_body_file_blocks`
deliberately carries both halves, so that "blocks whenever a `--body-file` flag
appears" cannot satisfy it.

### Block: inline em/en dash in a posting command (5) into `posting_commands_with_an_inline_dash_block`

| Old scenario | New test | How it is covered |
|---|---|---|
| `gh pr edit 42 --title "... — really"` (1) | `posting_commands_with_an_inline_dash_block` | element 1, U+2014 |
| `gh pr create --title "fix: a – b" --body "x"` (1) | same | element 2, U+2013 |
| `git commit -m "... — after invalidation"` (1) | same | element 3 |
| `git tag -a v1.0 -m "release – first"` (1) | same | element 4 |
| `gh pr comment 42 --body "nice work — ship it"` (1) | same | element 5 |

### Block: dash inside a referenced body/message file (4) into `a_dash_inside_a_referenced_body_file_blocks`

| Old scenario | New test | How it is covered |
|---|---|---|
| `gh pr create --body-file $DASH_FILE --title "clean title"` (1) | `a_dash_inside_a_referenced_body_file_blocks` | blocking loop, element 1 |
| `gh pr edit 42 --body-file=$DASH_FILE` (1) | same | element 2, the `=` form |
| `git commit -F $DASH_FILE` (1) | same | element 3 |
| `gh api -X POST ... --input $DASH_FILE` (1) | same | element 4 |

### Allow: clean posting commands (4)

| Old scenario | New test | How it is covered |
|---|---|---|
| `gh pr edit 42 --title "... - really"` (1) | `clean_posting_commands_pass` | element 1, ascii hyphen |
| `git commit -m "fix: stop stale reads after invalidation"` (1) | same | element 2 |
| `gh pr create --body-file $CLEAN_FILE --title "clean title"` (1) | `a_dash_inside_a_referenced_body_file_blocks` | the allow loop, element 1 |
| `git commit -F $CLEAN_FILE` (1) | same | the allow loop, element 2 |

### Allow: non-posting commands are never guarded (4) into `non_posting_commands_are_never_guarded`

| Old scenario | New test | How it is covered |
|---|---|---|
| `echo "a — b"` (1) | `non_posting_commands_are_never_guarded` | element 1 |
| `grep -n "–" notes.md` (1) | same | element 2 |
| `gh pr view 42` (1) | same | element 3 |
| `gh pr diff 42` (1) | same | element 4 |

Adds, with no old counterpart:

| New test | What it adds |
|---|---|
| `every_dash_in_the_family_blocks_a_posting_command` | `DASH_FAMILY` U+2012 figure and U+2015 horizontal bar. The shell suite only ever exercised en and em, so the outer two were unproven |
| `a_missing_body_file_does_not_block` | An unreadable path must not become a false deny |
| `the_env_switch_disables_the_guard` | `NO_DASH_GUARD=0` silences it |
| `malformed_and_empty_payloads_exit_silently` | Empty, `{`, `null`, no `command`, and `"tool_input":null` |

## Per-scenario rows: `hooks/precommit-check.test.sh`

**COMPLETE. All 12 old scenarios accounted for, zero blank rows, so WU-14 may
delete this suite.** The old suite's `run` helper fails any case that emits a
`permissionDecision: deny`; the Rust `warns_in` helper asserts the same
never-block property on every call, so that guarantee is preserved per-case
rather than in a single dedicated test.

| Old scenario (verbatim label) | New test | How it is covered |
|---|---|---|
| `git log is not a commit` (1) | `only_a_real_commit_is_inspected` | loop element 1 |
| `git status is not a commit` (1) | same | element 2 |
| `commit --help is not a commit` (1) | same | element 3 |
| `clean small staged diff` (1) | same | the trailing direct assert, staged `a.txt` |
| `nothing staged` (1) | `nothing_staged_is_quiet` | direct, untracked `b.txt` only |
| `staged .env` (1) | `secret_shaped_filenames_warn` | table row `("env", ".env", true)`, `add -f` |
| `staged .pem` (1) | same | table row `("pem", "server.pem", false)` |
| `console.log added` (1) | `debug_leftovers_in_added_lines_warn` | table row `("js", "app.js", ...)` |
| `breakpoint added` (1) | same | table row `("py", "app.py", ...)` |
| `over 600 changed lines` (1) | `oversized_commits_warn_by_lines_and_by_files` | first half, 700-line `big.txt` |
| `over 20 files` (1) | same | second half, 25 files |
| `PRECOMMIT_CHECK=0 should silence the guard` (1) | `the_env_switch_disables_the_guard` | direct, staged `.env` plus `PRECOMMIT_CHECK=0` |

Adds, with no old counterpart:

| New test | What it adds |
|---|---|
| `secret_shaped_filenames_warn` row `("key", "id_rsa", false)` | A third secret shape the shell suite never tried |
| `outside_a_git_repo_it_is_quiet` | No repo means no diff; the guard must not surface git's error |

## Per-scenario rows: `hooks/rm-workspace-guard.test.sh`

**COMPLETE. All 48 old scenarios accounted for, zero blank rows, so WU-14 may
delete this suite.** It got there in two steps on 2026-08-22: the mapping found
21 of 30 covered and 9 blank, and the 9 were closed by writing tests, not by
adjusting the table. This is the guard that blocks `rm` outside the workspace,
so the gap was the highest-consequence one in this file, wider in kind than the
`rebuild-memory-graph` count gap the top of this file calls the highest risk.

**Root cause of all 9 blanks: the Rust helper always set the variable.**
`blocked(command, roots)` set `PLAYBOOK_SAFE_ROOTS` on every single invocation,
and grepping the whole Rust tree for that name returned exactly that one line
under `tests/`, while `src/hooks/rm_workspace_guard.rs` carries no
`#[cfg(test)]` block of its own. So **no Rust test ever ran this guard with the
variable unset or empty**, leaving `safe_roots()` at `:76` and `git_repo_root()`
at `:89`, the entire zero-config fallback chain, executed only in production.

The shell suite covered that chain deliberately. Its comment at `:63-66` calls
those cases "the regression proof that the zero-config default is no weaker than
the old hardcoded `~/Workspace`, i.e. it must NOT default to `$HOME` (that would
unblock `~/.ssh` and `~/.aws`)". Deleting the suite before 2026-08-22 would have
dropped that proof.

**The fix was verified by mutation, not by a green run**, since a test that
cannot fail is what produced this gap in the first place:

| Mutation applied to `src/hooks/rm_workspace_guard.rs` | Result |
|---|---|
| `safe_roots()` falls back to `$HOME` instead of repo-root-then-cwd, the exact regression the shell comment names | **3 new tests fail**, no old test notices |
| The deny message's root list replaced with a hardcoded `~/Workspace/**` | **1 new test fails**, no old test notices |

Both mutations pass all 30 pre-existing tests. That is the measured proof the
old suite could not have caught either regression.

### Covered by the original port (21)

| Old scenario | New test | How it is covered |
|---|---|---|
| allow `rm -rf $HOME/Workspace/proj/build` (1) | `targets_inside_a_safe_root_are_allowed` | element 1 |
| allow `rm $HOME/Workspace/a.txt` (1) | same | element 2 |
| allow `rm -rf ~/Workspace/proj/node_modules` (1) | same | element 3, tilde form |
| allow `rm -rf $HOME/.claude/cache/x` (1) | same | element 4, `~/.claude` always allowed |
| allow `rm -rf $HOME/Workspace/proj/sub/../build` (1) | same | element 5, `..` collapsing back inside |
| block `rm -rf /etc/passwd` (1) | `targets_outside_every_safe_root_are_blocked` | element 1 |
| block `rm $HOME/secrets.txt` (1) | same | element 2 |
| block `rm -rf /` (1) | same | element 3 |
| block `rm -rf ~/Workspace/../.ssh` (1) | `dot_dot_traversal_out_of_a_safe_root_is_blocked` | element 1 |
| block `rm -rf $HOME/Workspace/../../../etc/passwd` (1) | same | element 2 |
| block `rm -rf $HOME/.claude/../.aws/credentials` (1) | same | element 3, escaping the always-on root |
| block traversal escaping a configured root, case 9 (1) | same | element 1 is the identical shape, `<root>/../.ssh` |
| block `cd /tmp && rm -rf foo` (1) | `unresolvable_commands_are_blocked_conservatively` | first assert |
| block `rm -rf $(echo /etc)` (1) | same | second assert, substitution |
| block `rm` on line 2 after a newline (1) | `multiline_and_tab_separated_commands_are_still_inspected` | first assert |
| block tab-separated `rm` (1) | same | second assert |
| allow multi-line with no `rm` on any line (1) | same | third assert |
| allow `rm` on line 2 targeting an allowed root (1) | same | fourth assert |
| allow target in the second of two colon-separated roots, case 4 (1) | `multiple_roots_are_all_honoured` | `$h/b/file` allowed against `$h/a:$h/b` |
| allow trailing-slash root, case 6 (1) | `a_trailing_slash_on_a_root_still_matches` | direct |
| block with a nonexistent root, case 8 (1) | `a_nonexistent_root_blocks_everything_outside_it` | direct |

### Closed 2026-08-22 by 6 new tests (9)

These 9 rows were blank when the mapping was written. They are closed by tests
added the same day, listed here with the same evidence standard as every other
row above.

| Old scenario | New test | How it is covered |
|---|---|---|
| block heredoc body quoting a deletion (1) | `a_heredoc_body_quoting_a_deletion_is_an_accepted_false_positive` | direct, and the doc comment carries the old suite's `:97-102` rationale so the trade-off stays deliberate |
| allow `$repo_dir/sub/file`, unset, cwd inside a git repo, case 1 (1) | `an_unset_variable_defaults_to_the_git_repo_root` | first assert, `git init` in a scratch repo, exercises `git_repo_root()` |
| block `$repo_sibling/file`, unset, case 2 (1) | same | second assert, the negative half |
| allow `$plain_dir/file`, unset, cwd NOT in a git repo, case 3 (1) | `an_unset_variable_outside_a_git_repo_defaults_to_the_cwd` | first assert, the `current_dir()` leg |
| block `$plain_sibling/file`, unset, case 3 (1) | same | second assert, the negative half |
| block outside with `PLAYBOOK_SAFE_ROOTS=""`, case 5 (1) | `an_empty_variable_behaves_like_unset` | first assert; the second pins that it falls back rather than blocking everything, which a block-all bug would otherwise satisfy |
| allow `$rel_base/relroot/file` with a relative root, case 7 (1) | `a_relative_root_resolves_against_the_guards_cwd` | first assert |
| block outside with that same relative root, case 7 (1) | same | second assert, proving no widening into a false allow |
| deny reason names the roots in effect, case 10 (1) | `the_deny_reason_names_the_roots_in_effect` | parses `hookSpecificOutput.permissionDecisionReason` and asserts both configured roots appear; the only content assertion on the message |

### Added to both suites 2026-08-23: unexpanded variables fail closed (5)

Not a port, and the only entry in this file that fixed a **fail-open** rather
than tightening an existing block. `canon` treats a leading `$` as a relative
path and joins it to the cwd, so `rm -rf "$HOME/.cache/x"` resolved to
`<repo>/$HOME/.cache/x`, landed inside a safe root and was ALLOWED, while the
shell expanded it to a real path outside the workspace. Found by noticing that
one of this session's own cleanup commands succeeded when it should not have.

| Old scenario | New test | How it is covered |
|---|---|---|
| block `rm -rf "$HOME/.cache/x"` (1) | `an_unexpanded_variable_target_is_unresolvable_and_blocks` | element 1, quoted |
| block `rm -rf $HOME/.cache/x` (1) | same | element 2, unquoted |
| block `rm -rf "${BUILD_DIR}/out"` (1) | same | element 3, brace form |
| block ``rm -rf `echo /etc`/passwd`` (1) | same | element 4, backtick |
| block `rm -rf "$REPO/target"` (1) | same | element 5, the accepted cost: a variable pointing INSIDE the workspace blocks too |

`literal_targets_are_unaffected_by_the_expansion_rule` has no old counterpart. It
pins that the rule did not become a blanket block, which a rule this broad could
otherwise satisfy while breaking every ordinary deletion.

**Mutation-verified:** dropping the `$`/backtick condition, which restores the
fail-open, fails `an_unexpanded_variable_target_is_unresolvable_and_blocks` and
nothing else.

### Added to both suites 2026-08-22: the temp exemption (6)

Not a port. These scenarios were written into the shell suite and the Rust suite
in the same change, so the pair stays at parity right up to the deletion. Both
implementations were verified against each other on 11 commands, including
`~/.ssh/id_rsa`, with zero disagreements.

| Old scenario | New test | How it is covered |
|---|---|---|
| allow `/tmp/scratch-file` (1) | `paths_inside_temp_are_allowed_but_the_temp_root_is_not` | allow loop, element 1 |
| allow `/private/tmp/scratch-file` (1) | same | allow loop, element 2, the macOS resolved spelling |
| block `/tmp` itself (1) | same | block loop, element 1 |
| block `/private/tmp` itself (1) | same | block loop, element 2 |
| block `/tmp/../etc/passwd` (1) | `the_temp_exemption_does_not_leak_through_traversal_or_prefixes` | first assert, `canon` collapses the traversal first |
| block `/tmpfoo/file` (1) | same | second assert, component-wise prefix match |

### Added to both suites 2026-08-22: quoted tokens are prose (7)

Also not a port. A `rm` inside a quoted region now counts as a command only when
a separator puts it in command position.

| Old scenario | New test | How it is covered |
|---|---|---|
| allow `git commit -m "... rm on /etc/passwd"` (1) | `a_quoted_mention_is_prose_not_a_command` | element 1, double quotes |
| allow `git commit -m '... rm on /etc/passwd ...'` (1) | same | element 2, single quotes |
| allow a quoted mention plus a `$(...)` elsewhere (1) | same | element 3, the exact shape that blocked an agent mid-commit |
| block `sudo rm -rf /etc/passwd` (1) | `quoting_does_not_hide_a_real_deletion` | element 1, unquoted is unchanged |
| block `xargs rm -rf /etc/passwd` (1) | same | element 2 |
| block `echo hi && rm -rf /etc/passwd` (1) | same | element 3 |
| block `sh -c "cd /x && rm -rf /etc/passwd"` (1) | same | element 4, quoted but in command position |

**Both directions are mutation-verified**, which matters here because the first
attempt at the prose test was vacuous: a message whose other words are relative
paths canonicalises inside the safe root, so the old code allowed it too and the
test passed against the very behaviour it was meant to reject. Each case now
carries an absolute path or a substitution.

| Mutation | Result |
|---|---|
| `is_command = true`, the old behaviour | `a_quoted_mention_is_prose_not_a_command` fails; shell suite fails 3 |
| `is_command = !quoted`, quoting as a blanket escape hatch | `quoting_does_not_hide_a_real_deletion` fails |

The 6 zero-config tests rest on two helper changes, both of which are the actual
fix rather than a restatement of the old suite:

- `run_guard_in(command, roots, cwd)` takes `roots: Option<&str>` and, on `None`,
  calls `env_remove` rather than setting an empty value. Removing it matters:
  the variable may be set in the environment the suite inherits, and then the
  zero-config branch would never run even in a test written to target it.
- `real_dir(tag)` canonicalises each scratch dir. macOS temp dirs sit under
  `/var`, a symlink to `/private/var`, while `git rev-parse --show-toplevel` and
  `getcwd` both report the resolved form, and the guard's `canon()` is lexical by
  design. Without this the fixtures would fail on path form, not guard logic.
  This is the same trap the shell suite documented at its `:110-113`.

## Deletion pre-flight: what still EXECUTES these files

The rows above answer "is the coverage preserved". They do not answer "does
anything still call this", and that is the other half of WU-14's risk. A stale
reference is exactly the shape that produced roughly 110 silent errors over 28
hours on 2026-08-11 (`hook-rename-lockstep-settings`).

Measured 2026-08-18 by separating references that EXECUTE a script from those
that merely name it in a comment. Grep alone conflates the two: of the 15 python
files, every one has references, but most `src/**.rs` hits are doc comments citing
the python as the port's specification and are harmless.

### Rust that runs python as a test oracle (6 files)

Deleting the script breaks `cargo test`, so these must change in the SAME unit
that deletes it.

| File | Executes |
|---|---|
| `tests/hooks_graph_reader.rs` | `memory-anchors.py`, `rebuild-memory-graph.py` |
| `tests/hooks_graph_writer.rs` | `rebuild-memory-graph.py` |
| `tests/hooks_preread.rs` | `preread-edit-check.py`, `preread-size-check.py` |
| `tests/init_merge.rs` | `merge-settings.py` |
| `tests/settings_gen.rs` | `gen-shared-settings.py` |
| `src/common/emit.rs` | `memory-capture.py`, in a `#[cfg(test)]` oracle |

### Runtime, installer and CI (5 files)

| File | Why it matters |
|---|---|
| `hooks/hooks.json` | Routes 11 functional hooks at their `.py` files. **Runtime.** Deleting the scripts without retiring this registry breaks every hook for every user |
| `shell/setup-local.sh:105` | Assigns `MERGE_BIN="$SELF_ROOT/shell/merge-settings.py"` and runs it during install. **Installer.** WU-11 replaces this path |
| `.github/workflows/shell-ci.yml:88` | Runs `check-shared-settings.py`. WU-21 removes this step |
| `Makefile:20` | Runs `$(GEN)`, which is `gen-shared-settings.py`. WU-20 repoints it at the binary |
| `shell/plugin-e2e.sh:128` | Runs `check-shared-settings.py`. WU-21 removes the check from this harness |

### Confirmed harmless

`shell/memory-context.sh:18`, `shell/ensure-deps.sh:17` and `statusline.sh:332`
each name a python file in a COMMENT only. `tests/hooks_session.rs` likewise
mentions `session-clean-exit.py` without executing it. `ruff.toml` names two hooks
in a config comment and becomes orphaned once no `.py` remains.

**Method note.** A first pass grepped for `python3 <file>.py` and missed
variable indirection: `setup-local.sh` assigns the path to
`MERGE_BIN` first and runs it later, so the exec scan reported it clean. It was
caught by going back through every file the plain-grep inventory listed and
checking each one by hand. When a scan and a broader grep disagree, the scan is
usually the one that is wrong.

## How to fill a per-scenario row

One row per assertion in the old suite:

| Old case (verbatim label from the suite) | New test | How it is covered |
|---|---|---|
| e.g. `s15: contested key freezes old base value` | `tests/init_merge.rs::mandatory_and_ported_fixtures_rust_and_python_mergers_agree` | `FIXTURES[1]`, plus a direct `NEWBASE_OUT["k"]` assertion |

Name the specific fixture or table index when a test is table-driven. "Covered
by the differential test" is not a row; it is the thing the row has to prove.
