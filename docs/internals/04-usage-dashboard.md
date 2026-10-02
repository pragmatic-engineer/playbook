# Internals: Usage Dashboard

`playbook usage` shows how many tokens and how many dollars your sessions use, split by day, week, model, repo, branch, effort, account, skill and agent. It reads Claude Code's own session history, so it needs no setup and no new instrumentation.

## Commands

| Command | What it does |
|---|---|
| `playbook usage` | Reads anything new, then prints a text summary in the terminal. |
| `playbook usage ingest` | Reads anything new into the store and prints how many events it added. |
| `playbook usage dashboard` | Starts the local dashboard server if it is not running, then opens it in your browser. |
| `playbook usage dashboard stop` | Stops the server. |

Every command that shows data reads new history first, so you never need to ingest by hand. `ingest` exists for scripts and for checking what was added.

The dashboard is for macOS and Linux. The server prints two addresses: `http://127.0.0.1:<port>`, which it always opens, and `http://playbook.localhost:<port>`, which works where your system resolves `*.localhost` (Chrome on macOS does).

## Where the data comes from

The source is `~/.claude/projects/**/*.jsonl`, the session transcripts Claude Code already writes. Per assistant message they carry the model, the effort, the working directory, the git branch and the token counts (input, output, cache write, cache read). Skill and agent calls appear as `tool_use` entries.

Four details are worth knowing:

- **One message spans several lines.** Each content block of a message is its own transcript line, and each line repeats the same usage. Counting lines would roughly double every number, so events are deduped by message id (and by tool call id).
- **Transcripts have no cost field.** Cost is computed from a per-model price table in `src/usage/pricing.rs`, copied by hand from the published pricing page (the source and date are in the file). Input, output, cache reads and both cache write lifetimes (five minute and one hour) are priced separately, and the cost is worked out each time you read the data, so a price fix applies to old events without a re-ingest. It is still an estimate and can drift from published prices. A model that is not in the table is never guessed: its messages count as $0, and the summary and the dashboard JSON (`totals.unpriced_messages`, `totals.unpriced_models`) say how many and which models.
- **The repo is the real repository, not the last folder.** The working directory of a worktree or an agent folder would otherwise show up as its own repo. `src/usage/repo.rs` maps a directory to its repo: first by path (`.claude/worktrees/<name>`, `.git/...`, and playbook's own `repos/<owner>/<repo>` storage, which still works after the directory is deleted), then by asking git for the shared git dir, then by the folder's own name.
- **The account is not in the transcripts.** It is read once per ingest from `oauthAccount.emailAddress` in `~/.claude.json` and tagged onto every event. If you are not signed in, it is `unknown`.

## Storage

One SQLite database at `~/.config/playbook/usage/usage.db`, not tied to any repo, because spend only makes sense as a total. Events stay there after Claude Code or `cc` prunes old sessions. Connection setup follows `src/gate/db.rs`: `busy_timeout` first, then WAL mode, because the server reads while `ingest` writes from other processes.

The first ingest after upgrading from an older version runs a one-time repair (`src/usage/backfill.rs`): it rescans the transcripts and corrects the repo and the cache lifetime split on rows stored earlier. A row whose transcript is gone keeps its old repo and is priced as five minute cache.

Ingest keeps a watermark per source (the newest timestamp it has read). It skips transcript files last modified before the watermark, so a repeated ingest is cheap. The watermark is inclusive and events are deduped by id, so re-reading the newest event is harmless. Each ingest runs in one `IMMEDIATE` transaction: the rows and the watermark commit together or not at all, and a second ingest waits for the first instead of failing.

## The dashboard server

`playbook usage dashboard` is a short-lived process. It checks the lock file `~/.config/playbook/usage/dashboard.lock`, which holds a pid, a port and the session token (mode 0600, in a 0700 folder). A lock in the older two field format has no token, so the command stops that old server (only if it really is a dashboard server) and starts a new one. A lock counts as live only when the process is alive and the port answers, so a killed server does not leave a dead lock behind. If the lock is live, the command opens the browser and exits. If not, it starts a detached copy of the same binary (a hidden `--serve` flag, `setsid`, no terminal) and waits up to 5 seconds for it to report its port.

The server binds `127.0.0.1` on a port the OS picks, so there is never a port clash and no `sudo`. It writes its own lock after it binds. Two commands started at once both start a server, but only one can claim the lock, and the other exits. `stop` signals only a live server, never a stale pid that may belong to something else now.

It uses `tiny_http`, which is synchronous. This binary has no async runtime on purpose (see the note at the top of `src/gate/db.rs`).

The page polls `/api/data` every 5 seconds. That endpoint ingests anything new, then returns the totals, every grouping and two charts. The charts are inline SVG drawn on the server, with every label escaped.

The page is three same-origin files: `/` (HTML), `/app.css` and `/app.js`. They hold no data and need no token. Only `/api/data` does.

Because a browser can reach the server, it is locked down:

- **Session token.** Each server start makes a random 32 byte token (read from `/dev/urandom`, 64 hex characters) and keeps it in the lock file. `/api/data` needs it in the `X-Playbook-Token` header and answers 401 without it. The comparison does not stop at the first wrong byte. This keeps another user account on the same machine from reading your usage data (it includes your account email, repo and branch names, and spend), because they cannot read your 0600 lock file.
- **The link carries the token in the fragment.** `playbook usage dashboard` opens and prints `http://127.0.0.1:<port>/#<token>`, and the same for the `playbook.localhost` alias. A browser never sends the fragment to the server, so it stays out of requests and logs. The page reads it, removes it from the address bar with `history.replaceState`, and keeps it in `sessionStorage` so a reload still works. If the token is missing or rejected, the page stops polling and tells you to run `playbook usage dashboard` for a fresh link. A new server start means a new token.
- **Accepted limit:** while the browser opener runs, the link is on its command line, so another user can see it in `ps` for a moment. The token only works for that one server run, and stopping the server ends it.
- It answers only when the `Host` header is `127.0.0.1`, `localhost` or `playbook.localhost` on its own port. Any other host gets a 403, which stops a web page from using DNS rebinding to read your usage data.
- `/api/data` also refuses a request a browser labels `Sec-Fetch-Site: cross-site` or `same-site`, so another site cannot make the server ingest.
- **Script policy.** The policy is `default-src 'none'; script-src 'self'; style-src 'self'; connect-src 'self'; img-src 'self' data:; frame-ancestors 'none'; base-uri 'none'; form-action 'none'`, with `nosniff`. Nothing is inline, so neither scripts nor styles allow `'unsafe-inline'`, and the chart SVG uses classes, not `style=` attributes. The policy is sent with the page, the script and the stylesheet.
- The page puts text into the DOM with `textContent`. The one `innerHTML` use is the escaped SVG.

## Adding another agent

`UsageSource` (`src/usage/mod.rs`) is the extension point. A source returns normalized `UsageEvent` and `ToolInvocationEvent` values newer than a watermark. Aggregation, storage, the summary and the dashboard never see an agent's own file format, so a second source only has to implement the trait. Only Claude Code is implemented today (`src/usage/claude_code.rs`).

## Tests

Tests run against hand-written transcripts in `tests/fixtures/usage/` and a scratch `HOME`, never your real `~/.claude`. Server tests (`tests/usage_dashboard.rs`, `tests/usage_auth.rs`) set `PLAYBOOK_USAGE_NO_BROWSER=1` so no browser opens, and they serialize behind a lock because they bind real sockets. One test runs `usage ingest` in a loop while polling the server, to prove reads never fail and no event is lost or doubled.

## Limits

- Cost is an estimate (see above). It is the price at the published API rate, not what a subscription plan charges you.
- It reads one machine's local history. It does not combine machines or users.
- It does not track live agent sessions.
- No LLM call is built in. The output is plain text and JSON, so the agent already running your session can read it and suggest where to cut cost.
