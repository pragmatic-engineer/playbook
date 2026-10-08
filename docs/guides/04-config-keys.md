# Config keys

Playbook reads 14 config keys. Run `playbook config list` to see the value each one resolves to in the current repo, and `playbook config get <key>` for one.

## Where a key is read from

A key can be set at three tiers. The first tier that sets it wins:

1. Repo: `playbook config set <key> <value>`
2. Org (every repo under one owner): `playbook config set --org <key> <value>`
3. Global (your own, for every repo): `playbook config set --global <key> <value>`
4. The default in the table below, when no tier sets it.

The command rejects an unknown key, a value of the wrong type, and a number outside its range.

## Reference

| Key | Default | Values | What it does |
|---|---|---|---|
| `autoReview.enabled` | `true` | `true`, `false` | `playbook:finish-pull-request` reviews the PR before it goes ready. |
| `autoReview.type` | `auto` | `quick`, `deep`, `auto` | Which review the self-review runs. `auto` picks `quick` only when the triage agrees three times, and `deep` otherwise. |
| `autoReview.fix` | `false` | `true`, `false` | Fixes every finding the self-review confirms, pushes the fixes, and re-runs the scoped checks. |
| `autoMerge.enabled` | `false` | `true`, `false` | After the review and fixes, marks the PR ready, waits for green checks, then merges with `gh pr merge --auto`. |
| `commit.signOff` | `true` | `true`, `false` | Commits made through `/playbook:commit-and-push` carry a `Signed-off-by` trailer. |
| `pr.draft` | `true` | `true`, `false` | `/playbook:create-pull-request` opens the PR as a draft. |
| `worktreeCleanup.enabled` | `true` | `true`, `false` | Lets the worktree sweep remove worktrees whose work has landed. Turn it off to keep every worktree. |
| `worktreeCleanup.staleAfterDays` | `30` | whole number, 0 or more | Days since the last commit of an agent or `cc worktree` branch after which the sweep treats it as landed, even when it was never merged. |
| `worktreeCleanup.conflictGracePeriodDays` | `90` | whole number, 0 or more | Days a `/playbook:implement` worktree stopped on a cherry-pick conflict is kept, counted from the timestamp in its stop marker, before the sweep treats it as landed. |
| `mode` | `ask` | `ask`, `auto` | `ask` stops for every decision. `auto` takes the recommended answer, lists it under Assumptions, and keeps going. |
| `auto.budgetUsd` | `5` | number, 0.01 or more | The session spend cap in US dollars while the mode is `auto`. |
| `auto.warnPct` | `70` | whole number, 1 to 100 | The percent of `auto.budgetUsd` at which the spend hook warns once. |
| `fix.maxFiles` | `3` | whole number, 1 or more | `/playbook:fix` stops and hands off to `/playbook:plan` when more files than this change. |
| `fix.maxLines` | `500` | whole number, 1 or more | `/playbook:fix` stops the same way when more lines than this change, not counting the new test. |

## See also

- [Review and PR flow](02-review-and-pr-flow.md#pr-and-commit-settings): how the PR keys behave in detail.
- [Auto mode](../../README.md#auto-mode): what `mode`, `auto.budgetUsd` and `auto.warnPct` turn on.
- [Plan and implement](01-plan-and-implement.md): the `/playbook:fix` limits.
- [Worktree engine](../internals/03-worktree.md): the sweep that the `worktreeCleanup.*` keys control.
