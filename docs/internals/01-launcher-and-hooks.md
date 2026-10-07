# Internals: Launcher and Hooks

The `cc` launcher is the entry point for every session. It wraps `claude` with a system prompt, transcript retention, and config-drift detection. Hooks extend the session lifecycle with guards, nudges, and state tracking. Together they're the machine the rest of the config runs on.

## The `cc`/`ccd` Launcher

The launcher has two thin entry points, `shell/zsh/cc.zsh` and `shell/bash/cc.sh`, one per shell. Each sources the same module files from `shell/shared/` (bust-cache, worktree, config-drift, retention, sessions, clean-resume, dispatch), which define the internal `_claude` dispatcher and the two public functions `cc` and `ccd`. The implementation is shared, so bash and zsh behave identically. `ccd` is `cc` with `--dangerously-skip-permissions` prepended. Nothing else differs.

On every launch, `cc` and `ccd` first run `playbook trust "$PWD"`, so Claude Code's trust dialog never blocks the directory you start in. It is best-effort: it never delays or fails the launch, and an older `playbook` binary without the subcommand is skipped silently.

On every invocation, `cc` passes `--system-prompt-file ~/.config/playbook/prompts/SYSTEM_PROMPT.md` to `claude`. After `claude` exits, it runs `_cc_prune` to keep only the newest `CCD_KEEP` transcripts (default 5, floor 2) per project. Older transcripts plus their sidecars and runtime state are deleted.

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

On every default resume, `cc` computes a SHA-256 hash of `settings.json` and every hook script, then compares it to the hash stored at session start (in `~/.config/playbook/cc-state/<project-slug>`). When they differ, `cc` forks a new transcript so the fresh copy loads the current config. A plain resume fires only when nothing changed.

The `session-init` hook (`playbook hook session-init`) mirrors this: on `source=resume`, it recomputes the hash and emits a user-visible warning when the resumed session is running on the old config. The README states this directly: config or hook edits take effect on a fresh session, not a resumed one. Use `cc fresh` or `cc clean` after editing `settings.json` or any hook.

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

ADR 0007 replaced the old mix of 11 python scripts and 4 bash scripts with a single Rust binary. Every hook is now a module under `src/hooks/`, and Claude Code invokes all 15 of them the same way: `playbook hook <name>`, where `<name>` is the hook's kebab-case form (clap's default `ValueEnum` casing turns the `HookName` variant `RmWorkspaceGuard` into `rm-workspace-guard` on the CLI, and so on for the rest). `src/hooks/mod.rs`'s `dispatch` function matches the parsed `HookName` to that module's `run` entry point. The match is exhaustive, so a new `HookName` variant fails the build until `dispatch` handles it, and a hook can't be silently forgotten.

Per-session state still lives in `~/.config/playbook/runtime/<session_id>/`. The session dir holds counters (`search-count`, `tool-count`, `edit-count`), an `edits.jsonl` log, a `seen-reads` list, timestamps, and the config hash baseline.

### How hooks get registered

The 18 hooks reach `~/.claude/settings.json`'s `.hooks` object through two different paths.

**The 5 always-on safety guards** (`rm-workspace-guard`, `bg-await-guard`, `no-slop-guard`, `precommit-check`, `commit-message-sanitizer`) are wired directly inside `settings.shared.json`, the template `playbook init` seeds or three-way-merges into a user's `settings.json`. Its `PreToolUse` block already carries all five in final `playbook hook <name>` form: `rm-workspace-guard`, `bg-await-guard`, `precommit-check` and `commit-message-sanitizer` on matcher `Bash` (`rm-workspace-guard` and `precommit-check` each scoped further by an `if` condition, `Bash(rm:*)` and `Bash(git commit:*)`), and `no-slop-guard` on both `Bash` and `Edit|Write`, since it checks two different things depending on which tool fired it.

**The other 13 functional hooks** (`session-init`, `preread-edit-check`, `preread-size-check`, `search-counter`, `memory-anchors`, `post-edit-track`, `rebuild-memory-graph`, `auto-model-detect`, `auto-guard`, `auto-cost`, `precompact-warn`, `session-clean-exit`, `memory-capture`) aren't declared anywhere as static JSON. `hooks/hooks.json`, the file that used to register all of them, today registers exactly one thing: a `SessionStart` call to `hooks/migration-check.sh`. That script is a plugin-level check that runs before a `playbook` binary is guaranteed to exist; it greps the user's `settings.json` for the string `playbook hook session-init`, and if that's missing (a plugin update with no matching installer re-run) it warns the user to re-run the installer. It never wires a functional hook itself. `.claude-plugin/plugin.json` carries no `hooks` key at all.

The real mechanism is Rust code. `playbook init` runs a `wire` step (`src/init/wire.rs`) that unconditionally upserts every entry from two hardcoded tables into `.hooks`: `PORTED_HOOK_SPECS` (the 13 functional hooks, 16 registration entries in total, since `memory-anchors` and `auto-guard` each fire on both `PreToolUse` and `UserPromptSubmit`, and `session-clean-exit` fires on both `Stop` and `SessionEnd`) and `GUARD_SPECS` (the same 5 guards `settings.shared.json` already carries, upserted again idempotently in the same bare form; `no-slop-guard` carries two entries since it fires on two matchers). `src/init/run.rs` orders the "settings" step (seed or merge `settings.shared.json` in) before the "hooks" step (`wire`), so `wire` always has a `.hooks` object to upsert into. `wire` recognizes a hook's legacy command too, either the old `<name>.py` path or the old guard `<name>.sh` path, and rewrites that same array slot instead of appending a duplicate, so a machine mid-migration, or a repeat `playbook init` run, self-heals without drift.

The old shell guard scripts (`hooks/precommit-check.sh`, `hooks/no-dash-guard.sh`, `hooks/bg-await-guard.sh`) and their shared `hooks/lib/common.sh` are gone. Every `GUARD_SPECS` entry points at the compiled binary, and `hooks/lib/` now holds only `config-hash.sh`.

### SessionStart

| Hook | Purpose |
|---|---|
| `session-init` | Creates the per-session runtime dir and zeros its counters. Clears the statusline PR/CI cache for the current branch. Checks the config hash and warns on drift when a resumed session is running on stale config. Injects `additionalContext`: the project memory slice, an auto-learn nudge, a skills/commands primer, and the async/deferred-tool discipline reminder. |

### PreToolUse

| Matcher | Hook | Purpose |
|---|---|---|
| `Bash`, only `rm` | `rm-workspace-guard` | Denies an `rm` whose target sits outside the safe roots (the current git repo root by default, or the colon-separated `PLAYBOOK_SAFE_ROOTS` override), `~/.claude/**`, and the scratch trees `/tmp` and `~/.cache` (their contents only, not the roots themselves). Best-effort protection against an accidental `rm`, not a security boundary. |
| `Bash` | `bg-await-guard` | Warns when a Bash call backgrounds an install, build, or typecheck whose output a later step usually needs. Warns only; never blocks. |
| `Bash` | `no-slop-guard` | Denies a posting command that carries an em or en dash, in the command text or in a body file it references. Scoped to posting commands, the last chokepoint before prose reaches GitHub or git history. |
| `Bash`, only `git commit` | `precommit-check` | A mechanical sanity pass over the staged diff before a commit: debug leftovers, secret-shaped filenames, an oversized commit. Warns only; never blocks. |
| `Bash` | `commit-message-sanitizer` | Removes AI attribution (a `Claude-Session:` line, a claude.ai link, a "Generated with" footer, a session id, or a credit trailer whose display name is an AI product or whose address is a vendor or bot no-reply address) from the message of a commit, tag, merge or PR title and body, and an AI `--author`, `--trailer` or identity variable. It never blocks: when something is removed it returns the same command with the message cleaned and no permission decision, so the normal permission flow still applies. A message kept in a file (`-F`, `--body-file`, `cat`, a `<` redirect, `MERGE_MSG`, `SQUASH_MSG`) is never rewritten on disk: the command reads the cleaned text from `-F -` or a heredoc, and a file that is a symlink, over 1 MiB, not UTF-8 or changed earlier in the same call is left to the backstop. Reads `-m`, `-F`, `--body-file`, heredocs, `printf` and `echo` pipes, `$'...'` strings and the message `--amend --no-edit` or `-C` reuses, inside `bash -c`, `eval`, `git rebase --exec`, `env` and other command wrappers, and `git -C`. A recognised `git commit` also gets `-s` so it carries a Signed-off-by line, unless `-s`, `--signoff`, `--no-signoff` or `--dry-run` is given, the message already has one, or the repository's own `prepare-commit-msg` or `commit-msg` hook adds it; signing (`-S`) is left to git config. The same rule backs `playbook sanitize commit-msg <file>` for a git `commit-msg` hook or CI. Best effort against an agent drifting, not a security boundary. |
| `Read` | `preread-edit-check` | When the target file was edited by this session in the last 30 minutes, injects a reminder that the post-edit state is already in context. Info only; never blocks. |
| `Read` | `preread-size-check` | Denies a full-file read of a large file (over the line or byte limit) when no `offset`/`limit` is set, pushing toward grep-first, then a targeted read. Allowlists a small set of config and docs files usually needed whole. The only hook in the toolkit that returns a deny decision. |
| `Read`, `Grep`, `Glob`, `Edit`, `Write`, `NotebookEdit` | `search-counter` | Tracks exploration breadth. Nudges Claude toward the Explore subagent at thresholds 4, 8, and 12 unique file reads or searches. |
| `Edit`, `Write` | `memory-anchors` | When the target path is anchored in the graph-first memory store (`~/.config/playbook/memory/memory.graph.json`), surfaces the facts that describe it, plus their `depends_on` and `contradicts` neighbours, as `additionalContext` before the edit lands. Never blocks. Also fires on `UserPromptSubmit`; see below. |
| `Edit`, `Write` | `no-slop-guard` | Denies an Edit or Write whose new content, in a Rust, shell, or Python file, carries a run of 3 or more consecutive comment lines, or a comment naming a plan, brief, dispatch id, or completion criterion. |

### PostToolUse

| Matcher | Hook | Purpose |
|---|---|---|
| `Edit`, `Write`, `NotebookEdit` | `post-edit-track` | Records the edited file's absolute path and a timestamp to `edits.jsonl` in the session runtime dir. Feeds `preread-edit-check` and the statusline. |
| `Edit`, `Write`, `NotebookEdit` | `rebuild-memory-graph` | Rebuilds `~/.config/playbook/memory/memory.graph.json` after any fact-file save. No-op unless the edited file is inside `~/.config/playbook/memory`. |
| `Bash` | `commit-message-sanitizer` | The backstop for what the PreToolUse rewrite could not see, such as a call another hook rewrote too. It runs git or gh only after a call that runs `git commit` or `git tag`, or `gh pr create`, `new` or `edit`. When that same call made an unpushed HEAD with a message of its own, did not push, and no rebase, merge, cherry-pick or bisect is under way, and its message or author still names an AI, it amends HEAD with the sanitised message (`git commit --amend --only --allow-empty -s -F -`, plus `--reset-author` for an AI author) and adds one line saying what was cleaned. The amend changes the message alone: the index stays as it is, the author and date are kept, the sign-off is added only when missing, and signing follows the repo's own config. An editor-written commit, an older unpushed commit, a pushed commit and an annotated tag are only reported, once per session. After `gh pr create` or `edit` it reads the PR back with `gh pr view` (with the `--repo` the call named) and edits a title or body that still carries attribution. A git process stopped on timeout gets SIGTERM first and is killed only after a short wait; an `index.lock` it leaves behind is reported, never deleted. |

### UserPromptSubmit

| Hook | Purpose |
|---|---|
| `auto-model-detect` | Nudges the main session toward delegating design and architecture-shaped prompts (ADR, schema, tradeoff, alternatives, etc.) to an Opus subagent, rather than reasoning inline on the default model. Skips slash commands and prompts under 20 characters. |
| `memory-anchors` | Matches prompt text and this session's touched files against the same anchor index `PreToolUse` builds, injecting the matched facts' bodies (not just names), deduped per session. Never blocks. |

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

### Optional: a git `commit-msg` hook

`playbook init` does not install a git hook, because a repo may already manage its hooks (a `core.hooksPath`, husky, lefthook) and a second hook would clobber or be clobbered by them. To get the same cleanup for commits made outside an agent, add one line to the repo's own `commit-msg` hook:

```sh
playbook sanitize commit-msg "$1"
```

It rewrites the message file in place and always exits 0, even when the file cannot be read or written, so it never blocks a commit. `--check` reports instead: exit 1 for something it would remove or a missing `Signed-off-by` line, exit 2 when the file cannot be read, so CI can tell them apart.


## See also

- [Authoring Commands, Skills, and Hooks](../authoring/01-commands-skills-hooks.md): how to write your own hook.
- [Internals: Model Routing and Memory](02-model-routing-and-memory.md): model routing and the system prompt.
- [Docs index](../index.md)
