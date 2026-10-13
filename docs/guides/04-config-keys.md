# Config keys

Run `playbook config list` to see every key with its value and source tier in the current repo, and `playbook config get <key>` for one.

The `usage.*` keys are also read by the standalone `playbook-telemetry` tool (alias `pbtop`) from the same `~/.config/playbook/config.json`. It has no config file of its own.

## Where a key is read from

The first tier that sets a key wins:

1. Repo: `playbook config set <key> <value>`
2. Org (every repo under one owner): `playbook config set --org <key> <value>`
3. Global (yours, for every repo): `playbook config set --global <key> <value>`
4. The default in the table below.

The command rejects an unknown key, a value of the wrong type, and a number out of range.

## Where the values are stored

All tiers live in one SQLite database, `~/.config/playbook/data/config.db`, not in files you edit by hand. Earlier versions called it `playbook.db`; the first run moves it and renames the old file to `playbook.db.migrated`. Besides the global, org and repo tiers it has an `app` tier per tool (`playbook`, `playbook-telemetry`, `playbook-code`) that the standalone tools read. See the [data folder](../internals/11-data-folder.md). Change values with `playbook config set`. Use `playbook config export` to print every stored value as JSON, and `playbook config import <file>` to load such a document back (it checks every key and value first, and stores nothing if one is invalid).

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
| `worktreeCleanup.staleAfterDays` | `30` | whole number, 0 or more | Days since the last commit of an agent or `playbook cc launch worktree` branch after which the sweep treats it as landed, even when it was never merged. |
| `worktreeCleanup.conflictGracePeriodDays` | `90` | whole number, 0 or more | Days a `/playbook:implement` worktree stopped on a cherry-pick conflict is kept, counted from the timestamp in its stop marker, before the sweep treats it as landed. |
| `mode` | `ask` | `ask`, `auto` | `ask` stops for every decision. `auto` takes the recommended answer, lists it under Assumptions, and keeps going. |
| `maxEffortLevel` | `auto` | `auto`, `low`, `medium`, `high`, `xhigh`, `max` | Playbook's own ceiling on effort, named like Claude Code's key. `auto` sets no ceiling, so Claude Code's `maxEffortLevel` decides, and each command, skill and agent keeps the effort it ships with. The lower of this and Claude Code's `maxEffortLevel` applies, so Claude Code always has the last word and playbook never goes above it. Playbook only reads `maxEffortLevel` and never changes it. Sessions started with `playbook cc` or `playbook ccd` get the ceiling. Set it with `playbook effort <level>` or `playbook config set --global maxEffortLevel <level>`. Only the global tier counts. |
| `effort.auto` | `off` | `off`, `suggest`, `on` | Optional automatic effort choice per dispatch (stage 3 of [ADR-0022](../adr/0022-effort-auto-tune-from-evidence.md)). `off` does nothing. `suggest` logs what it would pick and changes nothing. `on` uses the pick. It stays inside the agent's quality floor and every ceiling. See "Automatic choice per dispatch" below. |
| `effort.agents.<name>`, `effort.commands.<name>`, `effort.skills.<name>` | `auto` | `auto`, `low`, `medium`, `high`, `xhigh`, `max` | A ceiling for one component, for example `effort.agents.fact-checker`. The lowest of Claude Code's `maxEffortLevel`, playbook's `maxEffortLevel` and this key applies, and nothing is ever raised above the effort the component ships with. `auto` and `max` add no limit. A name that matches no plugin file is accepted and flagged by `playbook effort list`. Only the global tier counts. See [ADR-0017](../adr/0017-per-component-effort-ceilings.md). |
| `agents.variants` | `auto` | `auto`, `all`, `off` | Which effort variants of the agents a session gets when you start it with `playbook cc` or `playbook ccd`. `auto` gives the cheaper tiers to every agent, plus `xhigh` to `critic`, `implementer` and `reviewer`. `all` adds every other tier, including `max`. `off` gives none, so only the base agents exist. A variant above any effort ceiling in play is never passed. Sessions started without the launcher get base agents only. |
| `memory.source` | `both` | `both`, `playbook` | Which memory a session uses. `both` leaves Claude Code's own auto memory on, next to playbook memory. `playbook` turns Claude Code's auto memory off for sessions started with `playbook cc` or `playbook ccd`, by setting `CLAUDE_CODE_DISABLE_AUTO_MEMORY=1` in that session only. Playbook never edits Claude Code's settings or memory files. A session started without the launcher keeps Claude Code's own behavior. Set it with `playbook config set --global memory.source <value>`. |
| `models.haiku`, `models.sonnet`, `models.opus` | empty | empty, or `claude-<tier>-<major>[-<minor>]` for that tier | Points a tier alias at another model. Empty keeps the 5.5 default. `playbook cc` and `playbook ccd` export `ANTHROPIC_DEFAULT_<ALIAS>_MODEL` for each override. See [Model tiers](../internals/02-model-routing-and-memory.md#model-tiers-and-the-55-rule). |
| `routing.escalate` | `ask` | `ask`, `auto`, `deny` | What `playbook route` does when a task needs approval (the top model, the high implement tier, or a task that failed twice). `ask` tells the orchestrator to ask you first. `auto` proceeds, and auto mode with `auto.budgetUsd` still caps the spend. `deny` returns the cheaper route instead. Your effort ceiling always wins. |
| `auto.budgetUsd` | `5` | number, 0.01 or more | The spend cap of the main session in US dollars while the mode is `auto`, counted on its own transcript. |
| `auto.agentBudgetUsd` | `5` | number, 0.01 or more | The spend cap of each subagent in US dollars while the mode is `auto`, counted on its own transcript. At its cap only that agent is parked, and it can still commit, push and open a pull request. |
| `auto.sessionCeilingUsd` | `0` | number, 0 or 0.01 and more | A ceiling on what the whole tree of agents has spent. At the ceiling every agent is parked. `0` means ten times `auto.budgetUsd`. |
| `auto.warnPct` | `70` | whole number, 1 to 100 | The percent of a cap (`auto.budgetUsd` for the main session, `auto.agentBudgetUsd` for a subagent) at which the spend hook warns that agent once. |
| `fix.maxFiles` | `3` | whole number, 1 or more | `/playbook:fix` stops and hands off to `/playbook:plan` when more files than this change. |
| `fix.maxLines` | `500` | whole number, 1 or more | `/playbook:fix` stops the same way when more lines than this change, not counting the new test. |
| `review.stackMaxPrs` | `6` | whole number, 1 or more | The quick and deep review commands ask again before reviewing a whole PR stack with more open PRs than this. |
| `review.stackMaxLines` | `3000` | whole number, 1 or more | They ask again when the open PRs of the stack change more lines than this. |
| `security.defaults` | `false` | `true`, `false` | `playbook init` also merges the shipped security defaults (the `permissions` block) into `settings.json`. `init --security` does the same for one run and `--no-security` skips them even when this is true. Off, `init` leaves the permissions you already have as they are. See [Security](../../README.md#security). |
| `hygiene.enabled` | `true` | `true`, `false` | The context hygiene nudge: once the main session passes `hygiene.thresholdTokens`, it asks the model to save a handoff and tells you to run `/clear`. It never clears or compacts for you. See [hooks](../internals/01-launcher-and-hooks.md#userpromptsubmit). |
| `hygiene.thresholdTokens` | `250000` | whole number, 10000 or more | The main-session context size in tokens at which the nudge fires, once per crossing of each multiple. |
| `hints.pluginRemoved` | `true` | `true`, `false` | When `playbook init` saw the plugin installed and it is no longer in Claude Code's plugin list, the status line shows one line and the SessionStart hook adds one line of context, both telling you to run `playbook uninstall --yes`. Set it to `false` to hide both. Nothing is removed for you. |
| `docs.convert` | `auto` | `auto`, `off` | Whether playbook suggests (only when the anydoc converter is installed) `playbook doc read` for PDF and Office files. The command itself works whenever the optional `anydoc` converter is installed. |
| `docs.maxBytes` | `26214400` | whole number, 1 or more | The largest file `playbook doc read` opens, in bytes (25 MB). |
| `docs.timeoutSeconds` | `30` | whole number, 1 or more | The converter is stopped after this many seconds. |
| `docs.maxOutputChars` | `200000` | whole number, 1 or more | The text printed is cut after this many characters, with a marker that says how many were shown. |
| `docs.maxMemoryMb` | `1024` | whole number, 1 or more | The converter is stopped when its resident memory passes this many megabytes. |
| `docs.anydocPath` | empty | a path, or empty | The converter to run. Empty means the first `anydoc` on PATH. |
| `usage.weekStart` | `monday` | `monday`, `sunday`, `saturday` | The first day of the week for the `week` and `last-week` ranges of `playbook usage` (view, `--json`, `--summary`). Days are UTC. |
| `usage.refreshSeconds` | `5` | whole number, 1 to 3600 | How often the `playbook usage` terminal view reads new history and redraws, in seconds. |
| `usage.theme` | `default` | `default`, `dark`, `light`, `mono` | The color theme `playbook usage` starts with. `t` cycles the themes inside the view. `NO_COLOR` forces `mono`. |
| `sessions.staleAfterDays` | `3` | whole number, 1 or more | Days without activity after which `playbook sessions` calls an unfinished session stale. |
| `sessions.recentDays` | `7` | whole number, 1 or more | How far back `playbook sessions` looks for sessions. |
| `usage.budget.monthlyUsd` | `0` | number of dollars, 0 or more | A manual monthly spend limit for `playbook usage`. `0` means none. Used when the limit cannot be read from Claude Code. Set it with `--global`, like the other `usage.` keys. The month is the UTC calendar month, and the spend is the estimated cost from your local history. |
| `usage.budget.weeklyUsd` | `0` | number of dollars, 0 or more | The same for the week (the first day comes from `usage.weekStart`). |
| `usage.autoIngest` | `true` | `true`, `false` | Whether the `usage-warm` hook starts a background read of new session history at session start and end, so `playbook usage` opens with data. It runs detached at low priority, stops after about two minutes (the next run resumes), and never blocks or prints. `false` turns it off. |
| `usage.autoIngestMinutes` | `10` | whole number, 0 or more | The least time between two background reads. A hook that fires sooner than this after the last finished read starts nothing. `0` allows one at every session start and end. Only one read runs at a time. |
| `usage.telemetry` | `auto` | `auto`, `off` | Whether `cc` points a session at the local usage receiver (`playbook usage collect`) so `playbook usage` can show cost per agent. With `auto` it sets the OpenTelemetry variables for that session only, and only when you have set no telemetry yourself, starting the receiver in the background if none runs (once the usage database exists, and it stops after 10 idle minutes) (no `CLAUDE_CODE_ENABLE_TELEMETRY` or `OTEL_*` variable in your environment or in the `env` block of a Claude Code settings file). `off` sets nothing, and `playbook init --telemetry` writes nothing then either. Playbook reads those files and writes them only when you run `init --telemetry` and say yes, and only when none of those variables exist. `playbook usage status` says which case applies. See [usage telemetry privacy](../internals/04-usage-dashboard.md#telemetry-privacy). |
| `usage.telemetryRetentionDays` | `90` | whole number, 0 or more | How many days of telemetry rows `playbook usage` keeps (API requests, tool calls, API errors, metric points). Older rows are deleted when the receiver starts, every few hours while it runs, and by `playbook usage prune`. `0` keeps them. Transcript data and every total are untouched. |
| `usage.telemetryMaxRows` | `500000` | whole number, 0 or more | The most telemetry rows kept per table, the newest ones. `0` means no limit. Applied together with the retention days. |
| `launcher.claudeOverride` | `false` | `true`, `false` | Whether `playbook shell-init` also defines a `claude` shell function that loads the playbook system prompt for a plain interactive session. Calls that need Claude Code untouched (piped or scripted use, `-p`, subcommands, your own prompt or settings flags, `PLAYBOOK_BYPASS=1`) go straight to the real `claude`. Run `command claude` to skip the function once. |
| `launcher.shortName` | `false` | `true`, `false` | Whether `playbook shell-init` also defines `pb` as a short name for `playbook`. It is defined only when nothing else uses the name: `playbook init --short-name` writes the key only when no command named `pb` is on your PATH, and `shell-init` checks again each time your shell starts and skips `pb` when a command, alias, function or builtin of that name exists. It never overwrites one. |
| `launcher.systemPromptMode` | `append` | `append`, `replace` | How the launcher (`playbook cc`, `playbook ccd` and the `claude` function) loads the playbook prompt. `append` adds it to Claude Code's own prompt and keeps its tool and safety text. `replace` swaps Claude Code's prompt out for the playbook one. A system prompt flag you pass yourself wins and playbook adds none. |
| `hooks.rmGuard.allowedRoots` | empty list | list of absolute or `~/` paths | Extra directories whose contents the `rm` guard lets you delete, on top of the repo, `~/.claude`, `~/.cache`, the system temp folders and the `.worktrees` folder next to your repo. Only what is inside a listed directory is allowed, never the directory itself or a parent. Each path must be at least two levels deep and hold no `..`, glob or quote characters. Set it with a JSON array or a colon-separated list, for example `playbook config set --global hooks.rmGuard.allowedRoots '["~/scratch/builds"]'`. A symlink that leaves a listed directory stays blocked. |

## Tuning effort from your usage

`playbook effort suggest` lists the agents where a change to effort looks safe or needed. It reads your local usage store (the same one `playbook usage` fills) and prints, for each agent, how often it was dispatched, how often at a higher or lower variant than it ships at, repeats inside a session, and cost per effort level for its model. Add `--json` for scripts and `--range 30d|60d|90d|month|all` to change the window (default `30d`).

- `suggest` only reads. It never writes your config, your memory or any Claude Code settings. To act on a suggestion, use the patch below, run the `playbook config set --global effort.agents.<name> <level>` command it prints, or ignore it.
- It only proposes a lower level where a benchmark showed quality holds (the quality floor table in `crates/cli/src/effort/floor.rs`). Today every agent ships at its floor, so a clean report usually says no change. Agents where a missed finding is costly (the reviewer and the auditor) are never proposed for lowering.
- It tells you when a ceiling you set sits below an agent's floor, which can make results worse.
- With less than 7 days of history, or fewer than 20 dispatches over 5 sessions for an agent, it says there is not enough data and proposes nothing.
- Cost is per model and effort level and includes the main session, because a usage row does not say which agent made it. Read it as a rough guide.

The thresholds are constants in `crates/cli/src/effort/suggest.rs`.

### Apply a patch, and undo it

- `playbook effort suggest --patch` (add `--json` for scripts) prints the proposed `playbook config set --global effort.agents.<name> <floor>` lines with the evidence for each. It writes nothing.
- `playbook effort apply` builds the same patch, shows it and asks `y/N`. `--yes` approves without asking, and is required when input is not a terminal. Nothing is applied without one of the two.
- A patch line is a global ceiling for that agent, so it applies to every project and every diff. It never goes below the agent's quality floor, never names an agent marked costly to miss (the reviewer, the auditor), and never raises anything above a ceiling.
- Before writing, the previous global `effort.*` keys are saved to `~/.config/playbook/effort/backups/config-<time>.json`. The keys changed are recorded in `effort/ledger.json`, and each change is appended to `effort/changes.log` (one JSON object per line, UTC time). The write goes through playbook's own config store and atomic file helpers. Claude Code's settings are never touched.
- `playbook effort reset` puts each patched key back to what it was before, and leaves a key you edited by hand since. `playbook effort reset --all` also sets every other global `effort.*` key to its default. A backup is written first.

### Automatic choice per dispatch

Off by default. Set it with `playbook config set --global effort.auto suggest` (log only) or `on`.

- When the orchestrator dispatches an agent it asks `playbook effort choose <agent> --base <ref>`. The answer is the name of an existing variant (`reviewer-low`, `reviewer-xhigh`), because the `Agent` tool has no per-call effort and a variant file is how effort is set today.
- The choice looks at the diff (changed lines, files, bytes, languages), a risk class (paths such as auth, secrets, crypto, migrations and CI workflows, plus memory facts anchored to touched paths that name a security, hot, flaky or incident risk) and the usage evidence from `suggest`.
- Risky or large diffs are raised, to `xhigh` at most. Small clean diffs are lowered, to `low` at most and never below the agent's quality floor, and only when the usage evidence has enough history and shows no escalations. The reviewer, the auditor and any agent marked costly to miss are only raised.
- It never goes above a ceiling: Claude Code's `maxEffortLevel`, playbook's, the `effort.agents.<name>` key and the model cap all apply first, and the choice is one of the files they allow.
- Any error (a diff that cannot be read, a bad input, no usage store) gives the default.
- Every choice is appended to `~/.config/playbook/effort/auto.log` (UTC time, component, default and chosen level, inputs, reason). `playbook effort log` shows it, and a dispatch that differs from the default prints one line saying so. Nothing leaves your machine.

## See also

- [Review and PR flow](02-review-and-pr-flow.md#pr-and-commit-settings): how the PR keys behave in detail.
- [Auto mode](05-auto-mode.md): what `mode`, `auto.budgetUsd`, `auto.agentBudgetUsd`, `auto.sessionCeilingUsd` and `auto.warnPct` turn on.
- [Plan and implement](01-plan-and-implement.md): the `/playbook:fix` limits.
- [Worktree engine](../internals/03-worktree.md): the sweep that the `worktreeCleanup.*` keys control.
