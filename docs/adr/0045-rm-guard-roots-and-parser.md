# ADR-0045: Read rm Commands as Real Words and Build In the Safe Roots

- **Status:** Accepted, amended 2026-10-19 (playbook's footprint hard-coded; then cut down to a short fixed rule, which supersedes the roots and the parser below)
- **Date created:** 2026-10-18
- **Date modified:** 2026-10-19
- **Amends:** [ADR 0002](0002-plugin-based-install-safety-hooks.md), which ships the `rm-workspace-guard` always on. Builds on the self-healing goal of [ADR 0034](0034-self-healing-upgrades.md): the model may clean up playbook's leftovers while it repairs an install.

## Context

The rm guard and the policy guard kept blocking commands that delete nothing, including the model's own work. The rules behind each false positive:

- **rm guard, word splitting.** The old tokenizer split on `|` and `;` even inside quotes, so `grep "a | rm -rf x"` put `rm` in command position.
- **rm guard, redirects.** `2>/dev/null` after an `rm` was judged as a target and denied because it holds `>`.
- **rm guard, heredocs.** A heredoc line that started with `rm` counted as a command, even when the heredoc fed `cat` or `git commit -F -`.
- **rm guard, variables.** `$HOME/x` could not be resolved, so it was denied.
- **policy guard, config store.** Any word that named the config store, in a command that had a writer program (`sqlite3`, `jq`, `sed`, `python`, a shell) or any redirect, matched the rule for changing the rm guard's allowed roots. `sqlite3 config.db "select ..."`, `jq . config.json`, `cat config.json 2>/dev/null` and `ls ~/.config/playbook/data > /tmp/out` were all denied.
- **auto mode.** `auto_cost` treated any mention of the config database as a guarded call.

The allowed roots were also too narrow. `/var/folders`, `~/Library/Caches`, Linux `/var/tmp` and `/run/user/<uid>`, and playbook's own install files were all blocked, and the config key refused the `~/x/**` form people type first, with an error that did not say what a root covers.

## Decision

1. **Parse real command words.** `rm_workspace_guard::parse` reads the command with the shared shell lexer (`common::shell`). Only a word in command position is a deletion: after `&&`, `||`, `;`, `|`, inside `$(...)` and backticks, behind wrappers and runners (`command`, `builtin`, `sudo`, `doas`, `env`, `nohup`, `xargs` and others), `\rm`, `find -delete`, `find -exec rm`, and scripts given to a shell through `-c`, `eval`, a here-string or a heredoc. Quoted text, grep patterns, commit messages, `echo`, file paths and heredocs read by other programs are data. Only `$HOME` and `${HOME}` expand, and only when the command does not change `HOME`.
2. **Never block a read of the config store.** The policy guard denies only a write: a redirect into the store, `sed -i` or a `w` command, a writer program or interpreter handed the store, or `sqlite3` with anything other than a read statement. Auto mode uses the same check. Each denial starts with the rule that matched.
3. **Build in the temp and cache folders per OS**, contents only, from one function (`rm_workspace_guard::roots`) that the guard and the tests share. A test injects the home folder and the environment, so every OS is covered on any host.
4. **Build in playbook's scratch folders and install leftovers** so the model can clean them up while it repairs an install, for example old binary backups after an update or the collector's service file before setup writes it again. The heal in ADR 0034 runs inside the binary and never goes through this hook.
5. **Protect playbook's data** whatever root covers it, a configured root included.
6. **Name the rule in each denial**, show where a path resolved to, and give the exact `config set` command with the current roots plus the new one.
7. **Accept a trailing `/**` or `/*` on a configured root** and drop it before storing, since a root covers everything under it. The stored list holds plain paths, so an older binary still reads it. The error for a refused root says so and gives a working example. `config set` prints the stored value on success. There is no append form, because `playbook config` has none, so the docs show the full `set` command.

### Built-in roots per OS

All are contents only: the folder itself and every parent stay blocked, and a glob directly under the folder needs a named prefix of at least three characters.

| OS | Root | Why deleting inside it is safe |
| :- | :- | :- |
| All | `std::env::temp_dir()` | The OS temp folder. Programs expect its files to vanish. It also holds the session scratchpad. |
| macOS | `/tmp`, `/private/tmp` | System temp, cleared at boot. |
| macOS | `/private/var/folders/*/*/T` (and `/var/folders/...`) | The per-user temp folder, matched by shape because its name is a hash. |
| macOS | `$TMPDIR` and the path it resolves to | The per-user temp folder as the hook sees it. Only an absolute value counts. |
| macOS | `~/Library/Caches` | App caches, rebuilt on demand. |
| macOS, Linux | `~/.cache` | Caches by definition. Covers `~/.cache/playbook`. |
| Linux | `/tmp`, `/var/tmp`, `$TMPDIR` | System and per-user temp. `/private/tmp` stays listed on every OS, as the old guard had it. |
| Linux | `$XDG_CACHE_HOME` | The user cache, only when it is inside home, so a value of `/` cannot widen the allowance. |
| Linux | `/run/user/<uid>` | Per-session runtime files of this user, cleared at logout. The request named this root. Deleting a live socket here, such as an agent socket, can break a running service until the next login, so only a named path goes and a bare glob stays blocked. |
| Windows | `%TEMP%`, `%TMP%`, `%LOCALAPPDATA%\Temp` | The user temp folders. |

A folder from the environment, `std::env::temp_dir()` included, counts only when it is absolute, at least two folders deep, free of `..`, and neither home nor a folder above it. `TMPDIR=/` or `TMPDIR=$HOME` therefore adds nothing.

### Playbook roots for self-healing

| Root | Kind | Why deleting it is safe |
| :- | :- | :- |
| `~/.config/playbook/tmp` | contents | Document scratch, swept by `doc` itself. |
| `~/.config/playbook/staging` | contents | Reserved for staging files. Nothing durable lives there. |
| `~/.config/playbook/embedded` | contents | Files unpacked from the binary, written again on demand. |
| `~/.config/playbook/cc-state` | contents | Config drift markers, written again on the next launch. |
| `~/.config/playbook/**/tmp` | contents | Any deeper `tmp` folder is scratch by name. The protected list is checked first, so a `tmp` inside the memory store does not count. |
| `playbook.<version>.bak` in the install folder | name prefix | Backups that `update::swap` keeps and prunes. |
| `.playbook.*` in the install folder | name prefix | Temp and staging files from `install.sh` and self-update. |
| `~/Library/LaunchAgents/io.github.pragmatic-engineer.playbook.collector.plist` | exact name | The collector's launchd file, built from `playbook_telemetry::service`. Setup writes it again. |
| `~/.config/systemd/user/playbook-collector.service` | exact name | The collector's systemd unit, from the same code. |

The install folder is `$PLAYBOOK_BIN_DIR` or `~/.local/bin`, from `uninstall::install_dir`. A Homebrew install folder (`update::is_homebrew`) gets no allowance, so Homebrew stays in charge of its own files. The plugin cache `~/.claude/plugins/cache/pragmatic-engineer/playbook`, its orphaned version folders (migration 0014) and the marketplace clone sit under `~/.claude`, which was already allowed whole.

### Additions from the request

Each was added only where deleting is safe:

- `~/.cache/playbook/**`: covered by `~/.cache`.
- `~/.config/playbook/staging/**`: added as a playbook scratch root.
- `~/.config/playbook/**/tmp/**`: added, outside the protected paths.
- `/tmp/pb-*`: allowed by the named prefix rule under `/tmp`.
- `.worktrees/cargo-*`: covered by the `.worktrees` root.

### Protected paths

Blocked whatever root covers them: `~/.config/playbook/memory`, `data` (`config.db`, `memory.db`, the usage database), `repos` (plans, handoffs, per-repo config, live worktrees), `orgs` (legacy org config), `runtime` (the spend ledger the auto mode cap reads, the auto-learn queue and old handoffs), `effort`, `prompts`, `playbook.db` with its `-wal` and `-shm` files, `config.json`, any folder that holds one of them (such as `~/.config/playbook` itself), and any `.lock` folder under `~/.config/playbook`. The check compares the path as the disk stores it, so a different letter case on a case-insensitive disk (`MEMORY`, `runtime/HANDOFF`) is caught.

The playbook binary and its `pb` and `pbtop` links are not allowed either. Every hook runs the binary, so deleting it would turn every guard off, the rm guard and the auto mode cap included.

### Amendment 2026-10-19: playbook's footprint is hard-coded

The user ordered playbook's whole footprint hard-coded in the binary as allowed roots, per OS, with no config, no setup step and no migration. `roots::footprint` is the one function the guard reads. It returns the config folder (`~/.config/playbook`, honouring `XDG_CONFIG_HOME`; `%APPDATA%\playbook` on Windows), the cache, data and state folders (`~/.cache/playbook`, `~/.local/share/playbook`, `~/.local/state/playbook`, the `XDG_*_HOME` values, `~/Library/Caches/playbook`, `~/Library/Application Support/playbook` and `~/Library/Logs/playbook` on macOS, `%LOCALAPPDATA%\playbook` on Windows), and the plugin cache and marketplace folders under `~/.claude/plugins`. Each folder may go whole, the folder itself included. Only a folder named for playbook counts, so a bare `~/.config` or `~/.cache` stays blocked. The install files now include the binary and its `pb` and `pbtop` links, and a linked worktree root (its `.git` is a file pointing into the main repo's `worktrees` folder) in a folder playbook makes worktrees in (`.worktrees`, `WORKTREE_BASE_DIR`, `.claude/worktrees`) may go. A main checkout and any `.git` there stay blocked, a worktree folder that is `/`, home or above home is dropped, and a footprint folder that links to a folder not named for playbook is dropped.

The protected paths shrink to what cannot be made again: the memory store, `data/config.db` with its `-wal` and `-shm` files, `repos` (plans, handoffs), `orgs` and `runtime` (the spend ledger). The lock rule is gone. The denial names each protected item and the `playbook` command to use instead, or says to ask the user. This supersedes the protected list above and the rejected alternative to allow the binary: `playbook update` and `install.sh` still replace it, and the user chose to let the model remove a broken install.

The policy guard follows the same line. It leaves `rm` of the store to the rm guard, so the two guards no longer disagree about a deletion, and it judges `cp`, `install`, `rsync` and `dd` by where they may write (any operand after the first, a `-t` folder, `of=`), so a backup copy out of the store is a read. A link into or out of the store, and `mv` either way, stay denied. It now also denies a shell write into the memory store (a redirect, `tee`, `cp` into it, `mv` into or out of it, `sed -i`), since a fact goes in with the Write tool, which shows the fact format. `mkdir` and `touch` there stay allowed. The rm guard corpus gains `"hook": "all"` cases, judged by both guards as the user sees them.

## Alternatives considered

- **Allow the binary and its links** for self-healing, as first asked. Rejected: a deleted binary turns off every hook, and `playbook update` and `install.sh` replace it without an `rm`.
- **Allow `~/.config/playbook/runtime` as scratch.** Rejected: it holds the live session's spend ledger, so deleting it would reset the auto mode cap.
- **Expand `$TMPDIR` in commands.** Rejected: the Bash tool's shell can hold a different value than the hook, so the guard could allow a path the shell sends elsewhere.
- **Keep the temp folders in config only.** Rejected: every user would have to add the same OS folders by hand, and a wrong entry could cover too much.

## Consequences

- The model's ordinary commands stop tripping either guard, and reads of the config store always pass.
- The model can remove playbook's binary backups, staging files, scratch folders and service file without asking the user.
- A denial tells the user exactly what to run. The fix command is offered only for a plain folder inside home, never for a system folder or a folder whose first folder under home starts with a dot, such as `~/.ssh`.
- Not verified: Windows. The crate builds there, but the guard's path logic splits on `/` and has not run on Windows.
- Known gap: the hook is wired with `if: Bash(rm:*)`, so a `find -delete` or `xargs` command with no `rm` word never reaches it.
- `$TMPDIR` written in a command is not expanded, because the Bash tool's shell can hold a different value than the hook. Expanding it could allow a path the shell sends elsewhere.

## Amendment 2026-10-19: the guard is cut down to a short fixed rule

### Why

The roots and the parser kept denying harmless commands, and each denial became a support request. The rule set (safe roots, a deny set, `cd` and variable following, `find` and `xargs` handling) was about 4,600 lines with tests and could not be made right for every shell command. The cost in false positives and support was larger than the protection it gave. The maintainer chose to keep the one case nobody argues about.

### What

The `rm-workspace-guard` hook keeps its name, because the wiring written to users' settings.json names it. It now denies only an `rm`, `rmdir`, `unlink` or `shred` whose target is `/`, `/*`, `~`, `~/`, `~/*`, `$HOME`, `${HOME}`, `$HOME/`, `$HOME/*` or the literal path of the user's home folder (also `/**`, `~/.*` and `${HOME:?}`), with any flags, after a wrapper (`sudo`, `doas`, `command`, `env`, `exec`, `nohup`, `time`, `nice`, `ionice`, `timeout` or `xargs`), and in a chain split on `;`, `&`, `|`, `(`, `)`, backticks and line breaks. It also reads the script of `sh -c` and `bash -c`. Every other path is allowed. The guard does not follow `cd`, variables, `find -delete`, the input of `xargs`, here-docs or script files, and anything it cannot read passes. The denial is one fixed sentence.

Removed: `rm_workspace_guard/{parse,roots,message,expand,interp}.rs`, their corpus tests, the `hooks.rmGuard.allowedRoots` key (a stored value is kept, ignored and skipped by `config import` for one release; `config set` refuses it) and `playbook hooks explain-rm` (it prints a note and exits 2). The policy guard still refuses a model-run change to `hooks.rmGuard.*`. A one-time notice, the Auto migration `rm-guard-simplified`, tells an install that had the guard wired what changed. `playbook disk clean` keeps its own list of protected playbook data and does not depend on the guard.

### Consequence

The guard no longer protects a project folder, `~/.ssh`, `~/.claude`, the memory store, `config.db`, plans or handoffs from an `rm`. With the seed `defaultMode` set to `auto`, the only remaining deletion safety is Claude Code's own permission handling and the user's deny rules in settings.json. A user who wants a path protected adds a deny or ask rule for it there. The earlier decisions on roots, the footprint, the deny set and the parser in this ADR no longer apply, and the known gaps and accepted risks above are moot.
