# Launcher and hooks

`playbook cc` is the entry point for every session. It wraps `claude` with a system prompt, a model fallback chain, an effort ceiling, per-session agent variants, transcript retention and config-drift detection. Hooks add guards, nudges and state tracking around the session. Both are subcommands of the one `playbook` binary.

## The `playbook cc` and `playbook ccd` launcher

`playbook cc` and `playbook ccd` run `playbook cc launch` (`crates/cli/src/cc/launch.rs`) straight from the binary, and so do `pb cc` and `pb ccd`. No shell line is needed ([ADR 0044](../adr/0044-drop-the-shell-launcher.md)). `playbook ccd` is `playbook cc` with `--dangerously-skip-permissions` added. Nothing else differs.

A child process cannot change its parent's directory. After a session in a worktree that `playbook cc launch worktree <branch>` created or entered, the launcher prints the `cd` command to run, quoted when the path needs it. The `playbook shell-init` command is removed. Until 1.0, a run of it prints one line on stderr that says to delete the rc line, prints nothing on stdout and exits 0, so an old rc line does not break shell startup. The `retired-launcher-rc-line` migration removes the lines older setups wrote, after a backup, when you run `playbook setup`.

On every launch the launcher:

- Trusts `$PWD` (the equivalent of `playbook trust`), so Claude Code's trust dialog never blocks. Best effort.
- Passes `--system-prompt-file ~/.config/playbook/prompts/SYSTEM_PROMPT.md` when it is installed and `systemPrompt.enabled` is true, the default. With the key false it adds no prompt flag and passes your own prompt flags through, append flags included.
- Passes the model fallback chain with `--fallback-model`, and exports `ANTHROPIC_DEFAULT_<ALIAS>_MODEL` for any `models.*` override. See [Model tiers](02-model-routing-and-memory.md#model-tiers-and-the-55-rule).
- Passes no `--settings`. Your effort ceiling lowers the default `--effort` the launcher adds and the agent variants it renders. It moves `CLAUDE_CODE_EFFORT_LEVEL` to `--effort` so the variable cannot raise a playbook skill, command or agent above its own effort. See [Effort policy](02-model-routing-and-memory.md#effort-policy).
- Renders the agent effort variants into a throwaway session plugin and passes it with `--plugin-dir`. Nothing is committed. `agents.variants` (`auto`, `all`, `off`) picks which, and `playbook agents variants` shows them. `crates/cli/src/cc/agent_variants.rs`.
- Sets `CLAUDE_CODE_DISABLE_AUTO_MEMORY=1` for that session when `memory.source` is `playbook`. It never edits Claude Code's settings or memory.
- After `claude` exits, prunes to the newest `CCD_KEEP` transcripts per project (default 5, floor 2), with their sidecars and runtime state.

A session started without the launcher, such as a plain `claude`, gets none of this except the hooks and the status line: no playbook prompt, no effort default, no agent variants, no model overrides and no memory switch. The `claude` function that `launcher.claudeOverride` turned on is gone with the shell line.

### The `pb` short name

`pb` is a symlink to the `playbook` binary. `install.sh` links `$PLAYBOOK_BIN_DIR/pb`, and the Homebrew formula installs one with `bin.install_symlink`. `playbook setup --short-name` makes the same relative link in the install folder for other installs, only when no other `pb` is on your PATH, and stores the answer as `launcher.shortName`. `setup --yes` makes the link too, unless a no is stored.

### Subcommands

| Command | Behavior |
|---|---|
| `playbook cc` | Resumes the latest session for `$PWD` whose `customTitle` matches the directory name, or starts fresh. Forks a new transcript on config drift. |
| `playbook cc fresh` | New session, no history. |
| `playbook cc list` | Recent sessions for `$PWD` with timestamps and titles. |
| `playbook cc clean` | Clones the latest transcript with `/model`, `/effort`, `/config`, `/output-style` and `/style` overrides stripped, then resumes the clone. The original is untouched. |
| `playbook cc raw [id]` | Resumes verbatim: no fork, no cleanup. |
| `playbook cc launch worktree <branch>` | Creates or enters a git worktree, then starts a session there. See [Worktree engine](03-worktree.md). `playbook cc launch new` is an alias. |
| `playbook cc prune` | Prunes old transcripts now. |

### Config-drift detection

On a default resume, the launcher hashes `settings.json` and compares it to the hash stored at session start (`~/.config/playbook/cc-state/<project-slug>`). When they differ, it forks a new transcript so the fresh copy loads the current config. The `session-init` hook does the same check on `source=resume` and warns when a resumed session runs on old config. Edits to `settings.json` or hooks take effect on a fresh session, so use `playbook cc fresh` or `playbook cc clean` after editing them. The hash lives in `crates/cli/src/common/config_hash.rs`.

## The hook lifecycle

Every hook is a module under `crates/cli/src/hooks/`, run as `playbook hook <name>` (see ADR 0007). `dispatch` in `crates/cli/src/hooks/mod.rs` matches every `HookName` exhaustively, so a new hook cannot be forgotten. Per-session state lives in `~/.config/playbook/runtime/<session_id>/`: counters, an `edits.jsonl` log, a `seen-reads` list, a `read-state.jsonl` log of full Reads, timestamps and the config hash baseline.

### How hooks get registered

- **6 always-on guards** (`rm-workspace-guard`, `bg-await-guard`, `no-slop-guard`, `precommit-check`, `commit-message-sanitizer`, `policy-guard`) are in `settings.shared.json`, which `playbook setup` seeds or three-way merges into `settings.json`. All six are `PreToolUse` on `Bash`, and `no-slop-guard` and `policy-guard` are also on `Edit|Write`. `precommit-check` carries an `if` condition (`Bash(git commit:*)`). `rm-workspace-guard` runs for every Bash command, since a deletion need not hold an `rm` word.
- **18 functional hooks** (`session-init`, `preread-edit-check`, `preread-size-check`, `search-counter`, `memory-anchors`, `post-edit-track`, `rebuild-memory-graph`, `auto-model-detect`, `auto-guard`, `auto-cost`, `precompact-warn`, `session-clean-exit`, `memory-capture`, `context-hygiene`, `doc-read-hint`, `usage-warm`, `worktree-create`, `worktree-remove`) have no static JSON. `playbook setup` runs a `wire` step (`crates/cli/src/init/wire.rs`) that upserts them from two tables, `PORTED_HOOK_SPECS` and `GUARD_SPECS`. The `PostToolUse` backstop of `commit-message-sanitizer` lives in `GUARD_SPECS` only, so `wire` adds it on every `setup`.

`setup` runs the settings step before the hooks step, so `wire` always has a `.hooks` object. `wire` rewrites only an entry whose command is exactly `playbook hook <name>`, so a repeat `setup` self-heals. A user's own `.py` or `.sh` command is never touched, so an older install's `~/.claude/hooks/<guard>.sh` entries stay next to the new bare commands until you delete them by hand. All the old shell and Python scripts are gone.

### Pausing when the plugin is disabled

`playbook setup` writes the hook entries and `statusLine` into your user `settings.json`. Disabling the plugin in Claude Code sets `enabledPlugins["playbook@pragmatic-engineer"]` to `false` and leaves those entries, so playbook checks the key itself. `common::plugin_state::paused()` reads `~/.claude/settings.json` once per call, from the one place every hook (`run_hook` in `crates/cli/src/main.rs`) and `playbook statusline` go through. When the key is explicitly `false`, a hook exits 0 with no output and no side effects, and the status line prints nothing.

Only an explicit `false` pauses. A missing key, `true`, a missing file and a file that cannot be read or parsed all count as enabled, so Homebrew-only users and broken settings keep their guards. The safety guards pause too, which is intended: a disabled plugin means no playbook behaviour. The two worktree hooks are the exception and always run: they stand in for Claude Code's own worktree handling, and a skipped `WorktreeCreate` hook would print no path.

Set `PLAYBOOK_IGNORE_PLUGIN_DISABLED=1` to run hooks and the status line anyway. The check skips the JSON parse unless the settings text mentions the plugin key, and measured about 40 microseconds on a 7 KB `settings.json`.

### SessionStart

| Hook | Purpose |
|---|---|
| `session-init` | Creates the per-session runtime dir and zeros its counters. Clears the statusline PR/CI cache for the current branch. Checks the config hash and warns on drift when a resumed session is running on stale config. Injects `additionalContext`: the tool map (what `playbook tools map` prints, on every start including headless), an auto-learn nudge, a handoff (and, after `/clear`, the one the ended session saved for itself, see `context-hygiene`), and one small ranked memory block (pinned, most used, most linked facts, capped near 2,500 characters). The whole string stays under 9,000 characters because Claude Code replaces a hook string over 10,000 with a 2,000 character preview. Read only: it never writes to the memory store. |
| `usage-warm` | Starts a detached, low priority, time bounded `playbook usage warm` when the last finished run is older than `usage.autoIngestMinutes`, so the usage store is filled before you open `playbook usage`. Returns at once with no output. Off with `usage.autoIngest` false. |

**The handoff loop.** `context-hygiene` (UserPromptSubmit) asks the model to run `playbook handoff save --session <id>`. That file is named `<dir>-<epoch>-<pid>-<id>.md` under `~/.config/playbook/runtime/handoff/`, keyed by directory like any handoff. When the session ends with `/clear`, `session-clean-exit` (SessionEnd, reason `clear`) drops a marker in `handoff/cleared/`. The next `session-init` with source `clear` takes the tagged handoff whose marker is freshest, injects at most 3,000 characters of it with the path of the full file (moved to `handoff/used/`), and removes the marker, so it loads once. A tagged handoff with no marker belongs to a session still running and is left alone. Tagged handoffs expire after a day and markers after an hour. Handoffs saved without `--session` behave as before. `compact` does not load one, because compaction keeps the session and its summary.

### PreToolUse

| Matcher | Hook | Purpose |
|---|---|---|
| `Bash`, only `rm` | `rm-workspace-guard` | Denies an `rm` whose target sits outside the safe roots (the current git repo root by default, or the colon-separated `PLAYBOOK_SAFE_ROOTS` override), `~/.claude/**`, playbook's install files by name, and the scratch trees (their contents only, never the root itself or a parent): the temp and cache folders for the OS and playbook's scratch folders from `rm_workspace_guard::roots`, the `.worktrees` folder next to the repo, `WORKTREE_BASE_DIR` and the `hooks.rmGuard.allowedRoots` config list. Playbook's data (memory store, databases, plans, handoffs) and lock folders stay blocked under any root. The command is read into real command words by `rm_workspace_guard::parse` on the shared shell lexer. Symlinks and `..` are resolved before a target is judged, and a link is judged both by where it sits and where it points. A bare glob such as `/tmp/*` is denied, a named prefix such as `/tmp/pb-*` is allowed. Best-effort protection against an accidental `rm`, not a security boundary. The deny message names the rule that matched for each target, where it resolved to, and for a plain folder in home the exact `config set` command with the current roots plus the new one. See [ADR 0045](../adr/0045-rm-guard-roots-and-parser.md). |
| `Bash` | `bg-await-guard` | Warns when a Bash call backgrounds an install, build, or typecheck whose output a later step usually needs. Also nudges a foreground `gh run watch`, `gh pr checks --watch`, or `cargo test`/`build`/`nextest`/`clippy` that sets no timeout, to pass a longer timeout or use `run_in_background`, since the default 2 minute timeout kills it. Warns only; never blocks. |
| `Bash` | `no-slop-guard` | Denies a posting command that carries an em or en dash, in the command text or in a body file it references (`gh api` only when it sends data). Scoped to posting commands, the last chokepoint before prose reaches GitHub or git history. |
| `Bash`, only `git commit` | `precommit-check` | A mechanical sanity pass over the staged diff before a commit: debug leftovers, secret-shaped filenames, an oversized commit. Warns only; never blocks. |
| `Bash` | `commit-message-sanitizer` | Removes AI attribution (a `Claude-Session:` line, a claude.ai link, a "Generated with" footer, an AI credit trailer or `--author`) from the message of a commit, tag, merge or PR. Removing attribution never blocks: it returns the same command with the message cleaned and no permission decision. A message kept in a file is never rewritten on disk; the cleaned text is read from `-F -` or a heredoc. A recognised `git commit` also gets `-s` unless the message already has a sign-off, `commit.signOff` is false, or the repo's own hook adds it. Signing (`-S`) is left to git config. The same rule backs `playbook sanitize commit-msg <file>`. It also lints the prose of those messages, of PR text, and of `gh pr comment`, `gh pr review` and `gh issue` text against the writing-style phrase and structure table, and adds the findings to the model's context. `hooks.proseLint.mode` set to `block` denies the call on a hard finding. The same table runs on files with `playbook prose check`. Best effort, not a security boundary. |
| `Bash`, `Edit`, `Write` | `policy-guard` | Denies skipping git hooks or signing (`--no-verify`, `git commit -n`, `--no-gpg-sign`, `-c commit.gpgsign=false`), a hand-run `gh pr create` (use `playbook deliver`, or `/playbook:create-pull-request`, which writes the title and body and opens the PR), and a write to Claude Code's own memory (`~/.claude/projects/*/memory/`, `~/.claude/memory/`) from Bash, Edit or Write. It denies every write to a Claude Code settings file (`settings.json` or `settings.local.json`, user or project): an Edit or Write, or a Bash redirect, `tee`, `sed -i`, `perl -pi`, `cp`, `mv`, `install`, `rsync`, `ln`, `dd`, `rm`, `truncate` or interpreter one-liner. It reads the command text, so heredoc scripts, script files and paths built at run time get through. Settings change through `playbook setup`, `uninstall`, `config set` and `doctor`. In Bash, `playbook uninstall --yes` is denied. The user runs those with `!`. Reads are allowed. Fails safe: input it cannot read is allowed. `POLICY_GUARD=0` turns it off. |
| `Read` | `preread-edit-check` | When the target file was edited by this session in the last 30 minutes, injects a reminder that the post-edit state is already in context. Info only; never blocks. |
| `Read` | `preread-size-check` | Denies a full-file read of a large file (over 500 lines or 60,000 bytes) when no `offset`/`limit` is set, pushing toward grep-first, then a targeted read. The note also tells the model to pipe big command output through `head` or `tail` and to use `gh ... --json` with `jq`. Images (png, jpg, jpeg, gif, webp) and PDFs are never denied, since `offset` and `limit` cannot override the guard for them. Allowlists a small set of config and docs files usually needed whole. It also denies a full re-read of a file this session already read in full, when the file is unchanged (same mtime and size), has at least 150 lines, and no compaction happened since (`session-init` zeroes `read-state.jsonl` on every SessionStart, compact included). Never denies a Read with `offset`, `limit` or `pages`, an image, PDF or notebook, a subagent Read, or a file whose earlier read was partial. The note names Grep and offset/limit, and `offset:1` with a `limit` forces the re-read. |
| `AskUserQuestion` | `auto-guard` | Only when the resolved mode is `auto`: denies the question tool so the model takes the recommended option. See [Auto mode](../guides/05-auto-mode.md). |
| any tool | `auto-cost` | Only in auto mode: per agent, warns at `auto.warnPct` of that agent's cap and at the cap denies new work except reads, the closing steps (commit, push, pull request skills and commands, read-only `gh`, `playbook mode status`) and its own park note. The caller comes from `agent_id` in the hook input, and `auto.sessionCeilingUsd` stops new `Agent` spawns for the tree. See [Auto mode](../guides/05-auto-mode.md). |
| `Read`, `Grep`, `Glob`, `Edit`, `Write`, `NotebookEdit` | `search-counter` | Tracks exploration breadth. Nudges Claude toward the Explore subagent once, at 8 unique file reads or searches. |
| `Edit`, `Write` | `memory-anchors` | When the target path is anchored in the graph-first memory store (`~/.config/playbook/data/memory.db`), surfaces the facts that describe it, plus their `depends_on` and `contradicts` neighbours, as `additionalContext` before the edit lands. The message is capped at 600 characters (extra facts are summarized as a count) and is emitted once per file per session. Never blocks. Also fires on `UserPromptSubmit`; see below. |
| `Edit`, `Write` | `no-slop-guard` | Denies an Edit or Write whose new content, in a Rust, shell, or Python file, carries a comment naming a plan, brief, dispatch id, or completion criterion, or a comment line wider than 100 columns (a lone URL is exempt). For Write, only lines that differ from the file on disk are checked. |

### PostToolUse

| Matcher | Hook | Purpose |
|---|---|---|
| `Edit`, `Write`, `NotebookEdit` | `post-edit-track` | Records the edited file's absolute path and a timestamp to `edits.jsonl` in the session runtime dir. Feeds `preread-edit-check` and the statusline. |
| `Edit`, `Write`, `NotebookEdit` | `rebuild-memory-graph` | Rebuilds the graph in `~/.config/playbook/data/memory.db` after any fact-file save. No-op unless the edited file is inside `~/.config/playbook/memory`. |
| `Bash` | `commit-message-sanitizer` | Removes AI attribution (a `Claude-Session:` line, a claude.ai link, a "Generated with" footer, an AI credit trailer or `--author`) from the message of a commit, tag, merge or PR. It never blocks: it returns the same command with the message cleaned and no permission decision. A message kept in a file is never rewritten on disk; the cleaned text is read from `-F -` or a heredoc. A recognised `git commit` also gets `-s` unless the message already has a sign-off, `commit.signOff` is false, or the repo's own hook adds it. Signing (`-S`) is left to git config. The same rule backs `playbook sanitize commit-msg <file>`. Best effort, not a security boundary. |

### UserPromptSubmit

| Hook | Purpose |
|---|---|
| `auto-model-detect` | Nudges the main session toward delegating design and architecture-shaped prompts (ADR, schema, tradeoff, alternatives, etc.) to an Opus subagent, rather than reasoning inline on the default model. Skips slash commands and prompts under 20 characters. |
| `auto-guard` | In auto mode with the default permission mode, advises a trusted permission mode once per session. |
| `context-hygiene` | Advisory only: it never clears or compacts. Reads the input token size of the newest main-session assistant message from the tail of the transcript. Past `hygiene.thresholdTokens` (default 250,000; `hygiene.enabled` turns it off) it injects one message under 400 characters per crossing: save a handoff now with `playbook handoff save --session <id>`, then tell the user in one line that the handoff is saved and to run `/clear`, then `/playbook:session-start` if it did not load on its own. The message says the automatic load needs the SessionStart hook. The last level fired is kept in `hygiene-level` in the session dir, so the same level never repeats and a session that shrinks and grows again is nudged again. Any transcript problem means silence. Skipped in headless runs. |
| `doc-read-hint` | Advisory only. On a Read of a whole PDF or an Office, EPUB, RTF or OpenDocument file it adds one line that suggests `playbook doc read <file>`, only when `docs.convert` is `auto` and `anydoc` is on PATH. It never rewrites the Read input and never denies. It stays quiet for a PDF Read with `pages`, `offset` or `limit`, and for images, text and CSV. Paused with the rest of playbook when the plugin is disabled. |
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
| `SessionEnd` | `usage-warm` | The same background read, so a finished session is already stored. |

### WorktreeCreate / WorktreeRemove

| Event | Hook | Purpose |
|---|---|---|
| `WorktreeCreate` | `worktree-create` | Replaces Claude Code's default worktree creation so worktrees land in `<main-parent>/.worktrees/<repo>/<name>`, the `playbook cc launch worktree` launcher convention, instead of `.claude/worktrees/`. Prints only the absolute path on stdout. Falls back to `.claude/worktrees/<name>` if the primary location fails. See [03-worktree.md](03-worktree.md). |
| `WorktreeRemove` | `worktree-remove` | Removes the worktree only when it is clean and every commit is on a remote-tracking ref or the main branch. Otherwise keeps it and says why on stderr. Never forces. |

### Optional: a git `commit-msg` hook

`playbook setup` does not install a git hook, because a repo may already manage its hooks (a `core.hooksPath`, husky, lefthook) and a second hook would clobber or be clobbered by them. To get the same cleanup for commits made outside an agent, add one line to the repo's own `commit-msg` hook:

```sh
playbook sanitize commit-msg "$1"
```

It rewrites the message file in place and always exits 0, even when the file cannot be read or written, so it never blocks a commit. `--check` reports instead: exit 1 for something it would remove or a missing `Signed-off-by` line, exit 2 when the file cannot be read, so CI can tell them apart.

### Optional: a git `commit-msg` hook

`playbook setup` installs no git hook, because a repo may already manage its hooks (`core.hooksPath`, husky, lefthook). To get the same cleanup for commits made outside an agent, add one line to the repo's own `commit-msg` hook:

```sh
playbook sanitize commit-msg "$1"
```

It rewrites the message file in place and always exits 0. With `--check` it reports instead: exit 1 for something it would remove or a missing `Signed-off-by` line, exit 2 when the file cannot be read, so CI can tell them apart.

## Status line

`statusLine.command` runs `playbook statusline`, the Rust renderer. `playbook setup` rewrites the old `statusline.sh` command and leaves a custom command alone. The script is gone from the repo, and `playbook uninstall` still removes a copy an older install placed.

## See also

- [Authoring commands, skills and hooks](../authoring/01-commands-skills-hooks.md): how to write your own hook.
- [Model routing and memory](02-model-routing-and-memory.md): model tiers, effort and the memory graph.
- [Docs index](../index.md)
