# Usage view

`playbook usage` shows how many tokens and how many dollars your sessions use, split by day, week, model, repo, branch, effort, account, skill and agent. It reads Claude Code's own session history, so it needs no setup and no new instrumentation.

## Commands

| Command | What it does |
|---|---|
| `playbook usage` | Opens the terminal view. When stdout is not a terminal, prints the text summary instead. |
| `playbook usage --summary` | Reads anything new, then prints the text summary and exits. With `--range` it covers only that range and says which dates. For `today`, `week` and `month` it ends with the [period comparison](#period-comparison). |
| `playbook usage --json` | Prints the totals, the groups and the live view as JSON, for scripts. For `today`, `week` and `month` it adds a `comparison` object. |
| `playbook usage --range <name>` | Sets the range for the terminal view, `--json` and `--summary`. See [Ranges](#ranges). The default is `week` for the terminal view, `30d` for `--json` and all time for `--summary`. |
| `playbook usage ingest` | Reads anything new into the store. Deprecated, use `--summary` or `--json`. |

Every command that shows data reads new history first, so you never need to ingest by hand.

## The terminal view

The view is styled after [btop](https://github.com/aristocratos/btop), with btop's default theme as the default look: a black ground, rounded boxes with the key hint and title in the top border (`╭─┤¹Spend per day├──╮`), a gradient on the graphs and the meters, and a highlighted selected row. Each box has its own outline color, as in btop: spend uses the cpu green, tokens the net purple, models and projects the mem olive, sessions and messages the proc red.

The view needs a screen of at least 80 columns by 24 rows. Below that it says so and draws nothing else. Resizing needs no handling because every redraw reads the current size. It shows six panels under a header with the range, the cost, the tokens and the messages.

For `today`, `week` and `month` the header is a comparison: it says which two spans are compared and that they cover the same elapsed span (see [Period comparison](#period-comparison)), then shows cost, tokens and messages for both with the change in amount and percent (`new` when the previous value was zero). Increases are yellow and decreases green, and the sign says it without color. The other ranges show their own totals and say `(no previous period to compare)`. Everything is UTC and the header says so, with the first day of the week when the range is a week.

The view re-reads the store every 5 seconds (set `usage.refreshSeconds`, 1 to 3600, to change it) and the footer shows when it last did (`updated 01:46:40 UTC, every 5s`), or why the last read failed. A refresh swaps in the new snapshot without touching the chart cursor, the focus, the sorts, the spend view, a filter being typed or the selected rows: each table finds its selected row again by what it is (a model, a project, a session, a day), so a row that moved in the order stays selected, and the scroll follows it. A short footer keeps the time and drops the interval.

The panels:

| Key | Panel | Shows |
|---|---|---|
| `1` | Spend per day, USD | A bar chart of the cost for each UTC day in the range, with axes (below). `v` steps it through three views: the total, the same bars stacked by model, and a table with a row for each day (see below). |
| `2` | Tokens per day | The same for tokens, in the net gradient. |
| `3` | Models | Messages, tokens and cost per model, with a swatch in the model's color. For `today`, `week` and `month` it also shows the cost in the previous period and the change in amount and percent, for every model used in either period (a model that only the previous period used reads `$0.00` now, and one with no spend before reads `new`). Other ranges show a share meter instead. `s` also sorts by the previous cost and the change. |
| `4` | Projects | The same, per repo. |
| `5` | Live sessions | Sessions with a message in the last 15 minutes, with state, cost and age. |
| `6` | Sessions per day | Every session in the range, grouped by UTC day with the newest day first. Each row has the short session id, project, main model, messages, tokens and cost for that day, and a session that runs past midnight has a row for each day. The day is named on the first row of its group, and `s` orders the sessions inside each day. `Enter` filters the view to the session. |

### The charts

The two charts are bar charts drawn with eighth blocks, one bar per UTC day, with idle days at zero and the days still to come in a week or month in progress left empty. Today charts the last seven days, since one bar says nothing, and the title says so (`Spend per day, USD, 7 days`).

- **y axis.** Labels at the bottom, middle and top, with a blank column between them and the box, in dollars or tokens with units (`$0`, `$2.5`, `$250`, `$2.5K`, `500K`, `2.5M`). The top is a round value above the peak (1, 1.2, 1.6, 2, 2.5, 3, 4, 5, 6 or 8 times a power of ten), so the labels are round and the middle is half the top. Faint dotted guides mark the top and the middle.
- **x axis.** The first, middle and last date under the bars (with the year when it fits), and a row of weekday letters or names when the panel is tall enough and every bar is wide enough for a letter and a gap (a month in a narrow panel has none). Today's weekday is bold and underlined, the peak day's is yellow, and the cursor's is highlighted.
- **Totals line.** The total, the peak (with its date) and the average per day that has happened, for the range.
- **Cursor.** `Left` and `Right` move it one bar, `Home` and `End` jump to the first and last bar while a chart has the focus, and with the mouse on, hovering or clicking a bar puts it there. A readout line gives the day, its cost, messages, tokens and top model with its share (the token chart leads with tokens). It starts on today, and both charts share it. Changing the range resets it.
- **Legend.** The bar color follows its height on the axis along the theme gradient (green to red for cost), and the legend in the bottom border says so. The mono theme has no colors and no legend.
- **Fill.** A week or more of bars always fills the width of the plot, and fewer bars are never wider than 16 columns.
- **Wide ranges.** When the range has more days than the chart has columns, days are added up into bars of equal length (the title says `Spend per 3 days`), so nothing is cut off. The axis, readout and cursor then speak of those bars.
- **Small panels.** Rows go in this order when the panel is short: weekday letters, totals line, readout, dates, so the plot keeps three rows. The y axis goes when the panel is narrower than 16 columns, leaving the bars.

### Daily cost and cost per model

`v` switches the spend panel between three views of the same days.

- **Total.** One color per bar along the gradient, as above.
- **By model.** Each bar is stacked by model, the biggest model at the bottom, and the legend in the bottom border names them (`■ opus-5 ■ sonnet-5 +1` when they do not all fit). The edge between two models is exact to an eighth of a cell: a cell that holds an edge is drawn as a block in the lower model's color on the upper model's. Without color (the mono theme) the models are told apart by pattern (`█ ▓ ▒ ░ ▚ ▞ ▛ ▟`).
- **Daily.** A table with a row for each day up to today, newest first: date with weekday, messages, tokens, cost and the model that cost the most that day. `s` and `S` sort it by any column, the selection and the wheel work as in the other tables, and a click selects a row. Today is bold and underlined, the peak day is bold and yellow, and the legend in the bottom border says so. Idle days stay as zero rows, so it reads as a calendar.

A model keeps one color everywhere: its swatch in the Models panel, its band in the stacked bars, its legend entry and its name in the daily table. The color comes from a hash of the model name, so it stays the same when the range changes. Two models that want the same color in one range get the next free one, and with more than eight models the colors are shared.

Keys:

| Key | Action |
|---|---|
| `1` to `6`, `Tab`, `Shift-Tab` | Focus a panel. The focused panel's hint is a badge. |
| `Left`, `Right` | Move the chart cursor one day (or one bar). `Home`, `End`, `g`, `G` jump to the ends while a chart has the focus. |
| `Up`, `Down`, `j`, `k`, `PageUp`, `PageDown`, `Home`, `End`, `g`, `G` | Move the selected row of the focused table. |
| `v` | Step the spend panel: total, by model, daily table. |
| `s` | Sort the focused table by the next column. A marker in the header shows the column and the direction. Text columns start ascending and number columns descending. |
| `S` | Reverse the sort. |
| `Enter` | Filter the whole view to the selected row: a model, a project or a session id. `Enter` on that row again lifts the filter. |
| `r` | Step between today and this week, the two default views. From any other range it goes to today. |
| `d`, `w`, `m` | Jump to today, this week, this month (month to date). Each is shown against the previous period. |
| `W`, `M`, `a` | Jump to last week, last month, all time. These are plain ranges with no comparison. |
| `/` | Type a filter, a case insensitive substring of model, repo, branch or session id. `Enter` applies it, `Esc` cancels, `Backspace` edits. |
| `c` | Clear the filter. |
| `t` | Cycle the theme: btop, dark, light, mono. |
| `x` | Turn mouse reporting on or off. Turn it off to select text with the mouse. |
| `q`, `Esc`, `Ctrl-C` | Quit. |

The mouse is optional and on by default: a click focuses a panel and selects the row under it, the wheel moves the selection of the panel under the pointer, and hovering or clicking a chart bar moves the cursor.

### Ranges

Every range is a set of whole UTC days, shown in the header with its name and exact dates (`last week 2026-08-24 to 2026-08-30 UTC`). The time zone is UTC everywhere in the view, and the JSON, so there is no daylight saving: a day is always 86,400 seconds and a week is always seven days.

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

`today`, `week` and `month` are periods: each is compared with the one before it (yesterday, last week, last month) over the same elapsed span, so a Thursday afternoon is never set against a whole week. The code is `src/usage/period.rs`.

- The current span runs from the start of the period through the second of `now`. The previous span starts at the start of the previous period and lasts as long, but never runs past that period, so on March 31 the month so far is compared with all of a 28 day February. The header and `--summary` state the elapsed span.
- Everything is UTC seconds. A week starts at 00:00 UTC on the day `usage.weekStart` names, and an event at 23:59:59 UTC belongs to the earlier day whatever your local clock says.
- Cost, tokens and messages each get the amount of change and the percent. The percent is `n/a` (JSON `null`) when the previous value was zero.
- `--json` adds `comparison`: `period`, `span_seconds`, `current` and `previous` (`start`, `end`, `start_date`, `end_date`, totals), `change` for the three numbers, and `models` with each model's current and previous group.

While you type a filter every key is text, so `q` and `r` do not quit or change the range until you press `Enter` or `Esc`.

### Themes

| Theme | Look |
|---|---|
| `btop` | The default. btop's built-in default theme: black ground, `#cc` text, `#ee` titles, `#b54040` key hints, `#6a2f2f` selected row, per box outline colors, and three point gradients (start, mid, end) for the graphs and meters. |
| `dark` | The terminal's own ground with single colors. |
| `light` | The same with darker colors for a white ground. |
| `mono` | No color. The selected row is shown in reverse video. |

The theme comes from the `usage.theme` config key (`btop`, `dark`, `light`, `mono`; see [config keys](../guides/04-config-keys.md)) and `t` changes it for the session. `NO_COLOR` set to anything but empty forces `mono` and locks it.

Colors follow the terminal. With `COLORTERM=truecolor` or `24bit` the btop values are drawn exactly. Otherwise, when `TERM` names `256color`, each value goes to the nearest of the 256 color cube and gray ramp, and with neither it goes to the nearest of the 16 basic colors. The values come from btop's `Default_theme` and the code in `src/usage/tui/theme.rs` says so. btop is Apache 2.0 like playbook, and the attribution is in `NOTICE`.

The code is in `src/usage/tui/`. `data.rs` builds one immutable snapshot from the store, `worker.rs` refreshes it on a thread so a slow read never blocks a key, `app.rs` holds the state and the keys, `view.rs` and `panels.rs` draw (`chart.rs` is the bar chart with its axes and cursor, `scale.rs` the round axis values, `sort.rs` the panel list and the row sorts), and `theme.rs` holds the colors. The numbers come from `src/usage/query.rs`, the same layer the web JSON uses, so the same range gives the same totals in both. The reasons for ratatui and the measured binary cost are in [ADR-0020](../adr/0020-usage-terminal-ui.md).

## Where the data comes from

The source is `~/.claude/projects/**/*.jsonl`, the session transcripts Claude Code already writes. Per assistant message they carry the model, the effort, the working directory, the git branch and the token counts (input, output, cache write, cache read). Skill and agent calls appear as `tool_use` entries.

Four details are worth knowing:

- **One message spans several lines.** Each content block of a message is its own transcript line, and each line repeats the same usage. Counting lines would roughly double every number, so events are deduped by message id (and by tool call id).
- **Transcripts have no cost field.** Cost is computed from a per-model price table in `src/usage/pricing.rs`, copied by hand from the published pricing page (the source and date are in the file). Input, output, cache reads and both cache write lifetimes (five minute and one hour) are priced separately, and the cost is worked out each time you read the data, so a price fix applies to old events without a re-ingest. It is still an estimate and can drift from published prices. A model that is not in the table is never guessed: its messages count as $0, and the summary and the JSON (`totals.unpriced_messages`, `totals.unpriced_models`) say how many and which models.
- **The repo is the real repository, not the last folder.** The working directory of a worktree or an agent folder would otherwise show up as its own repo. `src/usage/repo.rs` maps a directory to its repo: first by path (`.claude/worktrees/<name>`, `.git/...`, and playbook's own `repos/<owner>/<repo>` storage, which still works after the directory is deleted), then by asking git for the shared git dir, then by the folder's own name.
- **The account is not in the transcripts.** It is read once per ingest from `oauthAccount.emailAddress` in `~/.claude.json` and tagged onto every event. If you are not signed in, it is `unknown`.

## Storage

One SQLite database at `~/.config/playbook/usage/usage.db`, not tied to any repo, because spend only makes sense as a total. Events stay there after Claude Code or `ccc` prunes old sessions. Connection setup follows `src/gate/db.rs`: `busy_timeout` first, then WAL mode, because a refresh reads while `ingest` writes from other processes.

The first ingest after upgrading from an older version runs a one-time repair (`src/usage/backfill.rs`): it rescans the transcripts and corrects the repo and the cache lifetime split on rows stored earlier. A row whose transcript is gone keeps its old repo and is priced as five minute cache.

Ingest keeps a watermark per source (the newest timestamp it has read). It skips transcript files last modified before the watermark, so a repeated ingest is cheap. The watermark is inclusive and events are deduped by id, so re-reading the newest event is harmless. Each ingest runs in one `IMMEDIATE` transaction: the rows and the watermark commit together or not at all, and a second ingest waits for the first instead of failing.

## Adding another agent

`UsageSource` (`src/usage/mod.rs`) is the extension point. A source returns normalized `UsageEvent` and `ToolInvocationEvent` values newer than a watermark. Aggregation, storage, the summary and the terminal view never see an agent's own file format, so a second source only has to implement the trait. Two sources exist today: Claude Code (`src/usage/claude_code.rs`) and Codex (`src/usage/codex.rs`). Every stored event carries an `agent` column (`claude-code` or `codex`, old rows default to `claude-code`).

The Codex source reads `~/.codex/sessions/**/rollout-*.jsonl` read-only and is skipped silently when that directory is missing. Format source: openai/codex `codex-rs/protocol/src/protocol.rs` (`TokenCountEvent`, `TokenUsageInfo`, `SessionMeta`, `TurnContextItem`). The model, effort and cwd come from `turn_context`, the session id, cwd and branch from `session_meta`, and tokens from `event_msg` records of type `token_count`. One event is one change of the cumulative `total_token_usage.total_tokens`, taking its tokens from `last_token_usage`, and its id is `codex:<session>:<timestamp>:<total>`, so re-reads dedupe. Cached input is reported as cache reads. OpenAI models are not in the price table, so Codex events show as unpriced rather than guessed. Compressed `.jsonl.zst` rollouts are not read, and the account is `unknown` because rollouts carry none.

## Tests

Tests run against hand-written transcripts in `tests/fixtures/usage/` and a scratch `HOME`, never your real `~/.claude`.

Terminal view tests render into a `TestBackend` buffer, with no real terminal. `src/usage/tui/snapshots.rs` compares whole screens at fixed sizes with the text files in `tests/fixtures/usage_tui/`; after an intended layout change, regenerate them with `UPDATE_SNAPSHOTS=1 cargo test --lib usage::tui::snapshots` and review the diff. `src/usage/tui/parity.rs` checks that, for every range, the view reports the same totals and model, repo and day groups as the web JSON, and again for events on both sides of every week and month edge (a week that spans a month boundary, a Sunday week start, a leap February). It also checks that the comparison in the view and in `--json` agrees with dates worked out by hand: the same elapsed span in the previous period, the second before and after midnight UTC, and a Sunday week start. The period math itself is tested in `src/usage/period.rs`. The snapshots cover 80x24, 100x30 and 160x50, the daily table and the stacked view at those sizes, and a week range. The scale math, the chart geometry and every range's bounds have their own unit tests, and a chart draw test runs the chart at every size from nothing up to 70x22. The fixtures are synthetic: no real project, model spend or session ever goes into a snapshot.

## Limits

- Cost is an estimate (see above). It is the price at the published API rate, not what a subscription plan charges you.
- It reads one machine's local history. It does not combine machines or users.
- Live shows what the transcripts record, so a session that is thinking but has not written a message yet does not appear until it does.
- The terminal view needs a real terminal of 80x24 or more. Use `--json` or `--summary` where there is none.
- No LLM call is built in. The output is plain text and JSON, so the agent already running your session can read it and suggest where to cut cost.

## Telemetry storage (first slice)

The usage database also holds two tables for OpenTelemetry data, `telemetry_requests` (one row per API request: session id, request id, model, effort, agent name, token counts and cost) and `session_sources` (which source a session reads from, `telemetry` or `transcript`). They are created next to the transcript tables and nothing reads them yet, so every total is unchanged. A session with no row in `session_sources` reads from transcripts. The rows never hold prompts, tool content, paths, emails or user ids. Dropping the two tables reverts the feature and loses no transcript data (`telemetry_db::migrate_down`). Telemetry follows [issue 769](https://github.com/pragmatic-engineer/playbook/issues/769): playbook never writes Claude Code settings, and transcripts stay the fallback.

## The telemetry receiver

`playbook usage collect` runs a local receiver for Claude Code's OpenTelemetry export. It is optional: without it every number comes from transcripts, as before.

- **Where it listens.** 127.0.0.1 only, JSON protocol only (`http/json`). The default port is 4318. If that is taken it picks a free one and records it. `--port N` asks for a specific port and fails when it is busy.
- **State.** `collector.json` next to the usage database (owner-only) holds the pid, the port and a bearer token. A second start finds the live collector and says so. A stale file (dead pid) is replaced. The token and port survive restarts, so a session started earlier keeps sending to the right place with the right token.
- **Idle shutdown.** It stops after 600 seconds with no request (`--idle-seconds`, 0 means never) and clears the pid.
- **Limits.** Headers 16 KiB, body 1 MiB (also when chunked), 50 requests per second (then 429), 5 seconds to send a request. Content type must be `application/json`, a compressed body is refused, and the bearer token is required. Paths: `/v1/logs` is stored, `/v1/metrics` and `/v1/traces` are accepted and dropped.
- **What is stored.** Only `api_request` events, keyed by session id and request id so a redelivered batch changes nothing. An allowlist of attribute keys is read (event name, session id, request id, model, effort, agent name, the four token counts, cost). Every other attribute, including prompts, tool content, file paths, emails, user and organization ids and resource attributes, is never read. Kept names must be plain ASCII (letters, digits and `. _ - : /`), else they are dropped.
- **Data loss.** Claude Code may drop telemetry sent while no receiver listens. Such a session is read from transcripts.
