# ADR-0045: Read rm Commands as Real Words and Build In the Safe Roots

- **Status:** Accepted
- **Date created:** 2026-10-18
- **Date modified:** 2026-10-18
- **Amends:** [ADR 0002](0002-plugin-based-install-safety-hooks.md), which ships the `rm-workspace-guard` always on. Builds on the self-healing goal of [ADR 0034](0034-self-healing-upgrades.md): playbook must be able to clean up and repair its own install.

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
4. **Build in playbook's scratch folders and install files** so playbook can heal its own install.
5. **Protect playbook's data** whatever root covers it, a configured root included.
6. **Name the rule in each denial**, show where a path resolved to, and give the exact `config set` command with the current roots plus the new one.
7. **Accept a trailing `/**` or `/*` on a configured root** and drop it, since a root covers everything under it. The error for a refused root says so and gives a working example. `config set` prints the stored value on success. There is no append form, because `playbook config` has none, so the docs show the full `set` command.

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
| Linux | `/tmp`, `/var/tmp`, `$TMPDIR` | System and per-user temp. |
| Linux | `$XDG_CACHE_HOME` | The user cache, only when it is inside home, so a value of `/` cannot widen the allowance. |
| Linux | `/run/user/<uid>` | Per-session runtime files of this user, cleared at logout. A bare glob stays blocked, so a live socket goes only when named. |
| Windows | `%TEMP%`, `%TMP%`, `%LOCALAPPDATA%\Temp` | The user temp folders. |

### Playbook roots for self-healing

| Root | Kind | Why deleting it is safe |
| :- | :- | :- |
| `~/.config/playbook/tmp` | contents | Document scratch, swept by `doc` itself. |
| `~/.config/playbook/staging` | contents | Reserved for staging files. Nothing durable lives there. |
| `~/.config/playbook/runtime` | contents | Per-session state, made again by the next session. `runtime/handoff` is protected. |
| `~/.config/playbook/embedded` | contents | Files unpacked from the binary, written again on demand. |
| `~/.config/playbook/cc-state` | contents | Config drift markers, written again on the next launch. |
| `~/.config/playbook/**/tmp` | contents | Any deeper `tmp` folder is scratch by name. The protected list is checked first, so a `tmp` inside the memory store does not count. |
| `playbook`, `pb`, `pbtop` in the install folder | exact names | The binary and its links. `playbook update` and `install.sh` replace them. |
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

Blocked whatever root covers them: `~/.config/playbook/memory`, `data` (`config.db`, `memory.db`, the usage database), `repos` (plans, handoffs, per-repo config, live worktrees), `runtime/handoff`, `effort`, `prompts`, `playbook.db` with its `-wal` and `-shm` files, `config.json`, any folder that holds one of them (such as `~/.config/playbook` itself), and any `.lock` folder under `~/.config/playbook`.

## Consequences

- The model's ordinary commands stop tripping either guard, and reads of the config store always pass.
- Playbook can remove its own binary backups, staging files, scratch and service file without asking the user.
- A denial tells the user exactly what to run. The fix command is offered only for a plain folder inside home, never for a system folder or a dot folder such as `~/.ssh`.
- Not verified: Windows. The crate builds there, but the guard's path logic splits on `/` and has not run on Windows.
- Known gap: the hook is wired with `if: Bash(rm:*)`, so a `find -delete` or `xargs` command with no `rm` word never reaches it.
- `$TMPDIR` written in a command is not expanded, because the Bash tool's shell can hold a different value than the hook. Expanding it could allow a path the shell sends elsewhere.
