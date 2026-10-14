# ADR-0031: Put the Hooks in settings.json

- **Status:** Accepted
- **Date created:** 2026-10-14
- **Date modified:** 2026-10-14
- **Supersedes:** decisions 2 and 3 of [ADR 0030](0030-skills-and-commands-in-their-own-repo.md), the parts about the `playbook-hooks` plugin

## Context

This returns to the decision of [ADR 0007](0007-rust-binary-for-hooks-and-launcher.md) to wire every hook through `settings.json`, which ADR 0030 had reversed for the hooks plugin.

ADR 0030 split the repository into a binary with its hooks, and a skills plugin. The hooks traveled in a second plugin, `playbook-hooks`, built as a zip on every release, pinned by hash in the marketplace and fronted by a `bin/playbook` shim.

That plugin cost more than it gave:

- It needed a release archive, an allowlist for the archive, a pin pull request in the marketplace and a version gate for Claude Code. All of that existed only to carry a list of `playbook hook <name>` commands.
- Two plugins had to be installed, kept in step and removed in the right order.
- The shim existed because the plugin could not rely on `playbook` being on `PATH`. The installer already puts the binary on `PATH`.

Claude Code already reads hooks from `~/.claude/settings.json`, and `playbook init` already edits that file.

## Decision

1. **The hooks live in `~/.claude/settings.json`.** `playbook init` writes one entry per hook, for every event, including the worktree hooks and the session start tool map. The commands use the bare form `playbook hook <name>`.
2. **Writing the entries is a choice the user can make.** In a terminal `init` asks. Without a terminal and without `--yes`, it writes no hooks and no status line and prints the command to run. `--hooks` or `--yes` is the explicit yes. `install.sh` asks for a typed yes before it runs `init`. The entries never go through `--settings`.
3. **The merge keeps the user's entries.** `init` adds its own entries next to existing ones and leaves the rest alone. An entry is ours when its command is exactly one of the `playbook hook <name>` commands `init` writes. `init` also rewrites in place a legacy entry whose script is named `<hook>.sh` or `<hook>.py` in the same event and matcher group. A second run changes nothing.
4. **Uninstall removes only the owned entries.** `playbook uninstall` deletes the entries that match those commands and keeps every entry the user wrote.
5. **The doctor checks the entries.** `playbook doctor` reports whether each entry is present and whether the command resolves to a working binary.
6. **Settings change only through the binary.** The `policy-guard` hook denies every Edit or Write on a Claude Code settings file (`settings.json` or `settings.local.json`, user or project), and every Bash command that writes one: a redirect, `tee`, `sed -i`, `perl -pi`, `cp`, `mv`, `install`, `rsync`, `ln`, `dd`, `rm`, `truncate` or an interpreter one-liner. The check reads the command text, so it cannot see a heredoc script, a script file or a path built at run time. It also denies `playbook uninstall --yes`. The binary handles consent, merging and the markers that tell its entries from the user's, so `playbook init`, `playbook uninstall`, `playbook config set` and `playbook doctor` are the way to change settings. Reading the files is allowed, and the user can still edit them by hand or with `!`.
7. **The installer installs only the binary.** `install.sh` puts the verified binary on `PATH` and then asks whether to run `playbook init`. With no terminal and no `--yes` it skips `init` and prints the command. It never installs or removes a Claude Code plugin.
8. **The hooks plugin is retired.** The `hooks.json` file, the zip and its allowlist, the `bin/playbook` shim, the pin step, `--skills-only` and the matching hints are removed. A user who still has `playbook-hooks` installed gets a doctor hint to uninstall it, because its hooks would run next to the ones in `settings.json`.

Decision 1 of ADR 0030 (the skills in their own repository) and its other decisions are not touched by this record.

## Alternatives considered

- **Keep the hooks plugin and simplify the pin.** Rejected: two plugins stay, and the archive stays.
- **Pass the hooks with `--settings` from the launcher.** Rejected: it works only for sessions started by `playbook cc`, and a flag on the command line is not visible to the user in a file they can read.
- **Write the entries without asking.** Rejected: the file belongs to the user, and the entries run code on every tool call.
- **Absolute paths in the commands.** Rejected: the path breaks when the binary moves, and the installer and Homebrew already put the binary on `PATH`.

## Consequences

- One install path for the hooks: `playbook init`. The doctor, the uninstall and the guard all look at the same entries.
- A user who upgrades from the hooks plugin runs `claude plugin uninstall playbook-hooks@pragmatic-engineer` and `playbook init` once.
- Pausing playbook by disabling the skills plugin still works, because the hooks read the plugin's enabled flag. A user who installs only the binary has no such flag, and the hooks run.
- A binary-only user fails the doctor check for the skills layer, because there is no skills plugin to find.
- The Homebrew formula caveat tells users to run `playbook init`.
