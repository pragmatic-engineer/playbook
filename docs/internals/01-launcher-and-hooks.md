# Launcher and hooks

`ccc` is the entry point for every session. It wraps `claude` with a system prompt, a model fallback chain, an effort ceiling, per-session agent variants, transcript retention and config-drift detection. Hooks add guards, nudges and state tracking around the session. Both are subcommands of the one `playbook` binary.

## The `ccc` and `ccd` launcher

The rc file holds one line: `command -v playbook >/dev/null 2>&1 && eval "$(playbook shell-init)"`. `playbook shell-init` prints two functions, `ccc` and `ccd`, that call `playbook cc launch` (`src/cc/launch.rs`). The same text works in bash and zsh. `ccd` is `ccc` with `--dangerously-skip-permissions` added. Nothing else differs.

A child process cannot change its parent's directory, so the functions create a temp file and pass its path in `PLAYBOOK_CC_CD_FILE`. When the launcher enters a worktree it writes the path there, and the function `cd`s to it after the session ends.

On every launch the launcher:

- Trusts `$PWD` (the equivalent of `playbook trust`), so Claude Code's trust dialog never blocks. Best effort.
- Passes `--system-prompt-file ~/.config/playbook/prompts/SYSTEM_PROMPT.md` when it is installed.
- Passes the model fallback chain with `--fallback-model`, and exports `ANTHROPIC_DEFAULT_<ALIAS>_MODEL` for any `models.*` override. See [Model tiers](02-model-routing-and-memory.md#model-tiers-and-the-55-rule).
- Passes your effort ceiling with `--settings` when playbook's `maxEffortLevel` is the lower one. See [Effort policy](02-model-routing-and-memory.md#effort-policy).
- Renders the agent effort variants into a throwaway session plugin and passes it with `--plugin-dir`. Nothing is committed. `agents.variants` (`auto`, `all`, `off`) picks which, and `playbook agents variants` shows them. `src/cc/agent_variants.rs`.
- Sets `CLAUDE_CODE_DISABLE_AUTO_MEMORY=1` for that session when `memory.source` is `playbook`. It never edits Claude Code's settings or memory.
- After `claude` exits, prunes to the newest `CCD_KEEP` transcripts per project (default 5, floor 2), with their sidecars and runtime state.

A session started without the launcher gets none of this except the hooks.

### Subcommands

| Command | Behavior |
|---|---|
| `ccc` | Resumes the latest session for `$PWD` whose `customTitle` matches the directory name, or starts fresh. Forks a new transcript on config drift. |
| `ccc fresh` | New session, no history. |
| `ccc list` | Recent sessions for `$PWD` with timestamps and titles. |
| `ccc clean` | Clones the latest transcript with `/model`, `/effort`, `/config`, `/output-style` and `/style` overrides stripped, then resumes the clone. The original is untouched. |
| `ccc raw [id]` | Resumes verbatim: no fork, no cleanup. |
| `ccc worktree <branch>` | Creates or enters a git worktree, then starts a session there. See [Worktree engine](03-worktree.md). `ccc new` is an alias. |
| `ccc prune` | Prunes old transcripts now. |

### Config-drift detection

On a default resume, the launcher hashes `settings.json` and compares it to the hash stored at session start (`~/.config/playbook/cc-state/<project-slug>`). When they differ, it forks a new transcript so the fresh copy loads the current config. The `session-init` hook does the same check on `source=resume` and warns when a resumed session runs on old config. Edits to `settings.json` or hooks take effect on a fresh session, so use `ccc fresh` or `ccc clean` after editing them. The hash lives in `src/common/config_hash.rs`.

## The hook lifecycle

Every hook is a module under `src/hooks/`, run as `playbook hook <name>` (see ADR 0007). `dispatch` in `src/hooks/mod.rs` matches every `HookName` exhaustively, so a new hook cannot be forgotten. Per-session state lives in `~/.config/playbook/runtime/<session_id>/`: counters, an `edits.jsonl` log, a `seen-reads` list, timestamps and the config hash baseline.

### How hooks get registered

- **6 always-on guards** (`rm-workspace-guard`, `bg-await-guard`, `no-slop-guard`, `precommit-check`, `commit-message-sanitizer`, `policy-guard`) are in `settings.shared.json`, which `playbook init` seeds or three-way merges into `settings.json`. All six are `PreToolUse` on `Bash`, and `no-slop-guard` and `policy-guard` are also on `Edit|Write`. `rm-workspace-guard` and `precommit-check` carry an `if` condition (`Bash(rm:*)`, `Bash(git commit:*)`).
- **13 functional hooks** (`session-init`, `preread-edit-check`, `preread-size-check`, `search-counter`, `memory-anchors`, `post-edit-track`, `rebuild-memory-graph`, `auto-model-detect`, `auto-guard`, `auto-cost`, `precompact-warn`, `session-clean-exit`, `memory-capture`) have no static JSON. `playbook init` runs a `wire` step (`src/init/wire.rs`) that upserts them from two tables, `PORTED_HOOK_SPECS` and `GUARD_SPECS`. The `PostToolUse` backstop of `commit-message-sanitizer` lives in `GUARD_SPECS` only, so `wire` adds it on every `init`.
- **2 worktree hooks** (`worktree-create`, `worktree-remove`) are registered by the plugin's `hooks/hooks.json`, never by `wire`, so they cannot fire twice.
- **1 plugin check**: the plugin's `SessionStart` entry runs `bin/playbook hook migration-check`. It warns when `settings.json` has no `playbook hook session-init`, which means a plugin update without an `init` re-run. It wires nothing.

`init` runs the settings step before the hooks step, so `wire` always has a `.hooks` object. `wire` also recognises legacy `.py` and `.sh` commands and rewrites that slot, so an upgrade or a repeat `init` self-heals. All the old shell and Python scripts are gone.

### SessionStart

| Hook | Purpose |
|---|---|
| `session-init` | Creates the per-session runtime dir and zeros its counters. Clears the statusline PR/CI cache for the current branch. Checks the config hash and warns on drift when a resumed session is running on stale config. Injects `additionalContext`: an auto-learn nudge, a handoff, and one small ranked memory block (pinned, most used, most linked facts, capped near 2,500 characters). The whole string stays under 9,000 characters because Claude Code replaces a hook string over 10,000 with a 2,000 character preview. Read only: it never writes to the memory store. |

### PreToolUse

| Matcher | Hook | Purpose |
|---|---|---|
| `Bash`, only `rm` | `rm-workspace-guard` | Denies an `rm` whose target sits outside the safe roots (the current git repo root by default, or the colon-separated `PLAYBOOK_SAFE_ROOTS` override), `~/.claude/**`, and the scratch trees `/tmp` and `~/.cache` (their contents only, not the roots themselves). Best-effort protection against an accidental `rm`, not a security boundary. |
| `Bash` | `bg-await-guard` | Warns when a Bash call backgrounds an install, build, or typecheck whose output a later step usually needs. Warns only; never blocks. |
| `Bash` | `no-slop-guard` | Denies a posting command that carries an em or en dash, in the command text or in a body file it references. Scoped to posting commands, the last chokepoint before prose reaches GitHub or git history. |
| `Bash`, only `git commit` | `precommit-check` | A mechanical sanity pass over the staged diff before a commit: debug leftovers, secret-shaped filenames, an oversized commit. Warns only; never blocks. |
| `Bash` | `commit-message-sanitizer` | Removes AI attribution (a `Claude-Session:` line, a claude.ai link, a "Generated with" footer, an AI credit trailer or `--author`) from the message of a commit, tag, merge or PR. It never blocks: it returns the same command with the message cleaned and no permission decision. A message kept in a file is never rewritten on disk; the cleaned text is read from `-F -` or a heredoc. A recognised `git commit` also gets `-s` unless the message already has a sign-off, `commit.signOff` is false, or the repo's own hook adds it. Signing (`-S`) is left to git config. The same rule backs `playbook sanitize commit-msg <file>`. Best effort, not a security boundary. |
| `Bash`, `Edit`, `Write` | `policy-guard` | Denies skipping git hooks or signing (`--no-verify`, `git commit -n`, `--no-gpg-sign`, `-c commit.gpgsign=false`), a hand-run `gh pr create` (use `/playbook:create-pull-request`, which calls `playbook pr create`), and a write to Claude Code's own memory (`~/.claude/projects/*/memory/`, `~/.claude/memory/`) from Bash, Edit or Write. Reads are allowed. Fails safe: input it cannot read is allowed. `POLICY_GUARD=0` turns it off. |
| `Read` | `preread-edit-check` | When the target file was edited by this session in the last 30 minutes, injects a reminder that the post-edit state is already in context. Info only; never blocks. |
| `Read` | `preread-size-check` | Denies a full-file read of a large file (over the line or byte limit) when no `offset`/`limit` is set, pushing toward grep-first, then a targeted read. Allowlists a small set of config and docs files usually needed whole. |
| `AskUserQuestion` | `auto-guard` | Only when the resolved mode is `auto`: denies the question tool so the model takes the recommended option. See [Auto mode](../guides/05-auto-mode.md). |
| any tool | `auto-cost` | Only in auto mode: warns at `auto.warnPct` of `auto.budgetUsd`, and at the cap denies every tool except reads, `git status`, `git diff`, `playbook mode status` and a park note. See [Auto mode](../guides/05-auto-mode.md). |
| `Read`, `Grep`, `Glob`, `Edit`, `Write`, `NotebookEdit` | `search-counter` | Tracks exploration breadth. Nudges Claude toward the Explore subagent once, at 8 unique file reads or searches. |
| `Edit`, `Write` | `memory-anchors` | When the target path is anchored in the graph-first memory store (`~/.config/playbook/memory/memory.graph.json`), surfaces the facts that describe it, plus their `depends_on` and `contradicts` neighbours, as `additionalContext` before the edit lands. Never blocks. Also fires on `UserPromptSubmit`; see below. |
| `Edit`, `Write` | `no-slop-guard` | Denies an Edit or Write whose new content, in a Rust, shell, or Python file, carries a run of 3 or more consecutive comment lines, or a comment naming a plan, brief, dispatch id, or completion criterion. |

### PostToolUse

| Matcher | Hook | Purpose |
|---|---|---|
| `Edit`, `Write`, `NotebookEdit` | `post-edit-track` | Records the edited file's absolute path and a timestamp to `edits.jsonl` in the session runtime dir. Feeds `preread-edit-check` and the statusline. |
| `Edit`, `Write`, `NotebookEdit` | `rebuild-memory-graph` | Rebuilds `~/.config/playbook/memory/memory.graph.json` after any fact-file save. No-op unless the edited file is inside `~/.config/playbook/memory`. |
| `Bash` | `commit-message-sanitizer` | Removes AI attribution (a `Claude-Session:` line, a claude.ai link, a "Generated with" footer, an AI credit trailer or `--author`) from the message of a commit, tag, merge or PR. It never blocks: it returns the same command with the message cleaned and no permission decision. A message kept in a file is never rewritten on disk; the cleaned text is read from `-F -` or a heredoc. A recognised `git commit` also gets `-s` unless the message already has a sign-off, `commit.signOff` is false, or the repo's own hook adds it. Signing (`-S`) is left to git config. The same rule backs `playbook sanitize commit-msg <file>`. Best effort, not a security boundary. |

### UserPromptSubmit

| Hook | Purpose |
|---|---|
| `auto-model-detect` | Nudges the main session toward delegating design and architecture-shaped prompts (ADR, schema, tradeoff, alternatives, etc.) to an Opus subagent, rather than reasoning inline on the default model. Skips slash commands and prompts under 20 characters. |
| `auto-guard` | In auto mode with the default permission mode, advises a trusted permission mode once per session. |
| `memory-anchors` | Matches prompt text and this session's touched files against the same anchor index `PreToolUse` builds, injecting each matched fact's name, description and body (body cut at 1,500 characters), deduped per session. Common words are ignored, at most 3 facts per prompt and 9 per session. Never blocks. |

### PreCompact

| Hook | Purpose |
|---|---|
| `precompact-warn` | Fires when Claude Code is about to auto-compact. Emits a user-visible warning and logs the event to `~/.config/playbook/runtime/compactions.log`, since `PreCompact` has no `additionalContext` channel to speak to Claude directly. |

### Stop / SessionEnd

| Event | Hook | Purpose |
|---|---|---|
| `Stop` | `session-clean-exit` | Refreshes `last-clean-ts` after every assistant turn, so a stale-session check only fires when a session is genuinely abandoned. |
| `Stop` | `memory-capture` | When the statusline has dropped a `capture-due` marker in the session dir, pauses the turn with a block decision asking the model to persist durable facts before continuing. |
| `SessionEnd` | `session-clean-exit` | Writes a clean-exit marker so the next session's `session-init` hook can tell a graceful exit from an orphaned, crashed one. |

### WorktreeCreate / WorktreeRemove

| Event | Hook | Purpose |
|---|---|---|
| `WorktreeCreate` | `worktree-create` | Replaces Claude Code's default worktree creation so worktrees land in `<main-parent>/.worktrees/<repo>/<name>`, the `ccc worktree` launcher convention, instead of `.claude/worktrees/`. Prints only the absolute path on stdout. Falls back to `.claude/worktrees/<name>` if the primary location fails. See [03-worktree.md](03-worktree.md). |
| `WorktreeRemove` | `worktree-remove` | Removes the worktree only when it is clean and every commit is on a remote-tracking ref or the main branch. Otherwise keeps it and says why on stderr. Never forces. |

### Optional: a git `commit-msg` hook

`playbook init` does not install a git hook, because a repo may already manage its hooks (a `core.hooksPath`, husky, lefthook) and a second hook would clobber or be clobbered by them. To get the same cleanup for commits made outside an agent, add one line to the repo's own `commit-msg` hook:

```sh
playbook sanitize commit-msg "$1"
```

It rewrites the message file in place and always exits 0, even when the file cannot be read or written, so it never blocks a commit. `--check` reports instead: exit 1 for something it would remove or a missing `Signed-off-by` line, exit 2 when the file cannot be read, so CI can tell them apart.

### Optional: a git `commit-msg` hook

`playbook init` installs no git hook, because a repo may already manage its hooks (`core.hooksPath`, husky, lefthook). To get the same cleanup for commits made outside an agent, add one line to the repo's own `commit-msg` hook:

```sh
playbook sanitize commit-msg "$1"
```

It rewrites the message file in place and always exits 0. With `--check` it reports instead: exit 1 for something it would remove or a missing `Signed-off-by` line, exit 2 when the file cannot be read, so CI can tell them apart.

## Status line

`statusLine.command` runs `playbook statusline`, the Rust renderer. `playbook init` rewrites the old `statusline.sh` command and leaves a custom command alone. The script is gone from the repo, and `playbook uninstall` still removes a copy an older install placed.

## See also

- [Authoring commands, skills and hooks](../authoring/01-commands-skills-hooks.md): how to write your own hook.
- [Model routing and memory](02-model-routing-and-memory.md): model tiers, effort and the memory graph.
- [Docs index](../index.md)
