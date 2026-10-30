# ADR-0056 Quality Report

- **Record:** `docs/adr/0056-share-one-code-index-across-worktrees.md`
- **Blueprint:** `docs/adr/0056-share-one-code-index-across-worktrees-blueprint.md`
- **Date:** 2026-10-30
- **Result:** The record is Accepted. The gate ran 25 rounds, well past its cap of 5, on the maintainer's instruction to repeat it until nothing blocking was left. The last ten critic rounds returned PASS. The last test review returned WARN.

## Gate checklist

| Phase | Latest verdict | Round | Note |
|---|---|---|---|
| Fact-check | PASS | 25 | Run inline: paths, line numbers, code claims, prose check, graph. |
| Adversarial review | PASS | 25 | Base critic at medium effort from round 8. Earlier rounds used the xhigh variant. |
| Test review | WARN | 25 | One timing-class item (the 100 ms cleanup gate) and two wording notes. All three were folded in after the gate check. |

## What the rounds changed

- The design moved from a content store keyed by sha256 to one store per repo with a manifest table per worktree, scoped TEMP views for the queries, and a lock protocol with a shared lock that recovery briefly upgrades.
- Writes use `BEGIN IMMEDIATE` through one helper, with a retry of the whole batch on a busy database.
- Adoption of stored content is one `INSERT ... SELECT ... ON CONFLICT` statement. Cleanup runs only at the end of `code index`, in chunks, behind a read-only check.
- The gates M1 (sharing potential), M2 (speed) and M3 (speed and disk on 52 worktrees) decide whether the work continues. Alternative B is the fallback.

## Known gaps

- The wording-only edits after the gate check (the last WARN notes and the status change) were not reviewed again.
- Planner behavior of `bm25` through the scoped views, and the join cost, are measured by M2 and M3 and not before.
- WU-0 Part B may pass 1,500 lines. The blueprint names the split.
