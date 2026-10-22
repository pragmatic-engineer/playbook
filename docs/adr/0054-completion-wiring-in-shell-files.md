# ADR-0054: Setup Wires Shell Completion Into Shell Files

- **Status:** Proposed
- **Date created:** 2026-10-23
- **Date modified:** 2026-10-23
- **Amends:** [ADR 0044](0044-drop-the-shell-launcher.md), decision 1 (no shell line), only for the completion block below. It adds one row to the decision 5 table.

## Context

ADR 0044 decision 1 removed the line that `playbook setup` wrote into the shell rc file. Doctor `layer3` still warns on the old lines. The stance since then is that setup writes nothing into `~/.zshrc` or `~/.bashrc`.

Tab completion for `playbook` and `pb` needs wiring in the shell. Bash and fish read completion files from their own autoload directories, so a stub file there is enough. Zsh reads `_playbook` only from a directory on `fpath`, and the default `fpath` has no user directory. Zsh needs one block in `~/.zshrc`.

## Decision

1. **Setup writes completion stubs.** A stub is a small file that asks `playbook completions` for the real script when the shell loads it. Locations:
   - bash: `~/.local/share/bash-completion/completions/playbook` and `pb`
   - fish: `~/.config/fish/completions/playbook.fish` and `pb.fish`
   - zsh: `~/.local/share/zsh/site-functions/_playbook`
2. **Zsh gets one marked block in `~/.zshrc`.** Three lines: the marker line `# playbook completions`, an `fpath` line, and one `compdef` or `compinit` line. The block is exact text, not a template.
3. **The write is safe.**
   - It is atomic: a temp file in the same directory, then a rename.
   - A missing `~/.zshrc` is created with mode 0644. An existing file keeps its mode.
   - A symlinked rc file is followed and the target is written. A dangling symlink is skipped.
   - A file owned by another user is refused. This is the `sudo playbook setup` case.
4. **Rerun detection uses the marker.** The marker with the exact lines after it means nothing to do. The marker with different lines after it is reported as `kept (edited)` and nothing is written. Setup never rewrites an edited block.
5. **The change only appends, so setup takes no backup.** The uninstall strip removes the block and takes `.zshrc.bak-<stamp>` first, as migration 0018 does.
6. **This part is a new setup row.** It reverses the ADR 0044 stance only for this block, and it comes with a new marker, a skip rule (the part is off with no terminal and no `--yes`), an uninstall step and a doctor row.

   | Part | `--yes` before | `--yes` after | No terminal, no `--yes` |
   |---|---|---|---|
   | `completions` | not a part yet | on | off |

   It is on by default in a terminal. It adds no config key and records no decline. Doctor reports a missing stub as INFO, never as a failure.
7. **Records of choices made:**
   - `playbook completions` is a hot path. It runs on every new shell, so it must stay fast and print only the script.
   - `pb` is decided at load time. The stub for `pb` checks then whether `pb` runs this binary, so setup stores nothing about it.
   - There is no config key and no migration. Existing installs get the stubs on the next `playbook setup`.

## Backups

The rc backups made by the uninstall strip are kept on purpose. `playbook uninstall` never removes them: `remove_backups` (`crates/cli/src/uninstall/mod.rs:312-330`) only touches `*.bak` files in the bin directory. The uninstall docs will say so and name the file for the user to delete by hand. The work unit that adds the uninstall step writes that note.

## Security note

A stub runs `playbook` from PATH, as the user. If a `playbook` earlier on PATH is not the one the user installed, that one runs. This is no worse than the user typing `playbook` at the prompt. Stubs print nothing and set no error when `playbook` fails, so a broken install never breaks shell startup.

## Alternatives considered

- **Print the line and let the user paste it.** Safe, but most users would not do it, and completion would stay unused.
- **Write the completion script itself into the autoload directory.** It goes stale after an upgrade. A stub always gets the script of the installed version.
- **Add a config key to turn it off.** There is nothing to remember: a user who does not want it answers no, and a removed stub stays removed until the next setup run asks again.

## Consequences

- `retired_rc_entries` (migration 0018 and doctor `layer3`) must keep ignoring the new marker. The block and the stubs must not match `shim::BLOCK_COMMENTS`, `SOURCE_LINE`, `BARE_EVALS` or a legacy `source` path, so doctor `layer3` stays PASS.
- Setup, uninstall and doctor each get a completions step.
- Because setup edits `~/.zshrc` again, the docs for setup say what is written and where.
- A change to the block text needs a new marker or a migration. Until then an edited block is left alone.
