# Usage view

`playbook usage` shows how many tokens and how many dollars your sessions use, split by day, week, model, repo, branch, effort, account, skill and agent. It reads Claude Code's own session history, so it needs no setup and no new instrumentation.

## Commands

| Command | What it does |
|---|---|
| `playbook usage` | Opens the terminal view. When stdout is not a terminal, prints the text summary instead. |
| `playbook usage --summary` | Reads anything new, then prints the text summary and exits. With `--range` it covers only that range and says which dates. For `today`, `week` and `month` it ends with the [period comparison](#period-comparison). |
| `playbook usage --json` | Prints the totals, the groups and the live view as JSON, for scripts. For `today`, `week` and `month` it adds a `comparison` object. |
| `playbook usage --range <name>` | Sets the range for the terminal view, `--json` and `--summary`. See [Ranges](#ranges). The default is `week` for the terminal view, `30d` for `--json` and all time for `--summary`. |
| `playbook usage collect` | Runs the local OpenTelemetry receiver in the foreground. `cc` or the `session-init` hook starts it for you when needed. See [The telemetry receiver](#the-telemetry-receiver). |
| `playbook usage status [--json]` | Shows whether the receiver runs, the telemetry row counts and the retention limits. Starts nothing. |
| `playbook usage prune [--dry-run]` | Deletes telemetry rows past the retention limits. See [Checking and running the collector](#checking-and-running-the-collector). |
| `playbook usage stop` | Stops the running collector through its authenticated shutdown route, waiting up to 3 seconds before it kills one that does not answer (only after checking the pid is the playbook collector). Prints `stopped` or `not running`. |
| `playbook usage doctor [--raw]` | Lists where `pbtop` can read your limits on this machine: the plan fields and cached usage numbers in the Claude Code config file, the statusline reading and the manual budgets. It prints field names, ages, the plan label, the window percentages and the manual budget amounts, and never an email, name, id, organization or token. `--raw` adds the values of six plan fields and the cached spend limit and amount used, in dollars. Run it on another machine and share the output when the limits look wrong. |

Every command that shows data reads new history first, so you never need to ingest by hand.

## The terminal view

The view is styled after a classic terminal system monitor, and the `default` theme is that look: a black ground, rounded boxes with the key hint and title in the top border (`╭─┤¹Spend per day├──╮`), a gradient on the graphs and the meters, and a highlighted selected row. Each box has its own outline color: spend uses the cpu green, tokens the net purple, models and projects the mem olive, sessions and messages the proc red.

The view needs a screen of at least 80 columns by 24 rows. Below that it says so and draws nothing else. The panels are split across tabs, three by default: Charts (spend and tokens), Numbers (models, breakdown, the Budget panel and the Projection panel, see [Spending limits and budget headroom](#spending-limits-and-budget-headroom)) and Sessions (live sessions and sessions per day). A header with the range, the cost, the tokens and the messages is on every tab, and a tab bar under it names the tabs. The tabs and their panels come from the `usage.ui.layout` config key (see the [config keys page](../reference/config-keys.md)). A layout with one tab keeps the old single screen, with the Budget panel from 36 rows. Resizing needs no handling because every redraw reads the current size.

For `today`, `week` and `month` the header is a comparison: it says which two spans are compared and that they cover the same elapsed span (see [Period comparison](#period-comparison)), then shows cost, tokens and messages for both with the change in amount and percent (`new` when the previous value was zero). Increases are yellow and decreases green, and the sign says it without color. The other ranges show their own totals and say `(no previous period to compare)`. Everything is UTC and the header says so, with the first day of the week when the range is a week.

The view re-reads the store every 5 seconds (set `usage.refreshSeconds`, 1 to 3600, to change it) and the footer shows when it last did (`updated 01:46:40 UTC, every 5s`), or why the last read failed. A refresh swaps in the new snapshot without touching the chart cursor, the focus, the sorts, the spend view, a filter being typed or the selected rows: each table finds its selected row again by what it is (a model, a project, a session, a day), so a row that moved in the order stays selected, and the scroll follows it. A short footer keeps the time and drops the interval.

The panels:

| Key | Panel | Shows |
|---|---|---|
| `1` | Spend per day, USD | A bar chart of the cost for each UTC day in the range, with axes (below). `v` steps it through three views: the total, the same bars stacked by model, and a table with a row for each day (see below). |
| `2` | Tokens per day | The same for tokens, in the net gradient. |
| `3` | Models | Messages, tokens and cost per model, with a swatch in the model's color. For `today`, `week` and `month` it also shows the cost in the previous period and the change in amount and percent, for every model used in either period (a model that only the previous period used reads `$0.00` now, and one with no spend before reads `new`). Other ranges show a share meter instead. `s` also sorts by the previous cost and the change. |
| `4` | Projects | The same, per repo. `v` steps it through Agents, Skills, Tools and Health: requests, tokens and cost per agent or per skill from telemetry, with a note of how much of the spend telemetry covers. With no telemetry rows in the range it says `Telemetry off` and how to turn it on. Agent, skill, tool and health rows cannot filter the view. Tools and Health are described under [Tool latency and health](#tool-latency-and-health). |
| `5` | Live sessions | Sessions with a message in the last 15 minutes, with state, cost and age. |
| `6` | Sessions per day | Every session in the range, grouped by UTC day with the newest day first. Each row has the short session id, project, main model, messages, tokens and cost for that day, and a session that runs past midnight has a row for each day. The day is named on the first row of its group, and `s` orders the sessions inside each day. `Enter` filters the view to the session. |

### The charts

The two charts are bar charts drawn with eighth blocks, one bar per UTC day, with idle days at zero and the days still to come in a week or month in progress left empty. Today charts the last seven days, since one bar says nothing, and the title says so (`Spend per day, USD, 7 days`).

- **y axis.** Labels at the bottom, middle and top, with a blank column between them and the box, in dollars or tokens with units (`$0`, `$2.5`, `$250`, `$2.5K`, `500K`, `2.5M`). The top is a round value above the peak (1, 1.2, 1.6, 2, 2.5, 3, 4, 5, 6 or 8 times a power of ten), so the labels are round and the middle is half the top. Faint dotted guides mark the top and the middle.
- **x axis.** One bar a day (a week, a month that fits, today's seven days) shows the weekday on one row and the day of the month, 1 to 31, on the row directly below it, for every bar where each cell is wide enough for the label and a gap. The weekday row needs a letter and a gap in every cell, so a month in a narrow panel has only the numbers. When the numbers do not fit, every second, third or seventh day is labeled, always with the first and the last bar (a number that would touch the last one is left out). No full dates sit on the axis: the header carries the period's UTC dates (`this week Oct 6 to Oct 12 until 12:00`, `last 30 days Sep 13 to Oct 12, 2026`), and the readout line names the day under the cursor in full. When a bar holds several days (a wide range), the axis keeps the short first, middle and last date. Today's labels are bold and underlined, the peak day's are yellow, and the cursor's are highlighted.
- **Totals line.** The total, the peak (with its date) and the average per day that has happened, for the range.
- **Cursor.** `Left` and `Right` move it one bar, `Home` and `End` jump to the first and last bar while a chart has the focus. A readout line gives the day, its cost, messages, tokens and top model with its share (the token chart leads with tokens). It starts on today, and both charts share it. Changing the range resets it.
- **Legend.** The bar color follows its height on the axis along the theme gradient (green to red for cost), and the legend in the bottom border says so. The mono theme has no colors and no legend.
- **Fill.** A week or more of bars always fills the width of the plot, and fewer bars are never wider than 16 columns.
- **Wide ranges.** When the range has more days than the chart has columns, days are added up into bars of equal length (the title says `Spend per 3 days`), so nothing is cut off. The axis, readout and cursor then speak of those bars.
- **Small panels.** Rows go in this order when the panel is short: weekday letters, totals line, readout, day numbers, so the plot keeps three rows. With no totals line the peak sits in the bottom border, so the top border keeps room for the `v view: <name>` tag. The y axis goes when the panel is narrower than 16 columns, leaving the bars.

### Daily cost and cost per model

`v` switches the spend panel between three views of the same days.

- **Total.** One color per bar along the gradient, as above.
- **By model.** Each bar is stacked by model, the biggest model at the bottom, and the legend in the bottom border names them (`■ opus-5 ■ sonnet-5 +1` when they do not all fit). The edge between two models is exact to an eighth of a cell: a cell that holds an edge is drawn as a block in the lower model's color on the upper model's. Without color (the mono theme) the models are told apart by pattern (`█ ▓ ▒ ░ ▚ ▞ ▛ ▟`).
- **Daily.** A table with a row for each day up to today, newest first: date with weekday, messages, tokens, cost and the model that cost the most that day. `s` and `S` sort it by any column, the selection works as in the other tables. Today is bold and underlined, the peak day is bold and yellow, and the legend in the bottom border says so. Idle days stay as zero rows, so it reads as a calendar.

A model keeps one color everywhere: its swatch in the Models panel, its band in the stacked bars, its legend entry and its name in the daily table. The color comes from a hash of the model name, so it stays the same when the range changes. Two models that want the same color in one range get the next free one, and with more than eight models the colors are shared.

Keys:

| Key | Action |
|---|---|
| `Tab`, `Shift-Tab` | Show the next or previous tab. With one tab they move the panel focus instead. |
| `F1` to `F9` | Show a tab by its number. |
| `e` | Open the layout editor: `Left`, `Right` pick the tab, `j`, `k` pick a panel, `Space` adds or removes it, `J`, `K` reorder panels, `<`, `>` reorder tabs, `n` adds a tab, `x` deletes one, `r` renames, `w` saves, `R` resets to the default tabs, `Esc` cancels. Saving writes `usage.ui.layout`; when there is no config store the footer shows the command to run instead, until the next key. `R` writes an empty layout, which means the default tabs. |
| `1` to `6`, `]`, `[` | Focus a panel of the current tab by its number, or the next or previous one. The focused panel's hint is a badge. |
| `Left`, `Right` | Move the chart cursor one day (or one bar). `Home`, `End`, `g`, `G` jump to the ends while a chart has the focus. |
| `Up`, `Down`, `j`, `k`, `PageUp`, `PageDown`, `Home`, `End`, `g`, `G` | Move the selected row of the focused table. |
| `v` | Change the view of the focused panel, and only that panel. With Spend focused it steps total, by model and daily table. With Projects focused it steps projects, agents, skills, tools and health. On the other four panels it does nothing. The title of Spend and Projects always shows `v view: <name>`, and the other panels show no such tag. |
| `h`, `?` | Open the help window: every key, grouped, with the panels each one acts on. |
| `R` | Read new usage now. The view also refreshes by itself. |
| `s` | Sort the focused table by the next column. A marker in the header shows the column and the direction. Text columns start ascending and number columns descending. |
| `S` | Reverse the sort. |
| `Enter` | Filter the whole view to the selected row: a model, a project or a session id. `Enter` on that row again lifts the filter. |
| `r` | Step between today and this week, the two default views. From any other range it goes to today. |
| `d`, `w`, `m` | Jump to today, this week, this month (month to date). Each is shown against the previous period. |
| `W`, `M`, `a` | Jump to last week, last month, all time. These are plain ranges with no comparison. |
| `/` | Type a filter, a case insensitive substring of model, repo, branch or session id. `Enter` applies it, `Esc` cancels, `Backspace` edits. |
| `c` | Clear the filter. |
| `p` | Open the [Sessions management view](09-sessions.md#the-sessions-view), a full screen table of recent sessions with their PR, state and suggestions. |
| `t` | Cycle the theme: default, dark, light, mono. |
| `q`, `Esc`, `Ctrl-C` | Quit. |

The view is keyboard only, so the terminal's own text selection and scrolling keep working.

**Help window.** `h` or `?` opens a centered window over the panels. It groups the keys (navigation, periods, views, sorting, filtering, refresh, quit) and says which panels each key acts on. It is sized for 80 by 24, and when the screen is shorter than the text the arrows, `PageUp`, `PageDown`, `Home` and `End` scroll it. Any other key closes it, and `Ctrl-C` still quits. Data keeps refreshing underneath and nothing the user set (focus, selection, sorts, views, cursor) changes. The key handler, the footer hints and the help window all read one table, `BINDINGS` in `crates/telemetry/src/tui/keys.rs`, so the help cannot list a key that does not work or miss one that does. A test presses every key on every panel and fails if one changes anything without a row in the table. The footer shows each hint with its verb (`m month`, `/ filter`, `h help`), and on a narrow screen drops hints in the table's `drop` order, always keeping `v view`, `/ filter`, `h help` and `q quit`, and `Tab tab` when there is more than one tab.

### Ranges

Every range is a set of whole days in the chosen time zone, shown in the header with its name and exact dates (`last week Aug 24 to Aug 30, 2026`). The zone is local time unless `--utc`, `PLAYBOOK_TELEMETRY_TZ` or `usage.timeZone` picks UTC, and the view, the summary and the JSON name it. In local time a day can be 23 or 25 hours long around a daylight saving change.

| Name (`--range`) | Days |
|---|---|
| `today` | Today only. |
| `week` | The calendar week that holds today, from its first day for seven days (it can run into next month, and the graph stops at today). |
| `last-week` | The seven days before that. A week can span a month boundary. |
| `month` | The 1st to the last day of this month. |
| `last-month` | The whole previous calendar month, from December when it is January. |
| `30d`, `60d`, `90d` | The last N days, including today. |
| `all` | Everything stored. |

The terminal view opens on `week`, compared with last week, and `r` steps between `today` and `week`. `today`, `week` and `month` (month to date) are periods and are shown against the previous one, so `last-week`, `last-month`, `30d`, `60d`, `90d` and `all` do not conflict with the comparison and stay: `W`, `M` and `a` jump to them, `--range` opens on any of them, and they show their own totals with no comparison.

Weeks start on Monday. Set `usage.weekStart` to `sunday` or `saturday` to change it (`playbook config set usage.weekStart sunday`). The terminal view, `--json`, `--summary` and the web `range=` parameter all read the key, so they agree. The key is spelled in camel case like the other config keys.

### Period comparison

`today`, `week` and `month` are periods: each is compared with the one before it (yesterday, last week, last month) over the same elapsed span, so a Thursday afternoon is never set against a whole week. The code is `crates/telemetry/src/period.rs`.

- The current span runs from the start of the period through the second of `now`. The previous span starts at the start of the previous period and lasts as long, but never runs past that period, so on March 31 the month so far is compared with all of a 28 day February. The header and `--summary` state the elapsed span.
- Everything is UTC seconds. A week starts at 00:00 UTC on the day `usage.weekStart` names, and an event at 23:59:59 UTC belongs to the earlier day whatever your local clock says.
- Cost, tokens and messages each get the amount of change and the percent. The percent is `n/a` (JSON `null`) when the previous value was zero.
- `--json` adds `comparison`: `period`, `span_seconds`, `current` and `previous` (`start`, `end`, `start_date`, `end_date`, totals), `change` for the three numbers, and `models` with each model's current and previous group.

While you type a filter every key is text, so `q` and `r` do not quit or change the range until you press `Enter` or `Esc`.

### Themes

| Theme | Look |
|---|---|
| `default` | The default look: black ground, `#cc` text, `#ee` titles, `#b54040` key hints, `#6a2f2f` selected row, per box outline colors, and three point gradients (start, mid, end) for the graphs and meters. |
| `dark` | The terminal's own ground with single colors. |
| `light` | The same with darker colors for a white ground. |
| `mono` | No color. The selected row is shown in reverse video. |

**First read of a large history.** The view opens at once and shows what the store holds. On a store with no watermark yet, a background thread reads transcripts in batches of up to 250 files, newest file first. Each batch commits its rows and the files it finished in one transaction, so the 5 second refresh shows the partial store growing and the footer reads `indexing 120/800 files` (counts only). An interrupted run resumes from the files not yet recorded. The source watermark moves only after the last batch, so an unread older file is never skipped. While it runs, an empty range says `Indexing` and that data will appear. `usage --summary` and `--json` still ingest fully before they print, and past two seconds they write the same file count to stderr. Several readers and writers share the store: it is in WAL mode with a 5 second busy timeout, and each batch is a short immediate transaction.

When there is nothing to chart, the body says why instead of showing empty panels. If the range has no messages but the store has some, it names the dates the store covers and the keys that widen the range (`a` all time, `m` month, `w` week), and when a filter is set it says how to clear it (`c`). If the store is empty, it names the folder transcripts are read from, how to check it, and `CLAUDE_CONFIG_DIR`, which moves that folder when Claude Code uses it. A failed read fills the body with the error before the first data and shows a banner over the last good numbers after it.

The theme comes from the `usage.theme` config key (`default`, `dark`, `light`, `mono`; see [config keys](../reference/config-keys.md)) and `t` changes it for the session. `NO_COLOR` set to anything but empty forces `mono` and locks it.

A small TV icon shows before the title in the top-left corner of the dashboard and the Sessions screen. It sits in a fixed three-cell block, so columns hold at emoji width 1 or 2. Set `PBTOP_ICON_WIDTH=1` if your terminal draws it one cell wide. The icon is left out, with plain text only, when the locale is not UTF-8, `NO_COLOR` is set, or `TERM` is `dumb`.

Colors follow the terminal. With `COLORTERM=truecolor` or `24bit` the `default` values are drawn exactly. Otherwise, when `TERM` names `256color`, each value goes to the nearest of the 256 color cube and gray ramp, and with neither it goes to the nearest of the 16 basic colors. The values are adapted from a third party theme under the Apache License 2.0, like playbook, and the attribution is in `NOTICE`.

The usage code lives in `crates/telemetry` (package `playbook-telemetry`), which playbook depends on by path; the paths below are relative to that folder.

## Where the data comes from

The source is `~/.claude/projects/**/*.jsonl`, the session transcripts Claude Code already writes. Per assistant message they carry the model, the effort, the working directory, the git branch and the token counts (input, output, cache write, cache read). Skill and agent calls appear as `tool_use` entries.

Four details are worth knowing:

- **One message spans several lines.** Each content block of a message is its own transcript line, and each line repeats the same usage. Counting lines would roughly double every number, so events are deduped by message id (and by tool call id).
- **Transcripts have no cost field.** Cost is computed from a per-model price table in `crates/telemetry/src/pricing.rs`, copied by hand from the published pricing page (the source and date are in the file). Input, output, cache reads and both cache write lifetimes (five minute and one hour) are priced separately, and the cost is worked out each time you read the data, so a price fix applies to old events without a re-ingest. It is still an estimate and can drift from published prices. A model that is not in the table is never guessed: its messages count as $0, and the summary and the JSON (`totals.unpriced_messages`, `totals.unpriced_models`) say how many and which models.
- **The repo is the real repository, not the last folder.** The working directory of a worktree or an agent folder would otherwise show up as its own repo. `crates/telemetry/src/repo.rs` maps a directory to its repo: first by path (`.claude/worktrees/<name>`, `.git/...`, and playbook's own `repos/<owner>/<repo>` storage, which still works after the directory is deleted), then by asking git for the shared git dir, then by the folder's own name.
- **The account is not in the transcripts.** It is read once per ingest from `oauthAccount.emailAddress` in `~/.claude.json` and tagged onto every event. If you are not signed in, it is `unknown`.

## Storage

One SQLite database at `~/.config/playbook/data/usage.db`, not tied to any repo, because spend only makes sense as a total. The standalone [pbtop](../../crates/telemetry/README.md) tool reads and writes the same file, so both show the same numbers. The first run moves an older database from `~/.config/playbook/usage.db`, `~/.config/playbook/usage/usage.db` or `~/.local/state/playbook-usage/usage.db` into place and renames the old file to `usage.db.migrated`. Events stay there after Claude Code or `playbook cc` prunes old sessions. Connection setup follows `crates/cli/src/gate/db.rs`: `busy_timeout` first, then WAL mode, because a refresh reads while `ingest` writes from other processes.

The first ingest after upgrading from an older version runs a one-time repair (`crates/telemetry/src/backfill.rs`): it rescans the transcripts and corrects the repo and the cache lifetime split on rows stored earlier. A row whose transcript is gone keeps its old repo and is priced as five minute cache.

Ingest keeps a watermark per source (the newest timestamp it has read). It skips transcript files last modified before the watermark, so a repeated ingest is cheap. The watermark is inclusive and events are deduped by id, so re-reading the newest event is harmless. Each ingest runs in one `IMMEDIATE` transaction: the rows and the watermark commit together or not at all, and a second ingest waits for the first instead of failing.

## Adding another agent

`UsageSource` (`crates/telemetry/src/lib.rs`) is the extension point. A source returns normalized `UsageEvent` and `ToolInvocationEvent` values newer than a watermark. Aggregation, storage, the summary and the terminal view never see an agent's own file format, so a second source only has to implement the trait. Two sources exist today: Claude Code (`crates/telemetry/src/claude_code.rs`) and Codex (`crates/telemetry/src/codex.rs`). Every stored event carries an `agent` column (`claude-code` or `codex`, old rows default to `claude-code`).

The Codex source reads `~/.codex/sessions/**/rollout-*.jsonl` read-only and is skipped silently when that directory is missing. Format source: openai/codex `codex-rs/protocol/src/protocol.rs` (`TokenCountEvent`, `TokenUsageInfo`, `SessionMeta`, `TurnContextItem`). The model, effort and cwd come from `turn_context`, the session id, cwd and branch from `session_meta`, and tokens from `event_msg` records of type `token_count`. One event is one change of the cumulative `total_token_usage.total_tokens`, taking its tokens from `last_token_usage`, and its id is `codex:<session>:<timestamp>:<total>`, so re-reads dedupe. Cached input is reported as cache reads. OpenAI models are not in the price table, so Codex events show as unpriced rather than guessed. Compressed `.jsonl.zst` rollouts are not read, and the account is `unknown` because rollouts carry none.

## Tests

Tests run against hand-written transcripts in `crates/telemetry/tests/fixtures/usage/` and a scratch `HOME`, never your real `~/.claude`.

Terminal view tests render into a `TestBackend` buffer, with no real terminal. `crates/telemetry/src/tui/snapshots.rs` compares whole screens at fixed sizes with the text files in `crates/telemetry/tests/fixtures/usage_tui/`; after an intended layout change, regenerate them with `UPDATE_SNAPSHOTS=1 cargo test --lib usage::tui::snapshots` and review the diff. `crates/telemetry/src/tui/parity.rs` checks that, for every range, the view reports the same totals and model, repo and day groups as the web JSON, and again for events on both sides of every week and month edge (a week that spans a month boundary, a Sunday week start, a leap February). It also checks that the comparison in the view and in `--json` agrees with dates worked out by hand: the same elapsed span in the previous period, the second before and after midnight UTC, and a Sunday week start. The period math itself is tested in `crates/telemetry/src/period.rs`. The snapshots cover 80x24, 100x30 and 160x50, the daily table and the stacked view at those sizes, and a week range. The scale math, the chart geometry and every range's bounds have their own unit tests, and a chart draw test runs the chart at every size from nothing up to 70x22. The fixtures are synthetic: no real project, model spend or session ever goes into a snapshot.

## Limits

- Cost is an estimate (see above). It is the price at the published API rate, not what a subscription plan charges you.
- It reads one machine's local history. It does not combine machines or users.
- Live shows what the transcripts record, so a session that is thinking but has not written a message yet does not appear until it does.
- The terminal view needs a real terminal of 80x24 or more. Use `--json` or `--summary` where there is none.
- No LLM call is built in. The output is plain text and JSON, so the agent already running your session can read it and suggest where to cut cost.

## Spending limits and budget headroom

The Budget panel, `--summary` and `--json` (`limits`) show how much of a limit you have used and how fast you are using it. The code is `crates/telemetry/src/limits.rs` (the math) and `crates/telemetry/src/limits_source.rs` (where limits come from). All times are UTC.

### Where a limit comes from

| Account | What can be read | Source | Tag in the UI |
|---|---|---|---|
| Claude Pro or Max | The 5 hour and the weekly window: percent used and reset time. | The `rate_limits` object Claude Code passes to the statusline command on stdin ([statusline docs](https://code.claude.com/docs/en/statusline)). The playbook statusline keeps the latest copy in `~/.config/playbook/data/rate_limits.json` and a short history of the two percentages in `rate_limits.jsonl` next to it (both owner only; see [The reading history](#the-reading-history)). | `measured` |
| Behind a Claude apps gateway | The spend limit: percent, reset time and, when the gateway and Claude Code are new enough, dollars used, the dollar limit and the period. | `rate_limits.spend_limit` in the same statusline input. | `measured` |
| Enterprise, or Claude Code with a cache | The monthly spend limit in dollars and the dollars used, and the two windows when the plan has them. This source is not documented and can change or be missing. | The usage numbers Claude Code caches in its config file (`~/.claude.json`, or `$CLAUDE_CONFIG_DIR/.claude.json`): `cachedUsageUtilization.utilization.spend.limit` and `.used` (minor units, dollars only, skipped when `spend.enabled` is false), its `utilization.five_hour` and `utilization.seven_day` windows and `fetchedAtMs`, plus the two rate limit tier names under `oauthAccount` for the plan. Only these fields are read. | `cached`, with how old it is |
| API organization | Nothing automatic. The organization budget is set in the Console and no documented API reads it. The Admin API needs an organization admin key and returns usage and cost reports, not the limit. | `usage.budget.monthlyUsd` and `usage.budget.weeklyUsd`, set by you. | `manual` |
| Any account | Your own estimated spend, from the session transcripts. | The usage store. | `local history` |

Three things to know:

- A statusline reading that carries one window keeps the other window as it was stored, with its own read time, until that window resets. A reading is stale as soon as one part held is over 6 hours old.
- The statusline input has no limit data on an API key account, and a Pro or Max account gets it only after the first response of a session. So the plan windows show up once you have used Claude Code with the playbook statusline (`playbook statusline`) in this setup. Each window can be missing, and Claude Code drops a window after its reset time, so playbook does the same. A reading older than six hours is tagged `stale`.
- Playbook reads only what Claude Code already passes to a local command and a few plan and usage fields of the Claude Code config file. It calls no endpoint with your login, reads no credential store, keeps no email, name, id or organization in the limits code (the account tag on each event is described under ingest), and writes nothing into Claude Code settings or memory. The stored file holds percentages, reset times and the gateway dollar limit, and no account, token or session id.

The monthly limit is picked in this order. A monthly spend limit from the statusline (behind a gateway) comes first, and the cache is not used then. Next a cached spend limit that is under six hours old, because it is what Claude Code enforces. Otherwise `usage.budget.monthlyUsd` wins when set. A cache older than six hours is used only when no budget is set, and then it is tagged `cached, stale` with its age. A cached window is used only while the cache is under six hours old and only when the statusline has no reading for that window. A fresh cached `used` replaces this machine's estimate, because it can cover every machine on the account (it is ignored when it was fetched before the month began), while the "last 24h" pace in the Projection panel always comes from this machine. The cache is read again only when the file changes, and a half written file keeps the last good reading. The period of the cached limit is assumed to be the calendar month.

The plan name comes from the rate limit tier names in the config file. Only the two tiers seen on Max accounts are recognized. Everything else is shown as "Enterprise or unknown plan". When a plan has no 5 hour and weekly windows, the Budget (when it has room) and Projection panels say "5 hour and weekly windows: not available for this plan" and add that Pro and Max get them from the playbook statusline. A plan this version cannot tell from Enterprise, such as Pro, gets the same text. On a Max plan with no reading yet they say a reading appears after a session with the playbook statusline. Run `playbook usage doctor` to see which sources exist on a machine.

A manual budget is the UTC calendar month or week (the week starts on `usage.weekStart`), and the spend is the estimated cost from your local history, which is not your bill. Set it with `playbook config set --global usage.budget.monthlyUsd 200`. `0` means no budget.

### What you see in each case

You set no limit for any of these. The rows come from what Claude Code reports on this machine, and the dashboard works out the burn rate, the projection and the time each of 50, 80 and 100 percent is crossed itself. The Burn-up panel charts the most used limit. Nothing is sent anywhere and no login token is read.

| Case | What Claude Code reports | Rows shown | Burn-up chart |
|---|---|---|---|
| A. Pro or Max with windows | The 5 hour and weekly windows, from the statusline or the cache. | One row per window: percent used, reset in, burn per hour, projected percent at reset, crossings. The plan label comes from the tier names. | A chart for a window, drawn from the readings the statusline saved while the window ran, or as a straight line from the window start to the reported percent when there are none, with the projection to the reset and the cutoff lines. |
| B. Max with extra usage on, cap not known | A, and the extra usage flag. | The same as A. The extra usage flag is kept in the plan fields that `--raw` shows. | As A. |
| C. Max with the extra usage cap known | A, and the cache `spend` block. | A, and a dollar row with used, cap, left, reset and crossings. | A, and a monthly dollar chart from this machine's cost, with the reported figure marked and the cutoff lines at the cap. |
| D. No windows, dollar cap known | The cache `spend` block, or the statusline `spend_limit`. | One dollar row. The row is tagged `measured` or `cached`. When a reported figure and this machine's estimate both exist, a line next to the row shows both, with the reported one labeled "reported by Claude". | Monthly spend, the projection to the end of the period and the cutoff lines. |
| E. Cap not reported | Nothing. | One line that says what is missing: a window plan is told to run a session with the playbook statusline, a dollar or unknown plan is told the cap is not reported by Claude Code on this login. Local spend and burn still show. | Spend and projection, and no cutoff lines. A limit of zero is never drawn. |
| F. API key, Bedrock, Vertex or Foundry | Nothing. | As E, and the `usage.budget.*` lines when you set them. | As E, or with the budget as the 100 percent line. |
| G. Reading over 6 hours old | Any. | A statusline reading stays and is tagged `measured, stale`. A cached dollar limit over 6 hours old is used only when no budget is set and is tagged `cached, stale`. A cached window over 6 hours old is dropped. The comparison line says "stale, read N ago". | The monthly dollar chart marks the time of the reported figure. A window chart ends at the last reading, and its saved readings stay as they were. |
| H. Window already reset | Any. | The window is dropped, as Claude Code drops it. | None. |
| I. A rate limit error seen | An `api_error` with status 429 in the Claude Code telemetry. | A line "Claude Code was rate limited at T (HTTP 429)" with the newest time. A 429 counts for the line and the reason for 7 days. When a window plan has no reading and a 429 was seen, the reason is `rate-limited-seen`; the other reasons keep their word. | A `×` marker at each 429 inside the period shown. A 429 before the period starts or at its end is not drawn, and a call that was retried counts once. |

The 429 is the only proof without a token that a cap was hit, so it is drawn and never turned into a limit value. When the reported dollar figure and this machine's estimate both exist, the summary shows both and says whether they differ by at least the larger of 1 dollar and 10 percent of the reported figure.

### The math

For a limit over a period from `start` to `end`, with `used` so far at `now` (all UTC seconds):

- elapsed is `now` minus `start`, clamped to the period, and left is `end` minus `now`, clamped the same way. Before the period starts, elapsed is zero.
- average per hour is `used / elapsed hours`, and per day is `used / elapsed days`. The first day is partial, so days are a fraction of 24 hours, not whole days. With zero elapsed time there is no average (`n/a`, JSON `null`), never a division by zero.
- remaining is `limit` minus `used`, never below zero.
- allowed per hour is `remaining / hours left`, and per day is `remaining / days left`. With no time left there is none.
- projected is `used + average * time left`, the total at the end of the period at the current burn rate. It needs at least one hour of elapsed time, because the first minutes say nothing.
- the state is `limit reached` when `used` is at or above the limit, `will exceed` with the UTC time the limit is reached at the current rate when the projection is above the limit, `on track` when the projection is between 80 percent and 100 percent of the limit, `under` below 80 percent, and `too early to project` before the first hour.

A month has 28 to 31 days (a leap February has 29) and a period ends at the next UTC midnight, so there is no daylight saving. The same math serves the 5 hour and weekly windows, where the limit is 100 percent and the rates are percent per hour and per day.

The panel also shows, for the period on screen (today, this week or this month, this week for other ranges), the average cost per hour and per day against the previous period over the same elapsed span, the same rule as the period comparison.

### Where it shows

- Terminal view: the Budget panel sits under the header and needs 36 rows of terminal. On a shorter screen it is left out, and `--summary` and `--json` have the same numbers. It lists at most two limits, worst first, and says how many more `--json` has. With no known limit it says how to set one.
- `--summary`: a "Limits and burn rate" section. After the limits it adds the amount Claude reports as used next to the local estimate, with its age and a stale marker once it is over 6 hours old (or why the two were not compared), one `<period> crossings` line per limit with the time each of 50, 80 and 100 percent was or will be crossed, and the time of the newest 429 of the last 7 days when there is one, and one line saying why no limit comes from Claude when none does. With a monthly dollar limit it also prints a plain `burn-up` table, one row per day through today with the cumulative local spend and the share of the limit, in ASCII, so a pipe and `NO_COLOR` read it as is. Before the period starts it says `the period has not started`. Days split at midnight in the time zone of the summary, or in UTC when the period came from Claude.
- `--json`: `limits` with `known`, `hint` (when nothing is known), `snapshot_captured_at`, `snapshot_stale`, `plan` (`max_5x`, `max_20x` or `enterprise_or_unknown`), `plan_label`, `windows_note` (null when a window is known), `reason` (why no limit comes from Claude: `no-reading`, `stale`, `cap-unknown`, `windows-reset` or `rate-limited-seen`, else null), `comparison` (null, `skipped` with the reason, or the amount Claude reports and the local estimate with `delta_usd`, `delta_pct`, `disagrees`, `read_at` and `stale`), `cache` (`fetched_at`, `age_seconds` and `fresh`, or null), `limits` (unit, period, origin (`measured`, `manual` or `cached`), used, limit, start, end, remaining, average and allowed per hour and per day, projected, state, `exceeds_at`, `thresholds` (three entries for 50, 80 and 100 percent with `pct`, `state` (`crossed`, `projected`, `beyond` or `unknown`), `at` for the first two and `period_end` for `beyond`) and `series` (for a dollar limit `basis` (`daily`), `days` (`day`, `cumulative_usd` and `percent_of_limit`) and `projected_end_usd`; for a percent window `basis` (`readings` or `line`), `points` (`at` and `percent`) and `projected_end_percent`; both kinds list `rate_limited_at`, the UTC seconds of each 429 in the period)) and `burn` for today, week and month, each with the current and previous period.

The Budget panel ignores the project filter: a limit belongs to the account, not to a repo.

### The Projection panel

The Projection panel says when each limit is reached if you keep going at your current pace, in the time zone shown in the panel title (UTC or local). Each limit gets one line in plain words. A limit in dollars has two paces on that line. The first is the last 24 hours: it follows what you are doing now. When the period started less than 24 hours ago, it uses the spend of the period over the age of the period, so spend from before the period never counts (under one hour of data it says "no pace yet"). The second is the period average since the period began, which is steadier and shows the long view. A limit that is a percentage of a window (the 5 hour and weekly windows of a plan) has the window average only, because the window has no per day cost. Each pace ends with "hit" and a time, "won't hit before it resets", "no hit" (nothing was used in that span) or "limit reached". With no limit known, the panel says how to set one.

### The burn-up panel

The Burn-up panel is on the Charts tab. It plots the spend of the period so far against a limit that Claude Code reported, with a dotted projection at the current pace, a cutoff line for the limit and lines for 50, 80 and 100 percent, each marked where it is crossed or when it will be. The chart uses the most used limit that has a burn-up. A monthly dollar limit is drawn from the daily cost of this machine, and a window in percent from the readings saved in `rate_limits.jsonl`, or as a straight line from the start of the window to the figure Claude Code reported when none were saved. With no limit known the panel says so and charts the spend alone. Nothing here is set by hand: the limit and the used figure come from what Claude Code reports. The math is in `crates/telemetry/src/burnup.rs` and the drawing in `crates/telemetry/src/tui/burnup.rs`.

### The reading history

`rate_limits.jsonl` sits next to `rate_limits.json` in the data folder. Each line is one reading: the time and the two window percentages, for example `{"t":1788400000,"five_hour":55.4,"seven_day":60}`. It holds no account, token or session id. The statusline adds a line only when a percentage changed and at least 300 seconds have passed since the last line, so a busy session writes at most 288 lines a day. Lines older than 8 days are dropped when a line is added, and the file never grows past 256 KiB: the oldest lines go first, and a file that is bigger anyway is ignored by the readers and replaced by the next write. A line that is not valid is skipped. Only the usage views read the file, and the chart of a window uses the lines inside that window. You can delete it at any time; the chart then goes back to the straight line.

Writing it is best effort. A folder that cannot be written, or a failed write, never changes the statusline output or its exit code. The common case reads the first and last line and appends one line, and a test checks that a full file adds no more than 5 milliseconds to a render.

### What is not automatic

- Enterprise and API organization budgets, as above.
- Pro and Max windows need the playbook statusline to be the Claude Code status line. If you use another status line, set a manual budget instead, or wire `playbook statusline` in.
- Headless `--output-format json`, OpenTelemetry and `/usage` do not carry limits in any documented way that a background process can read, so they are not used.

## Telemetry storage

The usage database also holds tables for OpenTelemetry data, created next to the transcript tables when it is opened:

| Table | One row per |
|---|---|
| `telemetry_requests` | API request: session id, request id, model, effort, agent name, skill name, token counts, cost, call duration. |
| `telemetry_tools` | Tool call: tool name, success, duration. |
| `telemetry_errors` | Failed API attempt: status code, attempt number, duration. |
| `telemetry_metrics` | Metric point of four counters: active time, lines of code, commits, pull requests. |
| `session_sources` | Session: which source it reads from, `telemetry` or `transcript`. |
| `session_reconcile` | Session with telemetry: the verdict and the numbers compared. |

Totals never read them, so every total is unchanged, and a session with no row in `session_sources` reads from transcripts. The rows never hold prompts, tool content, paths, emails or user ids. Dropping the tables reverts the feature and loses no transcript data (`telemetry_db::migrate_down`). Telemetry follows [issue 769](https://github.com/pragmatic-engineer/playbook/issues/769): playbook writes Claude Code settings only through `setup`, with consent (see Telemetry by default), and transcripts stay the fallback. Rows are kept for 90 days and 500000 rows per table unless you change the [retention keys](../reference/config-keys.md).

Plain `claude` sessions (not started through `cc`) send nothing to the receiver unless the OpenTelemetry variables are set for them. `playbook setup --telemetry` writes those variables into your Claude Code settings, with your consent, see [Telemetry by default](#telemetry-by-default). Without them, such sessions are read from transcripts.

## Background warm-up

The `usage-warm` hook (SessionStart and SessionEnd, wired by `playbook setup`) keeps the store current without you opening the view. It checks `usage.autoIngest` and a stamp file, then starts `playbook usage warm` detached and returns in a few milliseconds with no output. The child takes `auto-ingest.lock` in the usage directory so one read runs at a time (a lock older than 20 minutes counts as dead), lowers its priority, runs the same resume-safe batched ingest as the usage view, and stops after two minutes. A read that finished writes `auto-ingest.stamp`, and hooks skip while the stamp is younger than `usage.autoIngestMinutes` (default 10). A read that hit the time limit leaves no stamp, keeps its committed batches, and the next hook resumes it. It stores only what ingest already stores and logs nothing. The plugin-disabled pause applies, since no hook runs then.

## The telemetry receiver

`playbook usage collect` runs a local receiver for Claude Code's OpenTelemetry export. It is optional: without it every number comes from transcripts, as before. Playbook starts it for you and keeps it running, see [Checking and running the collector](#checking-and-running-the-collector).

- **Where it listens.** 127.0.0.1 only, JSON protocol only (`http/json`). The port is chosen once (default 47318, or a free one when that is taken on the first run) and recorded; later starts reuse it and fail with a message when another program owns it. An install that already recorded a port (often 4318) keeps it. `--port N` asks for a specific port, fails when it is busy, and replaces the recorded port, so run `playbook setup --only telemetry` afterwards to rewrite the endpoint in Claude Code settings.
- **State.** `collector.json` next to the usage database (owner-only) holds the pid, the port and a bearer token. The collector holds an exclusive lock on `collector.lock` for its whole life and writes its pid and version into that file. A second start finds the lock busy, says so and exits 0. A stale file (dead pid) is replaced. The token and port survive restarts, so a session started earlier keeps sending to the right place with the right token.
- **Idle shutdown.** A collector you start by hand stops after 600 seconds with no telemetry request (`--idle-seconds`, 0 means never) and clears the pid. The collector that playbook starts runs with 0 and does not idle out. Only a request that was read counts: a bare connect, such as the liveness probe `cc` makes or `playbook usage status`, does not reset the timer, and a test polls status while the collector idles out to prove it. A session that is still open when the collector stops loses its export from then on and is read from transcripts, so use `--idle-seconds 0` for a collector that should outlive quiet periods.
- **Limits.** Headers 16 KiB, body 1 MiB (also when chunked), 50 requests per second (then 429), 5 seconds to send a request, 64 open connections (then 503). Each connection has its own thread, and one writer thread stores the rows in batches. Content type must be `application/json`, a compressed body is refused, and the bearer token is required. Paths: `/v1/logs` and `/v1/metrics` are stored, `/v1/traces` is accepted and dropped.
- **What is stored.** `api_request`, `tool_result` and `api_error` events, keyed by session id and request id, tool use id, or event sequence, and four delta counters from the metrics (see Active time and counters), keyed by their interval, so a redelivered batch changes nothing. An allowlist of attribute keys is read (event name, session id, request id, model, effort, agent name, skill name, tool name, tool use id, success, duration, status code, attempt, event sequence, the four token counts, cost). Every other attribute, including prompts, tool content, file paths, emails, user and organization ids and resource attributes, is never read. Kept names must be plain ASCII (letters, digits and `. _ - : /`), else they are dropped.
- **Data loss.** Claude Code may drop telemetry sent while no receiver listens. Such a session is read from transcripts.

## Telemetry by default

Telemetry is the default source for the per agent, skill and latency views, and transcripts stay the authority for every total. `usage.telemetry` is `auto` unless you set it to `off`. What that does depends on the client:

| Client | What happens |
| :- | :- |
| `playbook cc` (and `playbook ccd`) | Sets the variables for that session only, when you have set no telemetry yourself and a collector runs. Like most playbook commands, it starts the collector first when your user settings point at it. |
| Plain `claude`, the IDE, the desktop app | If you answered yes to the setup question (the default) or ran `playbook setup --only telemetry`, the variables are in the `env` block of `~/.claude/settings.json`, which these clients read. The `session-init` hook, the prompt hooks and most playbook commands start the collector when it is down. With the hooks part off, only a playbook command starts it. Otherwise they are read from transcripts. |

Playbook never changes your Claude Code settings on its own. Only `setup` writes them, after you say yes (`--yes` counts, and the question defaults to yes), and only when no telemetry variable exists in your environment or in any settings file (user, project, local, managed). The one exception is a lone off switch (`CLAUDE_CODE_ENABLE_TELEMETRY` set to an off value and no other telemetry variable anywhere) in `~/.claude/settings.json`: a yes to the question replaces it, and `playbook uninstall` puts the old value back. Any other value you set is never overridden: if you turned telemetry off elsewhere, or point it at your own endpoint, playbook leaves it and reads transcripts. `setup` records what it wrote in `~/.config/playbook/telemetry-env.json`, and `playbook uninstall` removes exactly those keys, unless you changed them since. The shared settings template no longer pins `CLAUDE_CODE_ENABLE_TELEMETRY` to `0`; an install made earlier keeps that value until you remove it, and it counts as your setting.

`playbook usage status` and the usage view's footer say which case you are in:

- `telemetry: on, collector running`
- `telemetry: on, collector not running`, with the fix: run any playbook command or start a session.
- `telemetry: off (disabled in your Claude Code settings, using transcripts)`, with the fix: run `playbook setup --telemetry`, which replaces an off switch that stands alone in `~/.claude/settings.json`, or remove `CLAUDE_CODE_ENABLE_TELEMETRY` from the `env` block of your settings yourself when it is set anywhere else.
- `telemetry: off (set up by you for another endpoint, using transcripts)`
- `telemetry: off (usage.telemetry is off, using transcripts)`, with the fix: `playbook config set usage.telemetry auto`.
- `telemetry: off (not configured)`, with the fix: `playbook setup --telemetry`, or start sessions with `cc`.

If the collector is not running when a session starts, Claude Code may drop what it sends, and that session is read from transcripts. After a reboot or a logout this lasts until the next playbook command or hook starts the collector.

## Telemetry privacy

- **What is stored.** Only `api_request` rows with the session id, request id, model, effort, agent name, the four token counts, the cost and the call duration, plus one row per tool call (tool name, outcome, duration), per failed API attempt (status code, attempt number, duration) and per metric point of active time, lines of code, commits or pull requests (session id, type, interval, value). Prompts, tool content, file paths, emails, user, account and organization ids, and resource attributes are dropped when the receiver parses a request and never reach the database.
- **What `cc` sets.** With `usage.telemetry` at `auto`, a running receiver and no telemetry setting of yours, `cc` sets `CLAUDE_CODE_ENABLE_TELEMETRY`, `OTEL_LOGS_EXPORTER=otlp`, `OTEL_METRICS_EXPORTER=otlp`, the traces exporter to `none`, the `http/json` protocol, the loopback endpoint, an `Authorization` header with the receiver's token and `OTEL_METRICS_INCLUDE_ACCOUNT_UUID=false`, in the session's environment only. It never turns on `OTEL_LOG_USER_PROMPTS`, `OTEL_LOG_TOOL_DETAILS`, `OTEL_LOG_TOOL_CONTENT` or `OTEL_LOG_RAW_API_BODIES`.
- **What it never does.** `cc` never writes Claude Code settings and adds no `--settings` flag. Only `playbook setup --telemetry`, with your consent, writes the same variables (and nothing else) into the `env` block of your settings. If any telemetry variable is already set in your environment or in a settings `env` block (user, project, local or managed), or such a file cannot be parsed, it sets nothing, so your own configuration always wins. `setup` has one exception, described above: a yes replaces a lone off switch in the user settings.
- **How long it is kept.** 90 days and 500000 rows per table by default (`usage.telemetryRetentionDays`, `usage.telemetryMaxRows`), pruned automatically and by `playbook usage prune`.
- **Where it goes.** The receiver listens on 127.0.0.1 only and the store file is private to you. `usage.telemetry` set to `off` stops `cc` from touching the environment at all.

## Choosing a source per session

After each ingest, `usage::reconcile` compares every session that has telemetry rows with its transcript totals and records one verdict in `session_reconcile`. Transcripts stay the authority, and every total (`--json`, `--summary`, the terminal view) still sums the transcript rows priced by playbook, so nothing is counted twice.

- **Complete.** Tokens within 1 percent and cost within 0.5 percent. The session's source becomes `telemetry`, which makes its per agent split a valid split of the verified total.
- **Partial.** Telemetry has fewer tokens than the transcript (the receiver started late or missed a batch). The source stays `transcript`.
- **Disagree.** Telemetry has more tokens, or the cost differs. The source stays `transcript` and the verdict holds a note with the gap.
- **No telemetry rows.** Nothing is recorded and the session reads from transcripts, so old history needs no backfill.

The pass is idempotent, and a failure leaves transcripts as the source.

## Cost per agent

The Agents view (`4`, then `v`) groups `telemetry_requests` by agent name for the range (`query::load_agents`). The main session has no agent name and shows as `main`. The title note `source telemetry N% of spend` is the telemetry cost over the transcript cost of the range, capped at 100 percent. Only telemetry knows the agent, so a range with no telemetry rows shows the `Telemetry off` message instead of numbers. Totals are unchanged.

## Cost per skill

The Skills view (`4`, then `v` twice) groups `telemetry_requests` by skill name (`query::load_skills`). A short headless trial on Claude Code 2.1.293 (Haiku, under one cent) checked the attribute: the `api_request` event carries `skill.name` on the requests made while a skill is active (the request that follows the skill load), and requests before it carry none. So the skill row is the cost of the requests the skill shaped, not a share of the whole session. The separate `skill_activated` event redacts project and user skill names to `custom_skill`, but `api_request` showed the real name in the trial, and a plugin skill was checked later, see below. Names are stored only if they are plain ASCII, like every other name. Requests with no skill are left out of the view, so the rows are a share of telemetry cost and do not sum to the total. Tables made before this slice gain a `skill_name` column on open.

Plugin skills (a second trial on 2.1.293, Haiku, about 0.006 USD, a throwaway plugin loaded with `--plugin-dir`): the `api_request` event and the cost and token metrics do carry `skill.name` for a plugin skill, but a plugin that is not an official marketplace one is reported with the name `third-party` for both `skill.name` and `plugin.name`, and `skill_activated` shows `custom_skill` with `skill.source` `plugin`. The real name only appears when tool details are logged, which also exposes paths and commands, so playbook does not ask for it. The view therefore shows all such skills as one row, `plugin skills (names hidden)`. Skills of official plugins were not tried.

## Tool latency and health

Two more views of the projects panel (`4`, then `v` three and four times) read the telemetry tables `telemetry_tools` (one row per tool call: tool name, success, duration) and `telemetry_errors` (one row per failed API attempt: model, status code, attempt number, duration), plus the new `duration_ms` of `telemetry_requests`. Tables made earlier gain the column on open and the new tables are created next to the others.

- **Tools** lists each tool with calls, error rate, average and slowest call. For today, the week and the month it adds the previous period's average and the change over the same elapsed span, like the Models panel. The call and the error text are never read, only the name, the outcome and the time.
- **Health** shows API errors with their rate, retries, tool errors with their rate, active time (busy time when no metric arrived), API requests, lines changed, commits and pull requests, each against the previous period, and the cache hit rate. The cache hit rate is cache read tokens over all input side tokens and comes from the transcripts, so it shows with telemetry off too. Only a few rows fit in a small terminal and the table scrolls.
- **Retries.** An API error with attempt 2 or more is a failure that came after a retry, and that is what `Retries` counts. Claude Code does not export a plain retry counter in the log events. A 404 for an unknown model was seen with attempt 1, so it is an error and not a retry. `api_retries_exhausted` did not appear in a trial and is not read.
- **Active time and counters.** The receiver also reads four delta counters from `/v1/metrics`: `active_time.total` (user and cli seconds, added up), `lines_of_code.count`, `commit.count` and `pull_request.count`. Only the `session.id` and `type` attributes are read, and a cumulative sum is skipped because adding it up would count it again. A point is keyed by session, metric, type and its exact interval, so a redelivered point is stored once (table `telemetry_metrics`). Claude Code sends metrics every 60 seconds by default, so the last minute of a session can be missing. The Health view shows Active time when any was collected and otherwise Busy time, the sum of API call time and tool call time, which is a floor and not the metric. Cost and token metrics are not stored, the `api_request` events carry them with a key.
- **Sources and empty state.** Every figure except the cache hit rate is telemetry only, so the panel says `Telemetry off` when the range has no telemetry rows, and the title note reads `source telemetry`. Periods are UTC, the same windows as the rest of the view.
- **Not verified.** Tool names of MCP tools and plugin tools, which Claude Code may redact without the tool details flag, and the attempt numbers of a real overload (an unknown model was the trial case).

## Checking and running the collector

`playbook usage status` prints whether a collector is running (pid, port, start time and idle limit), how many telemetry rows are stored (API requests, tool calls, API errors, metric points), the retention limits and the time of the newest event with its age. `--json` prints the same fields. Like most playbook commands it can start the collector first (see below). It creates no database when there is none, never prints the bearer token, and exits 0 whether or not a collector runs, so a script reads the `running` field. When no collector is running it shows the last port it used.

**Starting it for you.** When the settings that `setup` wrote point at the collector, playbook starts `playbook usage collect --idle-seconds 0` itself, detached in its own session, from three places: the `session-init` hook (wired by the hooks part of `setup`), the two hooks wired only on `UserPromptSubmit` (`auto-model-detect` and `context-hygiene`), and most playbook commands, `cc` included. `--version`, `version`, `doctor`, `update`, `uninstall`, `migrate`, `setup` and its old name, `state`, the tool hooks, `statusline`, `json`, `commands` and `usage collect`, `warm`, `prune` and `stop` start nothing, nor does a headless run (`PLAYBOOK_HEADLESS` or `CI`) or `usage.telemetry` set to `off`. The collector holds an exclusive lock on `collector.lock` for its whole life, so a running one costs a starter one open and one `try_lock`, and parallel starts leave one collector. A spawn holds the next one off for 10 seconds (`collector.spawned`). The session hook waits at most 1.2 seconds for a new one and replaces a collector that holds the lock but does not answer; a prompt or a command waits for nothing. A collector from another playbook version is asked to stop and replaced. `playbook usage stop` stops it. The collector writes its messages to `collector.log` next to `collector.json`. For a `cc` session with no telemetry setting of yours, `cc` points the session at a collector that already runs.

**After a reboot.** Nothing starts the collector at login. Telemetry from sessions that run before the next playbook command or hook is dropped, and those sessions are read from transcripts. A plain `claude`, the IDE or the desktop app with the hooks part off starts nothing. Earlier setups installed a launchd agent or a systemd user unit for this. The `collector-service-removed` migration removes it, and `playbook doctor` warns while a service file is still installed. See [ADR 0048](../adr/0048-collector-daemon-started-by-commands.md).

To run it yourself, start it in a detached tmux session. On Windows playbook does not start it, so this is the only way there. The idle limit is off here so it does not stop in a quiet hour:

```
tmux new-session -d -s playbook-collect 'playbook usage collect --idle-seconds 0'
playbook usage status
tmux kill-session -t playbook-collect
```

`playbook usage collect` stays in the foreground, so any tool that runs a command in the background can run it the same way (`nohup playbook usage collect --idle-seconds 0 &` also works). A second start reports the running collector and exits, so it is safe to put the command in a shell profile or a tmux startup script.
