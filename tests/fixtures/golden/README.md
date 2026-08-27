# Golden fixtures: frozen oracles

**Not only python.** `common-sh.emitters.json` freezes a SHELL oracle,
`hooks/lib/common.sh`, for the same reason and by the same rule. WU-14d deletes
that file, and five tests in `src/common/emit.rs` sourced it live to prove the
Rust emitters matched byte for byte. Nobody had catalogued it as an oracle
because it is shell rather than python, so the deletion would have broken those
tests rather than silently weakening them. With the two fixtures below,
`cargo test` needs neither bash nor python3 on the machine.

Each file here is the exact output of an implementation that ADR 0007 replaced
with a Rust port, captured while that original still existed and committed so
the cross-implementation check survives its deletion. Most are python hooks;
`common-sh.emitters.json` is the shell library described above.

Before this, several tests were differential: they ran the original and
asserted the Rust port matched. That is the strongest evidence a port is
faithful, and it is exactly what the blueprint asked for, because a ported
suite passes happily against an empty stub. WU-14 deletes the python, which
would have removed the oracle and quietly downgraded those tests to
"Rust agrees with itself".

Freezing the output keeps the check: a future Rust change that drifts from what
python produced still fails, on a machine with no python installed.

The `init-merge.*.json` fixtures each bundle three python outputs for one
merge, `stdout`, `newbase` and `skip`, as string fields of one JSON object,
since `shell/merge-settings.py` writes to three separate places for a single
input triple. The test reads each field back as a plain string, so the
byte-for-byte comparison is unaffected by the wrapping.

`init-merge.n3-zero-withheld-keys.json`, `init-merge.n4-missing-base.json`,
`init-merge.n4-invalid-base.json` and `init-merge.c2-coincidence.json` follow
the same bundling idea, but each only carries the fields its test actually
asserts on (`exit_code`, and a subset of `stdout`, `stderr` or `skip`), rather
than the full `stdout`/`newbase`/`skip` triple, since these four tests never
compared all three. The two `n4` fixtures' `stderr` field has its BASE file's
absolute path normalised to the literal `<base-path>`, since python's N4
warning text embeds that path and it otherwise varies with the capturing
machine's temp directory; the assertions against it only check for the
substring "warning", so the normalisation changes nothing the tests verify.

| Fixture | Produced by | Input | Captured |
|---|---|---|---|
| `rebuild-memory-graph.scalar-fact.json` | `hooks/rebuild-memory-graph.py` | `populate_fixture_tree()` then rebuild for `scalar-fact.md`, canonicalised by `canonical_graph()` | 2026-08-21, at v0.11.0 |
| `preread-size-check.deny.txt` | `hooks/preread-size-check.py` | a 1500-line numbered fixture file, read through `preread-size-check`'s deny path | 2026-08-21, at v0.11.0 |
| `memory-anchors.src-a.txt` | `hooks/memory-anchors.py` | `BASE_GRAPH` fixture graph, edit of `src/a.py`, session `cross1` | 2026-08-21, at v0.11.0 |
| `common-sh.emitters.json` | `hooks/lib/common.sh` | the five `emit_*` helpers, one call each, bundled by emitter name | 2026-08-23, at v0.11.0 |
| `memory-capture.block.txt` | a python one-liner mirroring `hooks/memory-capture.py`'s block shape | `json.dumps` with `separators=(',',':')` and `ensure_ascii=False` | 2026-08-23, at v0.11.0 |
| `init-merge.user-key-absent-from-base.json` | `shell/merge-settings.py` | the `FIXTURES` "user key absent from base" base/template/user triple | 2026-08-21, at v0.11.0 |
| `init-merge.user-key-modified-from-base.json` | `shell/merge-settings.py` | the `FIXTURES` "user key modified from base" triple | 2026-08-21, at v0.11.0 |
| `init-merge.template-key-removed.json` | `shell/merge-settings.py` | the `FIXTURES` "template key removed" triple | 2026-08-21, at v0.11.0 |
| `init-merge.user-key-nested-three-deep-differs-from-base.json` | `shell/merge-settings.py` | the `FIXTURES` "user key nested three deep differs from base" triple | 2026-08-21, at v0.11.0 |
| `init-merge.user-hand-added-hook-entry-template-does-not-know-about.json` | `shell/merge-settings.py` | the `FIXTURES` "user hand-added hook entry" clobber-risk triple | 2026-08-21, at v0.11.0 |
| `init-merge.s1--user-unchanged-key-gets-the-template-update.json` | `shell/merge-settings.py` | the `FIXTURES` "s1" triple | 2026-08-21, at v0.11.0 |
| `init-merge.s3--new-template-key-added-when-absent-from-user.json` | `shell/merge-settings.py` | the `FIXTURES` "s3" triple | 2026-08-21, at v0.11.0 |
| `init-merge.s12--user-is-an-empty-object--output-equals-template.json` | `shell/merge-settings.py` | the `FIXTURES` "s12" triple | 2026-08-21, at v0.11.0 |
| `init-merge.s14--type-mismatch-on-a-contested-key-keeps-user-s-whole-value.json` | `shell/merge-settings.py` | the `FIXTURES` "s14" triple | 2026-08-21, at v0.11.0 |
| `init-merge.n3-zero-withheld-keys.json` | `shell/merge-settings.py` | `n3_zero_withheld_keys_writes_empty_skip_array`'s base/template/user triple (s19) | 2026-08-21, at v0.11.0 |
| `init-merge.n4-missing-base.json` | `shell/merge-settings.py` | `n4_missing_base_becomes_empty_object_with_warning_not_hard_fail`'s template/user pair over an absent BASE path (s5) | 2026-08-21, at v0.11.0 |
| `init-merge.n4-invalid-base.json` | `shell/merge-settings.py` | `n4_invalid_base_becomes_empty_object_with_warning_not_hard_fail`'s base/template/user triple, BASE not valid JSON (s13) | 2026-08-21, at v0.11.0 |
| `init-merge.c2-coincidence.json` | `shell/merge-settings.py` | `c2_coincidence_keeps_user_value_frozen_through_a_matching_template_cycle`'s two-cycle base/template/user sequence (s16) | 2026-08-21, at v0.11.0 |
| `gen-shared-settings.src-full.json` | `shell/gen-shared-settings.py` | `SRC_FULL` and `CANNED_PERMS` | 2026-08-21, at v0.11.0 |
| `gen-shared-settings.non-ascii.json` | `shell/gen-shared-settings.py` | `{"customUnknownKey":"café ☃"}` and `CANNED_PERMS` | 2026-08-21, at v0.11.0 |
| `gen-shared-settings.model-absent.json` | `shell/gen-shared-settings.py` | `SRC_NOMODEL` and `CANNED_PERMS` | 2026-08-21, at v0.11.0 |
| `gen-shared-settings.model-present.json` | `shell/gen-shared-settings.py` | `SRC_OPUS` and `CANNED_PERMS` | 2026-08-21, at v0.11.0 |
| `gen-shared-settings.hooks-filter.json` | `shell/gen-shared-settings.py` | `SRC_HOOKS` and `CANNED_PERMS` | 2026-08-21, at v0.11.0 |
| `setup-local.clean-install.json` | today's `shell/setup-local.sh` (its Step 2 python merge, before the `playbook init` cutover) | a fresh install: empty `CLAUDE_HOME`, no pre-existing `settings.json` | 2026-08-26, at v0.12.0 |
| `setup-local.skip-triggering.json` | today's `shell/setup-local.sh`, same as above | an existing `settings.json` seeded with a `cleanupPeriodDays` collision against the template | 2026-08-26, at v0.12.0 |

**Do not regenerate a golden to make a failing test pass.** The whole point is
that it does not move. If Rust output legitimately changes, that is a
behaviour change: say so explicitly and record why the divergence from the
original implementation is intended.
