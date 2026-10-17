# ADR-0044: Drop the Shell Launcher and Turn Every Setup Part On With `--yes`

- **Status:** Accepted
- **Date created:** 2026-10-18
- **Date modified:** 2026-10-18
- **Amends:** [ADR 0007](0007-rust-binary-for-hooks-and-launcher.md), which kept a small shell function per shell family for the `cd` into a worktree. Decision 6 of [ADR 0034](0034-self-healing-upgrades.md), for migration `0007-shell-init-rc-line`. The `--yes` defaults in [ADR 0033](0033-telemetry-on-by-default.md), [ADR 0042](0042-attribution-off-in-setup.md) and [ADR 0043](0043-collector-service-in-setup.md), which this record extends to every part.

## Context

`playbook setup` has three optional parts that act through one line in the shell rc file:

- `aliases` adds `command -v playbook >/dev/null 2>&1 && eval "$(playbook shell-init)"` to `~/.zshrc` or `~/.bashrc`, under the marker comment `# playbook launchers (ccc/ccd)`. `playbook shell-init` prints a `playbook` shell function. After `playbook cc` or `playbook ccd` enters a worktree, the function changes the shell's directory to it, through the file named by `PLAYBOOK_CC_CD_FILE`. It also defines `ccc` and `ccd` stubs that name their replacements.
- `claude-override` sets `launcher.claudeOverride`. `shell-init` then also defines a `claude` function (`crates/cli/src/init/claude_override.sh`). A plain interactive `claude` runs `playbook cc launch -- <args>`. Piped use, `-p`, subcommands, `--max-budget-usd`, the user's own prompt or settings flags and `PLAYBOOK_BYPASS=1` go straight to the real binary.
- `short-name` sets `launcher.shortName`. `shell-init` then also defines `pb` as `playbook` when nothing else uses the name.

All three are off by default and work only once the `aliases` line is in the rc file. The system prompt part is off by default too. With `--yes` a new install gets no system prompt, so `playbook cc` starts with Claude Code's own prompt.

The user decided: the shell line goes. `pb cc` must work and load the playbook system prompt by default. `--yes` turns on every feature. A user can still turn the system prompt off in setup, and then `pb cc` starts plain Claude Code.

## What only worked through the shell line

| Behavior | Where it lived | After this change |
|---|---|---|
| `cd` into the worktree after `playbook cc worktree <branch>` or `playbook cc new` | the `playbook` shell function and `PLAYBOOK_CC_CD_FILE` | Lost. A child process cannot change its parent's directory. `playbook cc` prints the worktree path and the `cd` command after the session ends. |
| `ccc` and `ccd` stubs that name `playbook cc` and `playbook ccd` | `shell-init` | Lost. `playbook doctor` still reports old `ccc` and `ccd` definitions. |
| A plain `claude` loads the playbook system prompt | the `claude` function | Moved to `pb cc` and `playbook cc` only. A plain `claude` starts with Claude Code's own prompt. |
| Default `--effort medium`, `CLAUDE_CODE_EFFORT_LEVEL` moved to `--effort`, agent variants and their environment, `models.*` overrides, `memory.source = playbook`, per-session telemetry variables, the trust entry, the cache bust, resume by name and retention | `playbook cc launch`, reached from the `claude` function | Already in `playbook cc`. A plain `claude` gets none of them. |
| `--max-budget-usd` | the `claude` function passed a call with it straight to the real `claude`, untouched | Nothing to move. The shell line added no budget handling of its own. |
| Starting the usage collector | `playbook cc` and the `session-init` hook | Unchanged. The hook starts it in every session when the settings point at it. |
| Session hooks, guards and the status line | `~/.claude/settings.json` | Unchanged. They never depended on the shell line. |
| `pb` | a shell function | A `pb` symlink to `playbook` next to the binary, as the install script already makes. |
| `PLAYBOOK_BYPASS=1` | the `claude` function | Gone, since nothing wraps `claude` any more. |

## Decision

1. **No shell line.** The `aliases` and `claude-override` parts leave `playbook setup`. Their flags `--aliases`, `--no-aliases`, `--claude-override` and `--no-claude-override` stay accepted for the 0.x line, hidden from `--help`, and do nothing. When stderr is a terminal, each prints one line on stderr that names this record, as ADR 0040 does for `init`. `--only` no longer takes `aliases` or `claude-override`.
2. **`shell-init` prints nothing that runs.** It stays as a hidden command and prints one comment line, so an rc file that still evaluates it starts a shell with no error and no function. The `claude_override.sh` and `short_name.sh` scripts are deleted.
3. **`pb` is a symlink.** The `short-name` part creates `pb -> playbook` in the directory of the `playbook` that is on PATH, the same rule `install.sh` uses. It writes nothing when `pb` already exists there or another `pb` is on PATH. `playbook uninstall --remove-binary` already removes the link. `launcher.shortName` and `launcher.claudeOverride` stay known config keys with no effect, so a stored value reads without an error, and migration 0018 can read the old value.
4. **`pb cc` loads the system prompt by default.** A new config key `systemPrompt.enabled`, default `true`, decides it. When it is `true` and `~/.config/playbook/prompts/SYSTEM_PROMPT.md` exists, `playbook cc` passes `--system-prompt-file`, as today. When it is `false`, `playbook cc` adds no prompt flag and Claude Code uses its own prompt. Setup writes the key: a yes to the question or `--system-prompt` writes `true` and places the file, a no or `--no-system-prompt` writes `false` and leaves an existing file in place. Running setup again with a yes turns it back on.
5. **`--yes` turns every part on.** The questions default to yes, and `--yes` takes every default:

   | Part | `--yes` before | `--yes` after |
   |---|---|---|
   | `hooks` | on | on |
   | `statusline` | on | on |
   | `path` | on | on |
   | `system-prompt` | off | on |
   | `telemetry` | on | on |
   | `collector-service` | on | on |
   | `short-name` | off | on |
   | `attribution-off` | on, adds missing keys only | on, adds missing keys only |
   | `aliases` | off | removed |
   | `claude-override` | off | removed |
   | `security` | off unless `security.defaults` | unchanged: it is never asked about and stays behind its flag and key |

   The consent rules of the earlier records stay. `--yes` never replaces a telemetry off switch the user set (ADR 0033), never replaces an `attribution` value the user set (ADR 0042), and never undoes a remembered no. For the system prompt, `--yes` keeps a stored `systemPrompt.enabled = false` and prints one line that names `playbook setup --only system-prompt`.
6. **Migration 0018 removes the old shell line, with consent.** `0018-retired-launcher-rc-line` is `Idempotent` with `Scope::Consent`. Its check reports each of `~/.zshrc` and `~/.bashrc` that holds a line playbook wrote: the marker comment `# playbook launchers (ccc/ccd)` or `# playbook launchers (cc/ccd)`, the exact guarded `shell-init` line, or a legacy `source` line for a launcher path under `~/.claude/shell` or `~/.config/playbook/shell`. Its command is `playbook setup`. In a full setup run it removes exactly those lines and a blank line the gap leaves, after it copies the file to `<rc>.bak-<time>`. A bare `eval "$(playbook shell-init)"` the user wrote by hand stays, because `shell-init` is now a harmless no-op. A file without the write bit is left alone and reported. A symlinked rc file stays a symlink. A second run finds nothing. `playbook migrate` lists it as a Consent item with the command, and the `session-init` hook names it as for any Consent item.
7. **Migration 0007 retires.** `0007-shell-init-rc-line` rewrote legacy `source` lines to the `shell-init` line. Its check now reports nothing and its run does nothing. Migration 0018 removes the legacy lines.
8. **Migration 0019 asks before it turns the system prompt on.** Existing installs that said no to the system prompt may have said no only because they did not use the launcher. `0019-system-prompt-default-on` is `Idempotent` with `Scope::Consent`. Its check reports when the playbook folder exists, `systemPrompt.enabled` is not stored in any tier and the prompt file is missing. The message is one line: `pb cc now loads the playbook system prompt by default, and this machine has none installed`. Its command is `playbook setup --only system-prompt`. Its run writes nothing. A no in setup stores `systemPrompt.enabled = false`, which keeps the check quiet from then on. A run that asked nothing stores nothing. A new install that runs setup with `--yes` stores `true`, so the item never shows there.
9. **Doctor.** Layer 3 now checks for the retired shell line: WARN with `playbook setup` when an rc file still has it, PASS otherwise, with a note when `pb` is not linked. Layer 4 says whether `pb cc` loads the prompt: PASS when it does, INFO when `systemPrompt.enabled` is `false`, INFO with `playbook setup --only system-prompt` when the file is missing.

## Alternatives considered

- **Keep the `claude` function as an opt-in.** Rejected by the user. It hid that `claude` was a wrapper, it needed a bypass list kept in step with Claude Code's flags, and it worked only in bash and zsh.
- **Remove `shell-init` outright.** Rejected: an rc file that still runs it would print an error on every new shell until the user consents to migration 0018.
- **Remove the shell line in the first-run step.** Rejected: ADR 0034 decision 8 keeps writes to files the user owns behind a run of `playbook setup`.
- **Turn the system prompt on for existing installs during the upgrade.** Rejected: a user who said no to it would get a new prompt in every `pb cc` session with no question asked.
- **Store the system prompt choice as the presence of the file.** Rejected: setup refreshes an existing copy on every run, so a no could not stick, and deleting the file on a no would lose an edited copy.

## Consequences

- A user who used `claude` through the function now types `pb cc` to get the playbook prompt, effort default and agent variants. The commit carries a `BREAKING CHANGE:` footer (ADR 0037).
- `playbook cc worktree <branch>` leaves the shell where it was. The session itself runs in the worktree.
- A new install with `--yes` gets the system prompt and `pb`.
- `launcher.claudeOverride` and `launcher.shortName` stay in the key table until 1.0 and do nothing.
- Plain `claude`, the IDE and the desktop app start with Claude Code's own system prompt. The guides say so.
