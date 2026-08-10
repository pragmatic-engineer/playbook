# Internals: Launcher and Hooks

The `cc` launcher is the entry point for every session. It wraps `claude` with a system prompt, transcript retention, and config-drift detection. Hooks extend the session lifecycle with guards, nudges, and state tracking. Together they're the machine the rest of the config runs on.

## The `cc`/`ccd` Launcher

Defined in `shell/zsh/cc.zsh`, the launcher sources six module files from `shell/zsh/` and `shell/shared/worktree.sh`, then defines two public shell functions. Both call the internal `_claude` dispatcher. `ccd` is `cc` with `--dangerously-skip-permissions` prepended. Nothing else differs.

On every invocation, `cc` passes `--system-prompt-file ~/.claude/prompts/SYSTEM_PROMPT.md` to `claude`. After `claude` exits, it runs `_cc_prune` to keep only the newest `CCD_KEEP` transcripts (default 5, floor 2) per project. Older transcripts plus their sidecars and runtime state are deleted.

### Subcommands

| Command | Behavior |
|---|---|
| `cc` (no args) | Resumes the most recent session for `$PWD` whose `customTitle` matches the directory name. If none exists, starts fresh. Forks a new transcript on config drift. |
| `cc fresh` | Starts a new session with no history. |
| `cc list` | Lists recent sessions for `$PWD` with timestamps and titles. |
| `cc clean` | Clones the latest matching transcript with `/model`, `/effort`, `/config`, `/output-style`, and `/style` overrides stripped, then resumes the clone. Conversation is preserved; runtime config resets to `settings.json`. The original transcript is untouched. |
| `cc raw [id]` | Resumes verbatim. No fork, no cleanup. Preserves the original UUID and frozen overrides. Defaults to the latest matching session when `id` is omitted. |
| `cc worktree <branch>` | Creates or enters a git worktree for the branch, then starts a session there. See [Worktree engine](#the-worktree-engine). |

### Config-drift detection

On every default resume, `cc` computes a SHA-256 hash of `settings.json` and every hook script, then compares it to the hash stored at session start (in `~/.claude/cc-state/<project-slug>`). When they differ, `cc` forks a new transcript so the fresh copy loads the current config. A plain resume fires only when nothing changed.

The `session-init.sh` hook mirrors this: on `source=resume`, it recomputes the hash and emits a user-visible warning when the resumed session is running on the old config. The README states this directly: config or hook edits take effect on a fresh session, not a resumed one. Use `cc fresh` or `cc clean` after editing `settings.json` or any hook.

## The Worktree Engine

`cc worktree <branch>` delegates to `_cc_worktree` in `shell/shared/worktree.sh`. It's only accessible through `cc`/`ccd`, not as a standalone command.

What it does, in order:

1. Detects the repo's base branch via `origin/HEAD`, falling back to `main`, `master`, `trunk`, or `develop`.
2. Auto-stashes any dirty main worktree and restores it afterward via a `zsh always {}` block.
3. Derives the folder name from the JIRA key in the branch name (`PROJECT-1234-foo-bar` → `PROJECT-1234/`). Falls back to the branch leaf when there's no JIRA key.
4. Creates the worktree at `<repo-parent>/<base>/<repo>/<folder>`, where `<base>` is `WORKTREE_BASE_DIR` (default `.worktrees`) and `<repo>` is the repo directory name, so worktrees from sibling repos that share a parent never collide. A relative `WORKTREE_BASE_DIR` sits under the repo's parent; an absolute one is used as-is. If the worktree already exists on the right branch, it fast-forward pulls instead.
5. Copies `.env` from the base repo (no-clobber).
6. Sets upstream tracking. Creates the remote branch via `git push -u` if it doesn't exist yet.
7. Rebases the branch onto the latest base when the branch belongs to you. With `--ai-resolve`, rebase conflicts go to Claude haiku for resolution. Without it, a conflict aborts the rebase. This subcommand always passes `--ai-resolve`.
8. In the background: full prune fetch, upstream sync, hardlink reuse of `node_modules` when `package-lock.json` hashes match, and a daily-rate-limited cleanup of merged or 30-day-old worktrees (skips open-PR branches and directories currently in use).

## The Hook Lifecycle

All hooks source `hooks/lib/common.sh` for JSON parsing, session-dir management, atomic file writes, and the `emit_*` helpers that produce compliant hook output. Hooks exit `0` on unexpected errors to avoid breaking the session.

Per-session state lives in `~/.claude/runtime/<session_id>/`. The session dir holds counters (`search-count`, `tool-count`, `edit-count`), an `edits.jsonl` log, a `seen-reads` list, timestamps, and the config hash baseline.

### SessionStart

| Script | Purpose |
|---|---|
| `session-init.sh` | Creates the per-session runtime dir and zeros its counters. Clears the statusline PR/CI cache for the current branch. Checks the config hash and warns on drift. Derives `<owner>/<repo>` from the git remote and injects `~/.claude/memory/<owner>/<repo>/MEMORY.md` as additional context when present. |

### PreToolUse

| Matcher | Script | Purpose |
|---|---|---|
| `Bash` (`rm` only) | `rm-workspace-guard.sh` | Blocks `rm` on any path outside `~/Workspace/` and `~/.claude/`. |
| `Read` | `preread-edit-check.sh` | When the target file was edited by this session in the last 30 minutes, injects a reminder that the post-edit state is already in context. Info only; never blocks. |
| `Read` | `preread-size-check.sh` | Blocks full-file reads on files over 1,000 lines or 200 KB when no `offset`/`limit` is set. Pushes toward grep-first, then targeted read. Allowlists common small configs (`package.json`, `CLAUDE.md`, `README.md`, etc.). |
| `Read`, `Grep`, `Glob`, `Edit`, `Write`, `NotebookEdit` | `search-counter.sh` | Tracks exploration breadth. Nudges Claude toward the Explore subagent at thresholds 4, 8, and 12 unique file reads or searches. Also increments the global tool counter that the statusline reads. |
| `Bash` | `rtk hook claude` | Routes every Bash command through `rtk` to cut token use. |

### PostToolUse

| Matcher | Script | Purpose |
|---|---|---|
| `Edit`, `Write`, `NotebookEdit` | `post-edit-track.sh` | Appends the edited file's absolute path and timestamp to `edits.jsonl` in the session runtime dir. Increments the edit counter for the statusline. |
| `Edit`, `Write`, `NotebookEdit` | `rebuild-memory-graph.sh` | Rebuilds `~/.claude/memory/graph.json` after memory edits. |

### UserPromptSubmit

| Script | Purpose |
|---|---|
| `auto-model-detect.sh` | Matches design and architecture intent in the prompt (ADR, schema, tradeoff, alternatives, etc.) via regex. When matched, injects context nudging Claude to delegate to an Opus subagent rather than reasoning inline on Sonnet. Skips slash commands and prompts under 20 characters. |

### PreCompact

| Script | Purpose |
|---|---|
| `precompact-warn.sh` | Fires when Claude Code is about to auto-compact. Emits a user-visible warning and a Claude-facing instruction to finish the current sub-task, persist unsaved memory, and suggest a clean restart. Logs the event to `~/.claude/runtime/compactions.log`. |

### Stop / SessionEnd

Both events wire to the same script.

| Script | Purpose |
|---|---|
| `session-clean-exit.sh` | On `Stop` (after every assistant turn), refreshes `last-clean-ts`. On `SessionEnd`, writes a clean-exit marker and emits a memory-flush reminder to Claude so durable facts get persisted before the session closes. |

## See also

- [Authoring Commands, Skills, and Hooks](../authoring/01-commands-skills-hooks.md): how to write your own hook.
- [Internals: Model Routing and Memory](02-model-routing-and-memory.md): model routing and the system prompt.
- [Docs index](../index.md)
