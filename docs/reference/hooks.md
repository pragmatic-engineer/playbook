# Hook reference

Every hook that `playbook init` wires into `~/.claude/settings.json`, and the three the plugin registers itself (marked in the last column). Each command is `playbook hook <name>`. A test fails when init wires a hook that is missing from the tables below, so this page cannot drift.

Paths below use `~/.config/playbook/` as the playbook home. The session directory is `~/.config/playbook/runtime/<session id>/`, owner-only (mode 0700). No hook reads a secret value, and no hook sends anything over the network.

## How to turn a hook off

Five ways, from narrowest to widest.

- A per-hook switch, where the last column names one. Environment switches (`POLICY_GUARD=0`) go in the `env` block of `settings.json` or your shell.
- Disable the plugin in Claude Code. That sets `enabledPlugins["playbook@pragmatic-engineer"]` to `false`. Every hook then exits with no output and no side effects, and the status line prints nothing, while the entries stay in `settings.json`. Enable the plugin again to resume.
- Delete the hook's entry from `~/.claude/settings.json`. Init puts it back on the next run unless you pass `--no-hooks`.
- `playbook init --no-hooks` never adds playbook hook entries. It does not remove ones that are already there.
- `playbook uninstall` removes every entry init wired. Add `--dry-run` to see the list first.

A hook that fails or cannot read its input allows the call. Only the hooks marked "can deny" or "can block" ever stop a tool call.

## Safety guards

| Event | Matcher | Command | What it does | Can block or change | Reads and writes | Turn off |
| :- | :- | :- | :- | :- | :- | :- |
| `PreToolUse` | `Bash` | `playbook hook rm-workspace-guard` | Stops an `rm` whose target is outside your safe roots. Runs only for `rm` commands. | Can deny the call. For playbook's own plans folder the denial points to `playbook plan cleanup <slug>`. | Reads the command and `PLAYBOOK_SAFE_ROOTS`. Writes nothing. | Entry only. `PLAYBOOK_SAFE_ROOTS` adds safe roots. |
| `PreToolUse` | `Bash` | `playbook hook bg-await-guard` | Warns when a command backgrounds an install, build or typecheck that a later step needs. | Warns only. Never blocks. | Reads the command. Writes nothing. | `BG_AWAIT_GUARD=0` |
| `PreToolUse` | `Bash` | `playbook hook no-slop-guard` | Denies a posting command (commit, PR text) that contains an em or en dash. | Can deny the call. | Reads the command and any message file it names. Writes nothing. | `NO_SLOP_GUARD=0` |
| `PreToolUse` | `Edit\|Write` | `playbook hook no-slop-guard` | Denies new code that carries a slop comment. | Can deny the call. | Reads the content being written. Writes nothing. | `NO_SLOP_GUARD=0` |
| `PreToolUse` | `Bash` | `playbook hook precommit-check` | Warns about debug leftovers, secret-shaped file names and oversized commits in the staged diff. Runs only for `git commit`. | Warns only. Never blocks. | Reads the staged diff through git. File names only for the secret check. Writes nothing. | `PRECOMMIT_CHECK=0` |
| `PreToolUse` | `Bash` | `playbook hook commit-message-sanitizer` | Removes AI attribution from commit, tag, merge and PR messages, and adds `-s` to a commit that has no sign-off. | Changes the command input. Never blocks. | Reads the command and the `commit.signOff` config key. Writes nothing. | `commit.signOff` false stops only the `-s` part. Otherwise entry only. |
| `PostToolUse` | `Bash` | `playbook hook commit-message-sanitizer` | Backstop for what the PreToolUse rewrite did not see. It can amend a signed commit and edit a PR. | Changes the commit or PR after the call. Never blocks. | Reads the command and the commit or PR text. Writes through git and `gh`. | Entry only. |
| `PreToolUse` | `Bash` | `playbook hook policy-guard` | Denies skipped git hooks or signing, a hand-run `gh pr create`, and memory writes outside playbook's store. | Can deny the call. | Reads the command. Writes nothing. | `POLICY_GUARD=0` |
| `PreToolUse` | `Edit\|Write` | `playbook hook policy-guard` | Denies writes to Claude Code's own memory paths. Shows the fact format when a fact file is written to playbook's store. | Can deny the call. | Reads the target path. Writes nothing. | `POLICY_GUARD=0` |

## Session lifecycle

| Event | Matcher | Command | What it does | Can block or change | Reads and writes | Turn off |
| :- | :- | :- | :- | :- | :- | :- |
| `SessionStart` | none | `playbook hook session-init` | Prepares the session directory, warns when a resumed session's config drifted, and adds a handoff, an auto-learn nudge and a small memory block to the context. Also sweeps stale worktrees. | Adds context. Never blocks. | Reads the memory graph, saved handoffs and the to-learn queue. Writes the session directory (counters, start time, config hash). | `AUTO_LEARN_NUDGE=0` stops the nudge. `worktreeCleanup.enabled` false stops the sweep. Otherwise entry only. |
| `Stop` | none | `playbook hook session-clean-exit` | Records that the session ended cleanly after each turn, so the next start can spot a crash. | Never blocks. | Writes `last-clean-ts` in the session directory and may queue an auto-learn entry under `runtime/to-learn/`. | `AUTO_LEARN_NUDGE=0` stops the queue entry. |
| `SessionEnd` | none | `playbook hook session-clean-exit` | Writes the clean-exit marker when the session truly ends. | Never blocks. | Writes `clean-exit` in the session directory. Reads `edit-count`. | `AUTO_LEARN_NUDGE=0` stops the queue entry. |
| `PreCompact` | none | `playbook hook precompact-warn` | Warns you that the context is about to be compacted and logs the event. | Warns only. Never blocks. | Appends a timestamp line to `~/.config/playbook/runtime/compactions.log`. | Entry only. |
| `SessionStart` | none | `playbook hook migration-check` | Tells you to re-run `playbook init` when a plugin update arrived without it. Silent once settings are wired. | Shows a warning. Never blocks. | Reads `~/.claude/settings.json`. Writes nothing. | Disable or remove the plugin. |

## Usage tracking and auto mode

The usage store is filled by `playbook usage` itself, by the `usage-warm` hook in the background, and from telemetry by `playbook usage collect`. See [usage telemetry privacy](../internals/04-usage-dashboard.md#telemetry-privacy). These hooks keep per-session counters and enforce the spend cap in `auto` mode.

| Event | Matcher | Command | What it does | Can block or change | Reads and writes | Turn off |
| :- | :- | :- | :- | :- | :- | :- |
| `PostToolUse` | `Edit\|Write\|NotebookEdit` | `playbook hook post-edit-track` | Records which files this session edited. | Never blocks. Prints nothing. | Appends the path and a time to `edits.jsonl` and bumps `edit-count` in the session directory. | Entry only. |
| `PreToolUse` | `Read\|Grep\|Glob\|Edit\|Write\|NotebookEdit` | `playbook hook search-counter` | Counts exploration calls and suggests the Explore subagent once, at the eighth. | Adds context once. Never blocks. | Reads and writes `search-count` in the session directory. | Entry only. |
| `PreToolUse` | `AskUserQuestion` | `playbook hook auto-guard` | In `auto` mode, denies questions to you so an unattended run does not stall. | Can deny the call, in `auto` mode only. | Reads the `mode` setting. Writes a once-only marker in the session directory. | `playbook mode ask` (the default). |
| `UserPromptSubmit` | none | `playbook hook auto-guard` | In `auto` mode, advises a trusted permission mode once per session. | Adds context. Never blocks. | Reads the `mode` setting. | `playbook mode ask` (the default). |
| `PreToolUse` | none | `playbook hook auto-cost` | In `auto` mode, tracks spend per agent against `auto.budgetUsd` (main session) and `auto.agentBudgetUsd` (each subagent), and denies new work for an agent at its cap while the closing steps stay allowed. Parks every agent at `auto.sessionCeilingUsd`. | Can deny the call, in `auto` mode only. | Reads the session transcripts and status line telemetry. Writes `auto-cost.json` in the session directory. | `playbook mode ask` (the default). |
| `SessionStart` | none | `playbook hook usage-warm` | Starts a short background read of new session history, so `playbook usage` opens with data. Prints nothing and waits for nothing. | Never blocks. Prints nothing. | Starts a detached, low priority `playbook usage warm`, which reads session transcripts the way `playbook usage` does and writes only the usage store plus `auto-ingest.lock` and `auto-ingest.stamp` in `~/.config/playbook`. It stores no transcript text beyond what the usage store already keeps. Skipped when the last run finished less than `usage.autoIngestMinutes` ago. | `playbook config set usage.autoIngest false`. |
| `SessionEnd` | none | `playbook hook usage-warm` | Starts the same background read when a session ends, so the finished session is already stored. | Never blocks. Prints nothing. | Same as above. | `playbook config set usage.autoIngest false`. |

## Memory

| Event | Matcher | Command | What it does | Can block or change | Reads and writes | Turn off |
| :- | :- | :- | :- | :- | :- | :- |
| `PreToolUse` | `Edit\|Write` | `playbook hook memory-anchors` | Shows the memory facts anchored to the file you are about to edit. | Adds context. Never blocks. | Reads `memory/memory.graph.json`. Writes an anchor index file in the session directory. | Entry only. |
| `UserPromptSubmit` | none | `playbook hook memory-anchors` | Adds the memory facts that match your prompt or the files touched this session. | Adds context. Never blocks. | Same as above. | Entry only. |
| `PostToolUse` | `Edit\|Write\|NotebookEdit` | `playbook hook rebuild-memory-graph` | Rebuilds the memory graph after a fact file is saved. Does nothing for any other file. | Never blocks. Prints nothing. | Reads the memory tree. Writes `memory/memory.graph.json` atomically. | Entry only. |
| `Stop` | none | `playbook hook memory-capture` | Once per context threshold crossing, asks the model to write down durable facts before it continues. | Can block the stop (up to two re-blocks), then releases. | Reads `edits.jsonl` and the memory graph. Writes `capture-attempts` in the session directory. Also runs the one-time memory move described in [What init installs](../guides/06-what-init-installs.md#memory-migration). | Entry only. |

## Hygiene

| Event | Matcher | Command | What it does | Can block or change | Reads and writes | Turn off |
| :- | :- | :- | :- | :- | :- | :- |
| `PreToolUse` | `Read` | `playbook hook preread-edit-check` | Reminds the model not to re-read a file it just edited. | Adds context. Never blocks. | Reads `edits.jsonl` in the session directory. | Entry only. |
| `PreToolUse` | `Read` | `playbook hook preread-size-check` | Denies a whole-file Read of a large file, and a repeat full Read of an unchanged file, and points to a targeted Read. | Can deny the call. | Reads file size and modification time. Writes `read-state.jsonl` in the session directory. | Entry only. |
| `PreToolUse` | `Read` | `playbook hook doc-read-hint` | When `docs.convert` is `auto` and anydoc is installed, suggests `playbook doc read` for PDF and Office files. It only suggests. | Adds context. Never blocks and never changes the call. | Reads config and looks for `anydoc` on PATH. Writes nothing. | `playbook config set docs.convert off` |
| `UserPromptSubmit` | none | `playbook hook auto-model-detect` | Suggests `/playbook:plan`, `/playbook:adr` or `/playbook:implement` when your prompt looks like design or build work. | Adds context. Never blocks. | Reads your prompt text. Writes a once-only marker in the session directory. | Entry only. Silent in headless runs. |
| `UserPromptSubmit` | none | `playbook hook context-hygiene` | Nudges the model to save a handoff when the context passes `hygiene.thresholdTokens`. | Adds context. Never blocks. | Reads the tail of the session transcript. Writes `hygiene-level` in the session directory. | `playbook config set hygiene.enabled false` |

## Worktree

These two come from the plugin's `hooks/hooks.json`, not from `settings.json`. They exist only while the plugin is installed. `playbook init` does not wire them. The `migration-check` hook in Session lifecycle comes from the plugin too.

| Event | Matcher | Command | What it does | Can block or change | Reads and writes | Turn off |
| :- | :- | :- | :- | :- | :- | :- |
| `WorktreeCreate` | none | `playbook hook worktree-create` | Creates worktrees under `<repo-parent>/.worktrees/<repo>/<name>`, copies files named in `.worktreeinclude` and keeps `origin/HEAD` at most a day old. | Replaces Claude Code's default worktree creation. | Runs git in your repo. Writes the new worktree and a refresh stamp inside the git directory. | Disable or remove the plugin. |
| `WorktreeRemove` | none | `playbook hook worktree-remove` | Removes a worktree only when nothing would be lost. Dirty trees and unpublished commits keep it; squash-merged branches count as published. | Never forces a removal. | Runs git in your repo. | Disable or remove the plugin. |
