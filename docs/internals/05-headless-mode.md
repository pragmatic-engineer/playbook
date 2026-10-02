# Headless mode

Can playbook run with no person at the keyboard, for example in CI? This page records what was tested on 2026-10-03 with playbook 0.16.0 and Claude Code 2.1.286. Every claim below came from a real run in a scratch `HOME` and a scratch git repo with no remote, unless it says it did not.

Docs consulted (Claude Code docs: headless, hooks, plugins, authentication, GitHub Actions). Where a doc claim was wrong or incomplete, the section says so.

## Short answer

- The compiled `playbook` binary already works unattended. No command read stdin or waited for input.
- No model is needed for useful CI checks: `agents check`, `manifest check`, `settings check`, and the `gate` commands.
- Slash commands also run under `claude -p` when the plugin is loaded with `--plugin-dir`, and playbook's hooks fire.
- The risks are around the model: a denied permission is not an error exit, the memory Stop hook can add turns, and any command that asks a question has no one to answer.

## What works without a model

All run with `CI=true`, stdin from `/dev/null`, and an empty `HOME`.

| Command | Unattended | Notes |
|---|---|---|
| `playbook --version` | yes | exit 0, one line |
| `agents check` | yes | needs a checkout (`agents/`); exit 1 outside one |
| `manifest check .` | yes | exit 0 on a clean tree |
| `settings check <template> <perms> <root>` | yes | three required arguments |
| `gate record` and `gate check` | yes | need `--source <plan file>` and a git `origin` remote; no remote exits 1 with a clear message; an edited plan reads `STALE`; a missing phase reads `MISSING`, exit 1 |
| `usage`, `usage ingest` | yes | no transcripts: exit 0, "No usage recorded yet" |
| `handoff save`, `handoff show`, `handoff status` | yes | `save` reads stdin and refuses empty input |
| `trust <path>` | yes | always exit 0 |
| `pr prepare --help` | yes | exit 0, so a caller can detect support |
| `init` | yes | no prompt; second run changes nothing; with no plugin root it wires hooks and skips the rest |
| `usage dashboard` | not useful | starts a background server for a browser |

Nothing needed anything from `~/.claude` except `init`, which writes `~/.claude/settings.json`. The state directories created under an empty `HOME` are `~/.config/playbook/usage`, `runtime/` (handoffs, session folders, `session-start.log`), `memory/`, and `repos/<owner>/<repo>/<worktree>/state.db` for the gate.

## Running Claude headless

Flags checked against `claude --help`: `-p`, `--output-format json|stream-json`, `--max-turns`, `--max-budget-usd`, `--allowedTools`, `--permission-mode`, `--permission-prompts`, `--plugin-dir`, `--setting-sources`, `--settings`, `--no-session-persistence`, `--session-id`.

Real exit codes and result fields (all with haiku, each call capped at 10 to 15 cents):

| Case | Exit | Result fields |
|---|---|---|
| Normal run | 0 | `is_error:false`, `subtype:success` |
| Budget exceeded (`--max-budget-usd 0.0001`) | 1 | `is_error:true`, `subtype:error_max_budget_usd`, `terminal_reason:budget_exhausted` |
| `--max-turns 1` on a prompt needing a tool | 1 | `subtype:error_max_turns`, `terminal_reason:max_turns` |
| A write the permissions deny | **0** | `is_error:false`; the denial is only in the `permission_denials` array |
| Not logged in | 1 | `result` is "Not logged in" |

The docs say a permission denial is a failure exit. It is not: a CI wrapper must read `permission_denials` and fail on a non-empty list.

Other facts:

- `--allowedTools` takes several values, so it swallows a prompt placed after it. Put the prompt first.
- `AskUserQuestion` does not exist under `-p` (the model said it has no such tool), with or without `--permission-prompts none`. The model falls back to asking in plain text, which no one reads, and the run still exits 0. A command that must ask has no headless path.
- Under a scratch `HOME`, `claude -p` says "Not logged in". Credentials live in the real `HOME`, so a runner needs `ANTHROPIC_API_KEY` or an OAuth token.
- `--bare` is excluded by decision. It skips hooks, plugins, skills, and keychain reads, and authenticates only with `ANTHROPIC_API_KEY`, so it would switch off the playbook plugin and its hooks. Design for a normal `claude -p` run with hooks on, the plugin loaded by `--plugin-dir`, and auth by `ANTHROPIC_API_KEY` or a subscription token (`CLAUDE_CODE_OAUTH_TOKEN` from `claude setup-token`).

## Plugin and hooks under `-p`

- `--plugin-dir <checkout>` loads playbook with no marketplace install. `system/init` in `stream-json` listed it as `playbook@inline`, version 0.16.0, with 23 playbook slash commands.
- `claude -p "/playbook:session-start"` ran the command: it called `playbook handoff show`, read the log, and checked git (4 turns, 14 cents).
- Hooks that fired in a headless run: SessionStart (`session-init`, source `startup`), UserPromptSubmit (`auto-model-detect`, `memory-anchors`), PreToolUse (`preread-*`, `no-slop-guard`, `bg-await-guard`, `memory-anchors`), PostToolUse (`search-counter`, `post-edit-track`), Stop (`memory-capture`, `session-clean-exit`). The `statusLine` never renders.
- Folder trust: the runs happened in a fresh scratch directory with no trust entry. The project hooks and the plugin still ran, so `playbook trust` is not needed for CI.

### The memory Stop hook

`memory-capture` blocks Stop only when a `capture-due` marker exists. That marker is written by `statusline.sh`, which does not run headless, so in normal headless runs it never fires (checked: zero markers after four runs).

To see the worst case, a marker was created for a fixed `--session-id`. The hook blocked Stop twice, then released. The run took 3 turns instead of 1, cost about 2 cents, and ended with exit 0. It did not loop or hang (the re-block cap is 2). One side effect: the final `result` text was the model's answer to the nudge, not to the prompt. Do not trust `result` as the answer when this hook may fire.

### Hook by hook, with hooks on

Each hook was judged for a normal `claude -p` run in a clean CI `HOME`. The last column held the recommendation; the next section says what shipped.

| Hook | Headless behavior (tested) | Recommended change |
|---|---|---|
| `session-init` | Exits 0 and writes `session-start.log`. On a cold `HOME` it injects about 800 bytes (an async and deferred-tool discipline note). Its nudges are switched off by `AUTO_LEARN_NUDGE=0`, `SKILLS_PRIMER=0`, and `ASYNC_DISCIPLINE=0`; with all three off it injects nothing. | Done: headless skips every nudge and injects no memory unless `PLAYBOOK_HEADLESS_MEMORY=1`. |
| `session-init` worktree sweep | Rate limited to once a day and gated by `worktreeCleanup.enabled`. A fresh CI checkout has nothing to sweep. No harm seen. | Done: skipped headless. |
| `session-init` config drift and memory injection | A cold `HOME` has no memory, so nothing is injected. A warm `HOME` (restored cache) would inject memory and spend tokens. | Done: opt in with `PLAYBOOK_HEADLESS_MEMORY=1`. |
| `memory-capture` (Stop) | Blocks only when `capture-due` exists, and only `statusline.sh` writes it, so it does not fire headless. If forced, it adds 2 turns and ends cleanly. | Done: never blocks headless, and never when `stop_hook_active` is true. |
| `memory-anchors`, `auto-model-detect` (UserPromptSubmit) | Fire and print nothing on a cold `HOME`. `auto-model-detect` only suggests a model. | Done: `auto-model-detect` is silent headless. |
| `preread-*`, `no-slop-guard`, `bg-await-guard`, `rm-workspace-guard`, `precommit-check` (PreToolUse) | Fire and stay quiet. These are safety guards. | Keep them on. They are the reason to run with hooks. |
| `search-counter`, `post-edit-track`, `session-clean-exit` | Write small files under `runtime/`. Harmless. | None, or skip the writes headless. |

No hook reads a TTY or prompts: only `src/hooks/mod.rs` touches stdin, to read the hook payload.

## The headless switch

One switch covers every row above. `PLAYBOOK_HEADLESS` set to `1`, `true`, `yes`, or `on` turns headless mode on, and `0`, `false`, `no`, or `off` forces it off. When `PLAYBOOK_HEADLESS` is unset, `CI=true` (or `CI=1`) turns it on, so a developer who runs with `CI=true` locally can opt out with `PLAYBOOK_HEADLESS=0`. The check lives in `src/common/headless.rs`.

When headless:

- `session-init` skips the worktree sweep, the handoff load, and the three nudges (the same effect as `AUTO_LEARN_NUDGE=0`, `SKILLS_PRIMER=0`, and `ASYNC_DISCIPLINE=0`). It injects no memory either, unless `PLAYBOOK_HEADLESS_MEMORY=1` opts memory in. It still writes `session-start.log`.
- `memory-capture` (Stop) never blocks, even if a `capture-due` marker exists.
- `auto-model-detect` stays silent.
- The safety guards (`preread-*`, `no-slop-guard`, `bg-await-guard`, `rm-workspace-guard`, `precommit-check`) behave exactly as they do interactively.

Separately from the switch, `memory-capture` now returns without blocking whenever the payload carries `stop_hook_active: true`. Claude Code sets that when a Stop hook already blocked the turn, so a blocking Stop hook cannot loop. The marker is kept, so the next Stop still nudges, and the re-block cap of 2 still applies.

## What must never run unattended

- `/playbook:implement` with `--boundary=land`. It merges code with no human review.
- Anything that pushes, opens PRs, or merges: `playbook pr create`, `/playbook:create-pull-request`, `gh pr merge`.
- `/playbook:plan`, `/playbook:adr`, and `/playbook:learn-project`. They are interviews. `AskUserQuestion` is absent and the model just writes the question as text.
- `/playbook:setup` unless `--yes` is passed, because it asks via `AskUserQuestion`.
- A model that reads untrusted pull request text (titles, bodies, comments, diffs from forks) with write or shell tools. That text is prompt injection input.

Commands that ask a person today: `implement` (delivery questions unless `--auto` and flags preset them), `plan` (the divergent phase always asks, even with `--auto`), `adr`, `setup` (`--yes` skips), `address-pr-comments` (per-comment gates, `-y` skips only the last), `quick-review` (asks how to submit), `learn-project` (asks if the Jira or Confluence target is ambiguous). `commit-and-push`, `create-pull-request`, `deep-review`, `session-start`, `doctor`, and `repo-audit` run without questions.

## GitHub Actions

The documented action is `anthropics/claude-code-action@v1`. It takes `anthropic_api_key` or `claude_code_oauth_token`, and `plugin_marketplaces`, `plugins`, and `claude_args`. It needs a write-access actor, and pull requests from forks get no secrets. Not run here.

First use case, no model, proven locally (checksum check passes, a wrong checksum fails, each check exits 0, nothing was written to `HOME`). This is a documentation snippet, not a workflow in this repo:

```yaml
jobs:
  playbook-checks:
    runs-on: ubuntu-latest
    steps:
      - uses: actions/checkout@v4
      - name: Install playbook
        run: |
          V=v0.16.0; A=playbook-0.16.0-x86_64-unknown-linux-musl
          gh release download "$V" --repo pragmatic-engineer/playbook --pattern "$A" --pattern SHA256SUMS
          grep " $A\$" SHA256SUMS | sha256sum -c -
          install -m 0755 "$A" /usr/local/bin/playbook
        env:
          GH_TOKEN: ${{ github.token }}
      - run: playbook agents check
      - run: playbook manifest check .
      - run: playbook settings check settings.shared.json permissions.shared.json .
```

State between runs: these checks create nothing. A gate check needs the `state.db` under `~/.config/playbook/repos/...`, so cache that directory if a pipeline records gates in one job and checks them in another.

## Recommended build order

1. Ship the no-model checks above as a documented snippet, and a `playbook ci` command that runs them together with one exit code.
2. Add `--output-format json` style output to `gate check` and `doctor` so a pipeline can parse results.
3. Give each command that asks a question a headless rule: a flag that presets every answer, or a hard stop with a clear message.
4. Only then run commands under `claude -p`, with a wrapper that fails on `permission_denials`, sets `--max-turns` and `--max-budget-usd`, and never passes `bypassPermissions`.

## Open items

- The GitHub Action itself was not run.
- Only macOS arm64 was used. Linux runners are expected to match but were not tested.
- Whether a `capture-due` marker can appear headless through any other path is not proven.
- Cost of a real review run (`/playbook:quick-review`) under `-p` was not measured.
