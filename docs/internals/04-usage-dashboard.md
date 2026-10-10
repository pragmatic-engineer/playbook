# Usage dashboard

`playbook usage` shows how many tokens and how many dollars your sessions use, split by day, week, model, repo, branch, effort, account, skill and agent. It reads Claude Code's own session history, so it needs no setup and no new instrumentation.

## Commands

| Command | What it does |
|---|---|
| `playbook usage` | Opens the terminal view. When stdout is not a terminal, prints the text summary instead. |
| `playbook usage --summary` | Reads anything new, then prints the text summary and exits. |
| `playbook usage --json` | Prints the totals, the groups and the live view as JSON, for scripts. |
| `playbook usage --range <30d\|60d\|90d\|month\|all>` | Sets the range for the terminal view and `--json`. The default is `30d`. |
| `playbook usage --web` | Opens the web dashboard. Deprecated, removed in v0.22.0. |
| `playbook usage ingest` | Reads anything new into the store. Deprecated, use `--summary` or `--json`. |
| `playbook usage dashboard` | Starts the dashboard server and opens it. Deprecated, use `--web`. |
| `playbook usage dashboard stop` | Stops the server. |

Every command that shows data reads new history first, so you never need to ingest by hand.

The web dashboard is for macOS and Linux. The server prints two addresses: `http://127.0.0.1:<port>`, which it always opens, and `http://playbook.localhost:<port>`, which works where your system resolves `*.localhost` (Chrome on macOS does).

## The terminal view

The view is styled after [btop](https://github.com/aristocratos/btop), with btop's default theme as the default look: a black ground, rounded boxes with the key hint and title in the top border (`╭─┤¹Spend per day├──╮`), a gradient on the graphs and the meters. Each box has its own outline color, as in btop: spend uses the cpu green, tokens the net purple, models and projects the mem olive, sessions and messages the proc red.

The view needs a screen of at least 80 columns by 24 rows. Below that it says so and draws nothing else. Resizing needs no handling because every redraw reads the current size. It refreshes about every two seconds. It shows six panels and a header with the range, the total cost, the message count and the token count:

| Key | Panel | Shows |
|---|---|---|
| `1` | Spend per day | A braille graph of the cost for each UTC day in the range, idle days at zero, colored by value from the cpu gradient (green, yellow, red). The peak and the date range sit in the border. |
| `2` | Tokens per day | The same for tokens, in the net gradient. |
| `3` | Models | Messages, tokens, cost and a share meter per model. |
| `4` | Projects | The same, per repo. |
| `5` | Live sessions | Sessions with a message in the last 15 minutes, with state, cost and age. |
| `6` | Recent messages | The newest messages with time, project, model, tokens and cost. |

Keys:

| Key | Action |
|---|---|
| `1` to `6`, `Tab`, `Shift-Tab` | Focus a panel. The focused panel's hint is a badge. |
| `r` | Step the range: 30d, 60d, 90d, month, all. |
| `/` | Type a filter, a case insensitive substring of model, repo, branch or session id. `Enter` applies it, `Esc` cancels, `Backspace` edits. |
| `c` | Clear the filter. |
| `t` | Cycle the theme: btop, dark, light, mono. |
| `q`, `Esc`, `Ctrl-C` | Quit. |

While you type a filter every key is text, so `q` and `r` do not quit or change the range until you press `Enter` or `Esc`.

### Themes

| Theme | Look |
|---|---|
| `btop` | The default. btop's built-in default theme: black ground, `#cc` text, `#ee` titles, `#b54040` key hints, per box outline colors, and three point gradients (start, mid, end) for the graphs and meters. |
| `dark` | The terminal's own ground with single colors. |
| `light` | The same with darker colors for a white ground. |
| `mono` | No color. |

The theme comes from the `usage.theme` config key (`btop`, `dark`, `light`, `mono`; see [config keys](../guides/04-config-keys.md)) and `t` changes it for the session. `NO_COLOR` set to anything but empty forces `mono` and locks it.

Colors follow the terminal. With `COLORTERM=truecolor` or `24bit` the btop values are drawn exactly. Otherwise, when `TERM` names `256color`, each value goes to the nearest of the 256 color cube and gray ramp, and with neither it goes to the nearest of the 16 basic colors. The values come from btop's `Default_theme` and the code in `src/usage/tui/theme.rs` says so. btop is Apache 2.0 like playbook, and the attribution is in `NOTICE`.

The code is in `src/usage/tui/`. `data.rs` builds one immutable snapshot from the store, `worker.rs` refreshes it on a thread so a slow read never blocks a key, `app.rs` holds the state and the keys, `view.rs` and `panels.rs` draw (`graph.rs` is the braille graph, `panel.rs` the panel list), and `theme.rs` holds the colors. The numbers come from `src/usage/query.rs`, the same layer the web JSON uses, so the same range gives the same totals in both. The reasons for ratatui and the measured binary cost are in [ADR-0020](../adr/0020-usage-terminal-ui.md).

## Where the data comes from

The source is `~/.claude/projects/**/*.jsonl`, the session transcripts Claude Code already writes. Per assistant message they carry the model, the effort, the working directory, the git branch and the token counts (input, output, cache write, cache read). Skill and agent calls appear as `tool_use` entries.

Four details are worth knowing:

- **One message spans several lines.** Each content block of a message is its own transcript line, and each line repeats the same usage. Counting lines would roughly double every number, so events are deduped by message id (and by tool call id).
- **Transcripts have no cost field.** Cost is computed from a per-model price table in `src/usage/pricing.rs`, copied by hand from the published pricing page (the source and date are in the file). Input, output, cache reads and both cache write lifetimes (five minute and one hour) are priced separately, and the cost is worked out each time you read the data, so a price fix applies to old events without a re-ingest. It is still an estimate and can drift from published prices. A model that is not in the table is never guessed: its messages count as $0, and the summary and the dashboard JSON (`totals.unpriced_messages`, `totals.unpriced_models`) say how many and which models.
- **The repo is the real repository, not the last folder.** The working directory of a worktree or an agent folder would otherwise show up as its own repo. `src/usage/repo.rs` maps a directory to its repo: first by path (`.claude/worktrees/<name>`, `.git/...`, and playbook's own `repos/<owner>/<repo>` storage, which still works after the directory is deleted), then by asking git for the shared git dir, then by the folder's own name.
- **The account is not in the transcripts.** It is read once per ingest from `oauthAccount.emailAddress` in `~/.claude.json` and tagged onto every event. If you are not signed in, it is `unknown`.

## Storage

One SQLite database at `~/.config/playbook/usage/usage.db`, not tied to any repo, because spend only makes sense as a total. Events stay there after Claude Code or `ccc` prunes old sessions. Connection setup follows `src/gate/db.rs`: `busy_timeout` first, then WAL mode, because the server reads while `ingest` writes from other processes.

The first ingest after upgrading from an older version runs a one-time repair (`src/usage/backfill.rs`): it rescans the transcripts and corrects the repo and the cache lifetime split on rows stored earlier. A row whose transcript is gone keeps its old repo and is priced as five minute cache.

Ingest keeps a watermark per source (the newest timestamp it has read). It skips transcript files last modified before the watermark, so a repeated ingest is cheap. The watermark is inclusive and events are deduped by id, so re-reading the newest event is harmless. Each ingest runs in one `IMMEDIATE` transaction: the rows and the watermark commit together or not at all, and a second ingest waits for the first instead of failing.

## The dashboard server

`playbook usage dashboard` is a short-lived process. It checks the lock file `~/.config/playbook/usage/dashboard.lock`, which holds a pid, a port and the session token (mode 0600, in a 0700 folder). A lock in the older two field format has no token, so the command stops that old server (only if it really is a dashboard server) and starts a new one. A lock counts as live only when the process is alive and the port answers, so a killed server does not leave a dead lock behind. If the lock is live, the command opens the browser and exits. If not, it starts a detached copy of the same binary (a hidden `--serve` flag, `setsid`, no terminal) and waits up to 5 seconds for it to report its port.

The server binds `127.0.0.1` on a port the OS picks, so there is never a port clash and no `sudo`. It writes its own lock after it binds. Two commands started at once both start a server, but only one can claim the lock, and the other exits. `stop` signals only a live server, never a stale pid that may belong to something else now.

It uses `tiny_http`, which is synchronous. This binary has no async runtime on purpose (see the note at the top of `src/gate/db.rs`).

The page has four tabs: Overview (totals, cost per day and per model charts), Live, Breakdowns (every grouping as a table, plus skill and agent counts) and Sessions. The tabs are buttons with `role="tab"` in a `tablist`, with arrow, Home and End keys. The chosen tab is kept in `localStorage` (never in the URL hash, which belongs to the token). The page polls every 5 seconds and fetches only the active tab's endpoint, so a hidden tab costs nothing. A hidden browser page stops polling and closes its live stream, and resumes when it is shown again. Every breakdown table sorts when you click a header (numbers compare as numbers).

Four endpoints return usage data, all behind the same guard (Host allowlist, `Sec-Fetch-Site`, token), which is one code path for any path under `/api/`:

| Endpoint | Returns |
|---|---|
| `/api/data?range=` | Totals, every grouping (day, week, model, repo, branch, effort, account, agent), skill and agent counts, two charts. |
| `/api/sessions?range=` | One row per session with a message in the range, newest first: start, end, duration, repo, branch, agent, main model and all models, messages, tokens, cost. At most 500 rows; `more` counts the rest. |
| `/api/session?id=` | One session's messages (time, model, tokens, cost), the newest 2000 at most, and a cost-per-message chart. |
| `/api/live` | A Server-Sent Events stream (not JSON), see below. |

The first three return JSON. Each endpoint ingests anything new first. The session `id` is checked before any lookup: 1 to 128 characters of letters, digits, `-`, `_` and `.`, starting with a letter or digit. Anything else is a 400, an unknown id is a 404. A session's row stats count only the messages inside the range, while its timeline shows all of them. The charts are inline SVG drawn on the server, with every label escaped.

The page has a date range toggle: last 30, 60 or 90 days, the current month, or all time. It sends the choice as `/api/data?range=30d|60d|90d|month|all`, and the server filters usage and skill and agent counts to that window before it groups anything. Days are UTC. "Last N days" includes today. "Current month" runs from the 1st to the last day of the month. A request with no `range` gets all time, and an unknown value gets a 400. The response names the window in `range` (`key`, `start`, `end`), and the page shows those dates above the totals. The page opens on the last 30 days and remembers your choice in `localStorage`. The layout works on a phone: wide tables and charts scroll sideways in their own box.

The page is three same-origin files: `/` (HTML), `/app.css` and `/app.js`. They hold no data and need no token. Every `/api/` path does.

Because a browser can reach the server, it is locked down:

- **Session token.** Each server start makes a random 32 byte token (read from `/dev/urandom`, 64 hex characters) and keeps it in the lock file. Every `/api/` endpoint needs it in the `X-Playbook-Token` header and answers 401 without it. The comparison does not stop at the first wrong byte. This keeps another user account on the same machine from reading your usage data (it includes your account email, repo and branch names, and spend), because they cannot read your 0600 lock file.
- **The link carries the token in the fragment.** `playbook usage dashboard` opens and prints `http://127.0.0.1:<port>/#<token>`, and the same for the `playbook.localhost` alias. A browser never sends the fragment to the server, so it stays out of requests and logs. The page reads it, removes it from the address bar with `history.replaceState`, and keeps it in `sessionStorage` so a reload still works. If the token is missing or rejected, the page stops polling and tells you to run `playbook usage dashboard` for a fresh link. A new server start means a new token.
- **Accepted limit:** while the browser opener runs, the link is on its command line, so another user can see it in `ps` for a moment. The token only works for that one server run, and stopping the server ends it.
- It answers only when the `Host` header is `127.0.0.1`, `localhost` or `playbook.localhost` on its own port. Any other host gets a 403, which stops a web page from using DNS rebinding to read your usage data.
- `/api/` paths also refuse a request a browser labels `Sec-Fetch-Site: cross-site` or `same-site`, so another site cannot make the server ingest.
- **Script policy.** The policy is `default-src 'none'; script-src 'self'; style-src 'self'; connect-src 'self'; img-src 'self' data:; frame-ancestors 'none'; base-uri 'none'; form-action 'none'`, with `nosniff`. Nothing is inline, so neither scripts nor styles allow `'unsafe-inline'`, and the chart SVG uses classes, not `style=` attributes. The policy is sent with the page, the script and the stylesheet.
- The page puts text into the DOM with `textContent`. The one `innerHTML` use is the escaped SVG.

## Live updates

The Live tab shows the sessions with a message in the last 15 minutes (repo, agent, model, last message, spend over the whole session so far), spend and tokens for the last hour and for today (UTC), a per-minute spend chart for the last 60 minutes (server-rendered SVG, classes only) and the latest 20 messages.

It is fed by `/api/live`, a Server-Sent Events stream: `event: live` with a JSON summary (not the whole dataset) right away and then every 2 seconds. The browser's `EventSource` cannot send a header, so the page uses `fetch` with the `X-Playbook-Token` header, reads `response.body` with a stream reader and parses the events itself. The token is never put in a URL. On a dropped stream the page reconnects with a doubling delay (1 second up to 30), and a 401 stops it. Leaving the Live tab or hiding the page closes the stream. `connect-src 'self'` already allows this, so the policy did not change.

The server answers one request at a time on its main thread, so a held-open stream would block everything. The stream request goes through the same guard as every other `/api/` path (Host, `Sec-Fetch-Site`, token) and is then handed to its own thread, which takes over the socket (`tiny_http`'s `into_writer`) and writes a chunked `text/event-stream` body, flushing after each event. Limits:

- At most 4 streams at once; a fifth gets a 503. A stream's place is freed when its thread ends.
- A stream ends when a write fails (the client left, seen within a tick or two) or after 30 minutes, with a clean end of the body, and the page reconnects.
- "Last hour" and the burn chart cover the current minute and the 59 before it, so the chart adds up to the tile.
- Each tick runs the incremental ingest (safe across processes, see above), then reads only the last hour and today, the active sessions and the newest messages from the database. The result is cached for 1.5 seconds and shared, with the cache lock held while it is computed, so several open streams or tabs cause one ingest per tick, not one each. A failure is shared the same way, so a busy database is not retried by every stream.
- Known limits: a client that leaves is noticed on the second write after it goes (a tick or two), so a quick reopen can see a 503 and the page retries with backoff; and a client that keeps the connection open but never reads can hold a slot until the server restarts, because `tiny_http` exposes no write timeout on a taken-over socket.

## Adding another agent

`UsageSource` (`src/usage/mod.rs`) is the extension point. A source returns normalized `UsageEvent` and `ToolInvocationEvent` values newer than a watermark. Aggregation, storage, the summary and the dashboard never see an agent's own file format, so a second source only has to implement the trait. Two sources exist today: Claude Code (`src/usage/claude_code.rs`) and Codex (`src/usage/codex.rs`). Every stored event carries an `agent` column (`claude-code` or `codex`, old rows default to `claude-code`).

The Codex source reads `~/.codex/sessions/**/rollout-*.jsonl` read-only and is skipped silently when that directory is missing. Format source: openai/codex `codex-rs/protocol/src/protocol.rs` (`TokenCountEvent`, `TokenUsageInfo`, `SessionMeta`, `TurnContextItem`). The model, effort and cwd come from `turn_context`, the session id, cwd and branch from `session_meta`, and tokens from `event_msg` records of type `token_count`. One event is one change of the cumulative `total_token_usage.total_tokens`, taking its tokens from `last_token_usage`, and its id is `codex:<session>:<timestamp>:<total>`, so re-reads dedupe. Cached input is reported as cache reads. OpenAI models are not in the price table, so Codex events show as unpriced rather than guessed. Compressed `.jsonl.zst` rollouts are not read, and the account is `unknown` because rollouts carry none.

## Tests

Tests run against hand-written transcripts in `tests/fixtures/usage/` and a scratch `HOME`, never your real `~/.claude`. Server tests (`tests/usage_dashboard.rs`, `tests/usage_auth.rs`) set `PLAYBOOK_USAGE_NO_BROWSER=1` so no browser opens, and they serialize behind a lock because they bind real sockets. One test runs `usage ingest` in a loop while polling the server, to prove reads never fail and no event is lost or doubled.

Terminal view tests render into a `TestBackend` buffer, with no real terminal. `src/usage/tui/snapshots.rs` compares whole screens at fixed sizes with the text files in `tests/fixtures/usage_tui/`; after an intended layout change, regenerate them with `UPDATE_SNAPSHOTS=1 cargo test --lib usage::tui::snapshots` and review the diff. `src/usage/tui/parity.rs` checks that, for every range, the view reports the same totals and model, repo and day groups as the web JSON.

## Limits

- Cost is an estimate (see above). It is the price at the published API rate, not what a subscription plan charges you.
- It reads one machine's local history. It does not combine machines or users.
- Live shows what the transcripts record, so a session that is thinking but has not written a message yet does not appear until it does.
- The terminal view needs a real terminal of 80x24 or more. Use `--json` or `--summary` where there is none.
- No LLM call is built in. The output is plain text and JSON, so the agent already running your session can read it and suggest where to cut cost.
