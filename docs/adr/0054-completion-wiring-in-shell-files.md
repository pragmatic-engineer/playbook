# ADR-0054: Setup Wires Shell Completion Into Shell Files

- **Status:** Proposed
- **Date created:** 2026-10-23
- **Date modified:** 2026-10-24
- **Amends:** [ADR 0044](0044-drop-the-shell-launcher.md), decision 1 (no shell line), only for the completion line below. It adds one row to the decision 5 table.

## Context

ADR 0044 decision 1 removed the line that `playbook setup` wrote into the shell rc file. Doctor `layer3` still warns on the old lines. The stance since then is that setup writes nothing into `~/.zshrc` or `~/.bashrc`.

Tab completion for `playbook` and `pb` needs wiring in the shell. Bash and fish read completion files from their own autoload directories, so a stub file there is enough. Zsh has no such directory. It reads `_playbook` only from a directory on `fpath`, and the default `fpath` has no user directory. Zsh needs one line in `~/.zshrc`.

For zsh, setup writes a loader file and one marked line in `~/.zshrc` that sources it. This design was chosen for these reasons:

- The loader runs `playbook completions zsh` lazily on the first Tab press. Shell startup pays nothing.
- It works above or below the user's own `compinit`.
- It is silent when `playbook` is missing or fails.
- The `pb` guard sits in the loader. `pb` is registered only when it is the same file as `playbook`.
- The logic lives in a file that setup and update rewrite. A logic change needs no rc edit, no new marker and no migration.
- Uninstall removes the file, so a leftover rc line is harmless.

The design was prototyped in a real zsh on 2026-10-24. The lazy stub was replaced by the real function on the first call. Both names were registered. There was no output without the binary. A real Tab press in a pty is not tested yet. The zsh pty test in S3b covers it.

## Decision

1. **Setup writes completion stubs for bash and fish.** A stub is a small file that asks `playbook completions` for the real script when the shell loads it. Locations:
   - bash: `~/.local/share/bash-completion/completions/playbook` and `pb`
   - fish: `~/.config/fish/completions/playbook.fish` and `pb.fish`
2. **Zsh gets a loader file.** Setup writes `~/.local/share/playbook/completions.zsh`. There is no `fpath` stub. The file starts with a one-line comment saying that `playbook setup` writes it and that update rewrites it. The rest is exact text, not a template:

   ```zsh
   (( $+commands[playbook] )) || return 0
   (( $+functions[compdef] )) || { autoload -Uz compinit && compinit -C; }
   _playbook() { unfunction _playbook; source <(playbook completions zsh 2>/dev/null); _playbook "$@"; }
   compdef _playbook playbook
   [[ ${commands[pb]} -ef ${commands[playbook]} ]] && compdef _playbook pb
   ```

3. **Zsh gets one marked line in `~/.zshrc`.** The line is exact text. The marker is the trailing comment `# playbook completions`.

   ```zsh
   [ -r "$HOME/.local/share/playbook/completions.zsh" ] && . "$HOME/.local/share/playbook/completions.zsh"  # playbook completions
   ```

4. **The write is safe.** These rules apply to the rc line and to the loader file.
   - It is atomic: a temp file in the same directory, then a rename.
   - A missing `~/.zshrc` is created with mode 0644. An existing file keeps its mode.
   - A symlinked file is followed and the target is written. A dangling symlink is skipped.
   - A file owned by another user is refused. This is the `sudo playbook setup` case.
5. **Rerun detection.**
   - For the rc line, the exact line means nothing to do. A line with the marker but different text is reported as `kept (edited)` and is left as it is. Uninstall strips every exact line, so two exact lines are both removed.
   - For the loader file, setup rewrites it whenever its text differs, because the file is ours. A loader file without the header comment is reported as `kept (not ours)` and is not overwritten.
6. **Backups.** Setup only appends to `~/.zshrc`, so it takes no backup. The uninstall strip removes the line and takes `.zshrc.bak-<stamp>` first, as migration 0018 does. Uninstall also removes the loader file.
7. **This part is a new setup row.** It reverses the ADR 0044 stance only for this line, and it comes with a new marker, a skip rule (the part is off with no terminal and no `--yes`), an uninstall step and a doctor row.

   | Part | `--yes` before | `--yes` after | No terminal, no `--yes` |
   |---|---|---|---|
   | `completions` | not a part yet | on | off |

   It is on by default in a terminal. It adds no config key and records no decline. Doctor reports a missing stub or loader as INFO, not as a failure.
8. **Records of choices made:**
   - `playbook completions` is a hot path. The bash and fish stubs run it on every new shell, so it must stay fast and print only the script. The zsh loader runs it once, on the first Tab.
   - `pb` is decided at load time. Each stub and the zsh loader check then whether `pb` runs this binary, so setup stores nothing about it.
   - There is no config key and no migration. Existing installs get the stubs and the loader on the next `playbook setup`.

## Backups

The rc backups made by the uninstall strip are kept on purpose. `playbook uninstall` does not remove them: `remove_backups` (`crates/cli/src/uninstall/mod.rs:312-330`) only touches `*.bak` files in the bin directory. The uninstall docs will say so and name the file for the user to delete by hand. The work unit that adds the uninstall step writes that note.

## Security note

A stub or the loader runs `playbook` from PATH, as the user. If a `playbook` earlier on PATH is not the one the user installed, that one runs. This is no worse than the user typing `playbook` at the prompt. They print nothing and set no error when `playbook` fails, so a broken install does not break shell startup. The loader file is sourced by the shell, so it is owned by the user and written with the same safe-write rules as the rc file.

## Alternatives considered

- **Print the line and let the user paste it.** Safe, but most users would not do it, and completion would stay unused.
- **Write the completion script itself into the autoload directory.** It goes stale after an upgrade. A stub gets the script of the installed version.
- **Add a config key to turn it off.** There is nothing to remember: a user who does not want it answers no, and a removed stub stays removed until the next setup run asks again.
- **A 3-line block in `~/.zshrc` with an `fpath` stub.** Rejected. A change to the block text needs a migration, and it needs more code.
- **One self-contained line in `~/.zshrc`.** Rejected. The logic would be frozen in the rc file, and the line would be long.
- **Detect zsh frameworks and use their completion folders.** Rejected. It covers a minority of users, and each framework is an unverified assumption.
- **Ship a plugin.** Rejected. It is a separate channel and does not help plain zsh.

## Consequences

- `retired_rc_entries` (migration 0018 and doctor `layer3`) must ignore any rc line that ends with the marker `# playbook completions`. That line must not match `shim::BLOCK_COMMENTS`, `SOURCE_LINE`, `BARE_EVALS` or a legacy `source` path, so doctor `layer3` stays PASS. A test pins this rule.
- Setup and uninstall each get a completions step. Doctor gets a completions row.
- Because setup edits `~/.zshrc` again, the docs for setup say what is written and where.
- A change to the loader logic needs no rc edit. Setup and update rewrite the file. Only a change to the rc line itself would need a migration.
- S4a (the stub writer, merged), S4b (doctor and uninstall, merged) and S4c (#1335, draft) must be reworked in a new Segment S4d after this ADR is accepted. The loader and the rc line replace the zsh stub and block in all of them.
