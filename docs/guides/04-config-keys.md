# Config keys

Run `playbook config list` to see every key with its value and source tier in the current repo, and `playbook config get <key>` for one.

## Where a key is read from

The first tier that sets a key wins:

1. Repo: `playbook config set <key> <value>`
2. Org (every repo under one owner): `playbook config set --org <key> <value>`
3. Global (yours, for every repo): `playbook config set --global <key> <value>`
4. The default in the table below.

The command rejects an unknown key, a value of the wrong type, and a number out of range.

## Where the values are stored

All tiers live in one SQLite database, `~/.config/playbook/playbook.db`, not in files you edit by hand. Change values with `playbook config set`. Use `playbook config export` to print every stored value as JSON, and `playbook config import <file>` to load such a document back (it checks every key and value first, and stores nothing if one is invalid).

Older versions kept `config.json` files. The first run imports them and renames each to `config.json.migrated`. `playbook init` and `playbook config` adopt a later `config.json` when its values agree with the stored ones. When a value differs, or the file is not valid JSON, the file stays and `playbook config` warns with its path. Auto mode blocks the model from running `playbook config import` and from touching the database. See the [state store](../internals/08-state-store.md).

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
| `worktreeCleanup.staleAfterDays` | `30` | whole number, 0 or more | Days since the last commit of an agent or `ccc worktree` branch after which the sweep treats it as landed, even when it was never merged. |
| `worktreeCleanup.conflictGracePeriodDays` | `90` | whole number, 0 or more | Days a `/playbook:implement` worktree stopped on a cherry-pick conflict is kept, counted from the timestamp in its stop marker, before the sweep treats it as landed. |
| `mode` | `ask` | `ask`, `auto` | `ask` stops for every decision. `auto` takes the recommended answer, lists it under Assumptions, and keeps going. |
| `maxEffortLevel` | `auto` | `auto`, `low`, `medium`, `high`, `xhigh`, `max` | Playbook's own ceiling on effort, named like Claude Code's key. `auto` sets no ceiling, so Claude Code's `maxEffortLevel` decides, and each command, skill and agent keeps the effort it ships with. The lower of this and Claude Code's `maxEffortLevel` applies, so Claude Code always has the last word and playbook never goes above it. Playbook only reads `maxEffortLevel` and never changes it. Sessions started with `ccc` or `ccd` get the ceiling. Set it with `playbook effort <level>` or `playbook config set --global maxEffortLevel <level>`. Only the global tier counts. |
| `effort.agents.<name>`, `effort.commands.<name>`, `effort.skills.<name>` | `auto` | `auto`, `low`, `medium`, `high`, `xhigh`, `max` | A ceiling for one component, for example `effort.agents.fact-checker`. The lowest of Claude Code's `maxEffortLevel`, playbook's `maxEffortLevel` and this key applies, and nothing is ever raised above the effort the component ships with. `auto` and `max` add no limit. A name that matches no plugin file is accepted and flagged by `playbook effort list`. Only the global tier counts. See [ADR-0017](../adr/0017-per-component-effort-ceilings.md). |
| `agents.variants` | `auto` | `auto`, `all`, `off` | Which effort variants of the agents a session gets when you start it with `ccc` or `ccd`. `auto` gives the cheaper tiers to every agent, plus `xhigh` to `critic`, `implementer` and `reviewer`. `all` adds every other tier, including `max`. `off` gives none, so only the base agents exist. A variant above any effort ceiling in play is never passed. Sessions started without the launcher get base agents only. |
| `memory.source` | `both` | `both`, `playbook` | Which memory a session uses. `both` leaves Claude Code's own auto memory on, next to playbook memory. `playbook` turns Claude Code's auto memory off for sessions started with `ccc` or `ccd`, by setting `CLAUDE_CODE_DISABLE_AUTO_MEMORY=1` in that session only. Playbook never edits Claude Code's settings or memory files. A session started without the launcher keeps Claude Code's own behavior. Set it with `playbook config set --global memory.source <value>`. |
| `models.haiku`, `models.sonnet`, `models.opus` | empty | empty, or `claude-<tier>-<major>[-<minor>]` for that tier | Points a tier alias at another model. Empty keeps the 5.5 default. `ccc` and `ccd` export `ANTHROPIC_DEFAULT_<ALIAS>_MODEL` for each override. See [Model tiers](../internals/02-model-routing-and-memory.md#model-tiers-and-the-55-rule). |
| `routing.escalate` | `ask` | `ask`, `auto`, `deny` | What `playbook route` does when a task needs approval (the top model, the high implement tier, or a task that failed twice). `ask` tells the orchestrator to ask you first. `auto` proceeds, and auto mode with `auto.budgetUsd` still caps the spend. `deny` returns the cheaper route instead. Your effort ceiling always wins. |
| `auto.budgetUsd` | `5` | number, 0.01 or more | The session spend cap in US dollars while the mode is `auto`. |
| `auto.warnPct` | `70` | whole number, 1 to 100 | The percent of `auto.budgetUsd` at which the spend hook warns once. |
| `fix.maxFiles` | `3` | whole number, 1 or more | `/playbook:fix` stops and hands off to `/playbook:plan` when more files than this change. |
| `fix.maxLines` | `500` | whole number, 1 or more | `/playbook:fix` stops the same way when more lines than this change, not counting the new test. |
| `review.stackMaxPrs` | `6` | whole number, 1 or more | The quick and deep review commands ask again before reviewing a whole PR stack with more open PRs than this. |
| `review.stackMaxLines` | `3000` | whole number, 1 or more | They ask again when the open PRs of the stack change more lines than this. |
| `security.defaults` | `false` | `true`, `false` | `playbook init` also merges the shipped security defaults (the `permissions` block and `DISABLE_AUTOUPDATER`) into `settings.json`. `init --security` does the same for one run and `--no-security` skips them even when this is true. Off, `init` leaves the permissions you already have as they are. See [Security](../../README.md#security). |
| `usage.theme` | `btop` | `btop`, `dark`, `light`, `mono` | The color theme `playbook usage` starts with. `t` cycles the themes inside the view. `NO_COLOR` forces `mono`. |

## See also

- [Review and PR flow](02-review-and-pr-flow.md#pr-and-commit-settings): how the PR keys behave in detail.
- [Auto mode](05-auto-mode.md): what `mode`, `auto.budgetUsd` and `auto.warnPct` turn on.
- [Plan and implement](01-plan-and-implement.md): the `/playbook:fix` limits.
- [Worktree engine](../internals/03-worktree.md): the sweep that the `worktreeCleanup.*` keys control.
