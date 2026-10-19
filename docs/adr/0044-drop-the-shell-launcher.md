# ADR-0044: Drop the Shell Launcher and Turn Every Setup Part On With `--yes`

- **Status:** Accepted. Amended on 2026-10-19, see [Amendment 2026-10-19: remove `shell-init`](#amendment-2026-10-19-remove-shell-init). The `collector-service` row of decision 5 and the collector start row below are superseded by [ADR 0049](0049-collector-daemon-started-by-commands.md): the part is gone, and playbook commands and hooks start the collector.
- **Date created:** 2026-10-18
- **Date modified:** 2026-10-20
- **Amends:** [ADR 0007](0007-rust-binary-for-hooks-and-launcher.md), which kept a small shell function per shell family for the `cd` into a worktree. Decision 6 of [ADR 0034](0034-self-healing-upgrades.md), for migration `0007-shell-init-rc-line`. The `--yes` defaults in [ADR 0033](0033-telemetry-on-by-default.md), [ADR 0042](0042-attribution-off-in-setup.md) and [ADR 0043](0043-collector-service-in-setup.md), which this record extends to every part. Decision 5 of [ADR 0040](0040-rename-init-to-setup.md), which kept the `shell-init` name, through the amendment of 2026-10-19.

## Context

`playbook setup` has three optional parts that act through one line in the shell rc file:

- `aliases` adds `command -v playbook >/dev/null 2>&1 && eval "$(playbook shell-init)"` to `~/.zshrc` or `~/.bashrc`, under the marker comment `# playbook launchers (ccc/ccd)`. `playbook shell-init` prints a `playbook` shell function. After `playbook cc` or `playbook ccd` enters a worktree, the function changes the shell's directory to it, through the file named by `PLAYBOOK_CC_CD_FILE`. It also defines `ccc` and `ccd` stubs that name their replacements.
- `claude-override` sets `launcher.claudeOverride`. `shell-init` then also defines a `claude` function (`crates/cli/src/init/claude_override.sh`). A plain interactive `claude` runs `playbook cc launch -- <args>`. Piped use, `-p`, subcommands, `--max-budget-usd`, the user's own prompt or settings flags and `PLAYBOOK_BYPASS=1` go straight to the real binary.
- `short-name` sets `launcher.shortName`. `shell-init` then also defines `pb` as `playbook` when nothing else uses the name.

All three are off by default and work only once the `aliases` line is in the rc file. The system prompt part is off by default too. With `--yes` a new install gets no system prompt, so `playbook cc` starts with Claude Code's own prompt. `install.sh` asks about the system prompt only after a yes to the shell wrapper, and passes `--aliases`, `--system-prompt` and `--yes` to setup. The `pb` command already exists on the two main channels: `install.sh` links `$PLAYBOOK_BIN_DIR/pb` to `playbook`, and the Homebrew formula installs a `pb` symlink with `bin.install_symlink`.

The user decided: the shell line goes. `pb cc` must work and load the playbook system prompt by default. `--yes` turns on every feature. A user can still turn the system prompt off in setup, and then `pb cc` starts plain Claude Code.

## What only worked through the shell line

| Behavior | Where it lived | After this change |
|---|---|---|
| `cd` into the worktree after `playbook cc launch worktree <branch>`, `playbook ccd worktree <branch>` and the other launch forms that enter a worktree | the `playbook` shell function and `PLAYBOOK_CC_CD_FILE` | Lost. A child process cannot change its parent's directory. The session still runs in the worktree, and `playbook cc` prints the `cd` command after the session ends. `playbook cc worktree <branch>` never moved the shell: the function passed it straight to the binary, which only prints the path. |
| `ccc` and `ccd` stubs that name `playbook cc` and `playbook ccd` | `shell-init` | Lost. `playbook doctor` still reports old `ccc` and `ccd` definitions. |
| A plain `claude` loads the playbook system prompt | the `claude` function | Moved to `pb cc` and `playbook cc` only. A plain `claude` starts with Claude Code's own prompt. |
| Default `--effort medium`, `CLAUDE_CODE_EFFORT_LEVEL` moved to `--effort`, agent variants and their environment, `models.*` overrides, `memory.source = playbook`, per-session telemetry variables, the trust entry, the cache bust, resume by name and retention | `playbook cc launch`, reached from the `claude` function | Already in `playbook cc`. A plain `claude` gets none of them. |
| `--max-budget-usd` | the `claude` function passed a call with it straight to the real `claude`, untouched | Nothing to move. The shell line added no budget handling of its own. |
| Starting the usage collector | `playbook cc` and the `session-init` hook | Unchanged. The hook starts it in every session when the settings point at it. |
| Session hooks, guards and the status line | `~/.claude/settings.json` | Unchanged. They never depended on the shell line. |
| `pb` | a shell function | The `pb` symlink that `install.sh` and the Homebrew formula already make, plus the `short-name` setup part for other installs. |
| `PLAYBOOK_BYPASS=1` | the `claude` function | Gone, since nothing wraps `claude` any more. |

## Decision

1. **No shell line.** The `aliases` and `claude-override` parts leave `playbook setup`. Until 1.0, the flags `--aliases`, `--no-aliases`, `--claude-override` and `--no-claude-override`, and the names `aliases` and `claude-override` in `--only`, stay accepted, hidden from `--help`, and do nothing. When stderr is a terminal, each prints one line on stderr that names this record, as ADR 0040 does for `init`. `launcher.claudeOverride` stays a known config key with no effect until 1.0, so a stored value reads without an error.
2. **`shell-init` prints nothing that runs.** It stays as a hidden command and prints one comment line, so an rc file that still evaluates it starts a shell with no error and no function. The `claude_override.sh` and `short_name.sh` scripts are deleted.
3. **`pb` is a symlink.** The `short-name` part passes when a `pb` on PATH already runs this binary, as after `install.sh` or Homebrew. Otherwise, when no other `pb` is on PATH and the running binary sits in the install directory that `playbook uninstall` knows (`$PLAYBOOK_BIN_DIR`, default `~/.local/bin`), it creates the relative symlink `pb -> playbook` there, the same link `install.sh` makes. `playbook uninstall --remove-binary` removes that link. When the link is in place but the install directory is not on PATH, the part passes and names the directory to add to PATH. When the running binary sits outside the install directory, it writes nothing and prints the `ln -s` command to run by hand. When another `pb` is on PATH, or a different `pb` already sits in the install directory, it writes nothing. A yes stores `launcher.shortName = true` and a no stores `false`. `--yes` keeps a stored `false`. A no removes nothing.
4. **`pb cc` loads the system prompt by default.** A new per-user config key `systemPrompt.enabled`, default `true`, decides it. It is listed in `USER_KEYS`, so it lives only in the global tier, like `update.mode`. When it is `true` and `~/.config/playbook/prompts/SYSTEM_PROMPT.md` exists, `playbook cc` passes `--system-prompt-file`, as today, and still refuses `--append-system-prompt` with exit code 2. When it is `true` and the file is missing, `playbook cc` prints the note it prints today and starts with Claude Code's prompt. When it is `false`, `playbook cc` adds no prompt flag and passes the user's own prompt flags through unchanged, append flags included. Setup writes the key: a yes to the question or `--system-prompt` writes `true` and places the file, a no or `--no-system-prompt` writes `false` and leaves an existing file in place. Running setup again with a yes turns it back on. `install.sh` drops its shell wrapper question and asks the system prompt question on its own, then passes `--system-prompt` or `--no-system-prompt` to setup. Under `--yes` it asks nothing and passes no prompt flag, so setup can keep a stored no, and `--binary-only` passes `--no-system-prompt`.
5. **`--yes` turns every part on.** The questions default to yes, and `--yes` takes every default. A run with no terminal and no `--yes` keeps the system prompt and short name parts off, like the parts that write Claude Code settings, and its stderr notice names them.

   | Part | `--yes` before | `--yes` after | No terminal, no `--yes` |
   |---|---|---|---|
   | `hooks` | on | on | off |
   | `statusline` | on | on | off |
   | `path` | on | on | on |
   | `system-prompt` | off | on | off |
   | `telemetry` | on | on | off |
   | `collector-service` | on | on | off |
   | `short-name` | off | on | off |
   | `attribution-off` | on, adds missing keys only | on, adds missing keys only | off |
   | `aliases` | off | removed | removed |
   | `claude-override` | off | removed | removed |
   | `security` | off unless `security.defaults` | unchanged | unchanged |

   `security` is never asked about and stays behind its flag and key. The consent rules of the earlier records stay. `--yes` never replaces a telemetry off switch the user set (ADR 0033), never replaces an `attribution` value the user set (ADR 0042), and never undoes a remembered no. For the system prompt, `--yes` turns it on only when nothing says the user chose otherwise. It keeps a stored `systemPrompt.enabled = false`. It also leaves the part alone on an existing install that ran setup before without the prompt: the key is not stored, the prompt file is missing and `~/.claude/settings.json` holds a `playbook hook` entry or the `playbook statusline` status line, which only an earlier setup run writes. In both cases it prints one line that names `playbook setup --only system-prompt`, and migration 0019 asks.
6. **Migration 0018 removes the old shell line, with consent.** `0018-retired-launcher-rc-line` is `Idempotent` with `Scope::Consent`. One matcher serves the check and the run. It finds, in `~/.zshrc` and `~/.bashrc`, the lines playbook wrote: the marker comment `# playbook launchers (ccc/ccd)` or `# playbook launchers (cc/ccd)`, alone or above a removed line, the exact guarded `shell-init` line, and a legacy `source` line for a launcher path under `~/.claude/shell` or `~/.config/playbook/shell`. The check reports each writable file that holds one. Its command is `playbook setup`. A full setup run copies the file to `<rc>.bak-<time>`, then removes exactly those lines and a blank line the gap leaves. A bare `eval "$(playbook shell-init)"` the user wrote by hand stays, because `shell-init` is now a harmless no-op. A file without the write bit is left out of the check, since setup cannot clear it, and doctor reports it. A symlinked rc file stays a symlink. A second run finds nothing. `playbook migrate` lists it as a Consent item with the command, and the `session-init` hook names it as for any Consent item.
7. **Migration 0007 retires.** `0007-shell-init-rc-line` rewrote legacy `source` lines to the `shell-init` line. Its check now reports nothing and its run does nothing. Migration 0018 removes the legacy lines.
8. **Migration 0019 asks before it turns the system prompt on.** Existing installs that said no to the system prompt may have said no only because they did not use the launcher. `0019-system-prompt-default-on` is `Idempotent` with `Scope::Consent`. Its check uses the existing install rule of decision 5: the key is not stored, the prompt file is missing and `~/.claude/settings.json` shows an earlier setup run. Its message is one line: `pb cc now loads the playbook system prompt by default and none is installed; to keep it off, run: playbook config set systemPrompt.enabled false`. Its command is `playbook setup --only system-prompt`. Its run writes nothing. A no in setup or the `config set` command stores `false`, which keeps the check quiet from then on. A run that asked nothing stores nothing. This uses `Consent` the way 0015 and 0016 do, for a question only setup asks, and amends ADR 0034 decision 6 to say so: the command writes playbook's own config and prompt file, and no file of the user's.
9. **Doctor.** Layer 3 checks for the lines migration 0018 removes: WARN with `playbook setup` when a writable rc file has one, WARN with a permissions hint when the file is not writable, PASS otherwise. A bare hand-written `eval "$(playbook shell-init)"` passes. A PASS adds a note when no `pb` on PATH runs this binary. Layer 4 says whether `pb cc` loads the prompt: PASS when it does, INFO when `systemPrompt.enabled` is `false`, INFO with `playbook setup --only system-prompt` when the file is missing.

## Alternatives considered

- **Keep the `claude` function as an opt-in.** Rejected by the user. It hid that `claude` was a wrapper, it needed a bypass list kept in step with Claude Code's flags, and it worked only in bash and zsh.
- **Remove `shell-init` outright.** Rejected: an rc file that still runs it would print an error on every new shell until the user consents to migration 0018.
- **Remove the shell line in the first-run step.** Rejected: ADR 0034 decision 6 puts writes to files the user owns in the `Consent` scope, and decision 13 lets the first-run step run only `Owned` migrations.
- **Turn the system prompt on for existing installs during the upgrade.** Rejected: a user who said no to it would get a new prompt in every `pb cc` session with no question asked.
- **Store the system prompt choice as the presence of the file.** Rejected: setup refreshes an existing copy on every run, so a no could not stick, and deleting the file on a no would lose an edited copy.
- **Drop the `short-name` part, since `install.sh` and Homebrew already make `pb`.** Rejected: a manual download or a binary placed by hand has no `pb`, and the user asked for `pb cc` to work.
- **Link `pb` next to whichever `playbook` is on PATH.** Rejected: on a Homebrew install that writes into the Homebrew bin folder, which `playbook uninstall` does not clean, and the formula already provides `pb` there.

## Consequences

- A user who used `claude` through the function now types `pb cc` to get the playbook prompt, effort default and agent variants. Plain `claude`, the IDE and the desktop app start with Claude Code's own system prompt, and the guides say so. The commits carry a `BREAKING CHANGE:` footer (ADR 0037).
- A launch that enters a worktree leaves the shell where it was.
- `plugins/playbook/commands/setup.md` drops the two parts, maps its old `--install-aliases` to nothing, and its Questions table shows yes as the default for the system prompt and short name.

## Amendment 2026-10-19: remove `shell-init`

The user decided that the shell line is not needed any more, since `pb cc` and `pb ccd` start sessions from the binary. This amendment replaces decision 2 and changes decisions 6 and 9. It also replaces the part of ADR 0040 decision 5 that kept the `shell-init` name. ADR 0040 rejected removing a working command before 1.0. `shell-init` already did nothing after decision 2, and the special case below keeps an old rc line from breaking shell startup until 1.0.

1. **`shell-init` is removed.** The command, its handler, its module and its tests go. It is not in `playbook --help` or the CLI reference.
2. **One special case until 1.0.** An rc file that still runs `eval "$(playbook shell-init)"` would print clap's `unrecognized subcommand` error on every new shell. So the binary checks its arguments before clap parses them. When the first argument is exactly `shell-init`, whatever arguments follow, it prints one line on stderr, `playbook shell-init was removed (ADR 0044). Delete the line that runs it from your shell rc file, or run: playbook setup`. It writes nothing to stdout, so the `eval` runs an empty string, writes nothing under the home, and exits 0. The check lives in `crates/cli/src/fastpath.rs`. It stays at least until the release line after the one that removes the command, and it is removed in the 1.0.0 release, marked as a breaking change.
3. **Migration 0018 also removes the hand-written line.** Decision 6 kept a bare `eval "$(playbook shell-init)"` because it was a no-op. It now prints the stderr line on every new shell, so migration 0018 also finds and removes it, with and without the old `--shell zsh` or `--shell bash` flag. Only a line that starts at column 0 matches: an indented line may be the only command of the user's own `if` block, and bash rejects an empty `then` body. The migration id and its place in the registry do not change.
4. **Doctor warns on the hand-written line.** Decision 9 let a bare hand-written line pass. Layer 3 now warns on the lines that decision 3 of this amendment adds.
5. **Config keys.** `launcher.claudeOverride` stays a known key with no effect until 1.0, as decision 1 says, so a stored value still reads. `launcher.shortName` stays, since the `short-name` part reads and writes it.

Consequences: a script that read the output of `shell-init` gets an empty string. After 1.0, an rc file that still runs the line gets clap's error, so doctor and migration 0018 keep finding the line. The commit carries a `BREAKING CHANGE:` footer.
