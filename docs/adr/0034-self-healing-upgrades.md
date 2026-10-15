# ADR-0034: One Plugin Root Resolver and Migrations Between Versions

- **Status:** Accepted
- **Date created:** 2026-10-15
- **Date modified:** 2026-10-15
- **Amends:** decision 7 of [ADR 0016](0016-migration-framework.md), which ran migrations only from `playbook init`

## Context

A user upgraded the binary from 0.25 to 0.26 and a skill failed with "could not find the plugin root folder". The binary and the plugin now have their own version lines (binary 0.26, plugin 0.27 to 0.30), and an upgrade leaves old state behind. Five problems on the binary side made this worse:

- Five call sites find the plugin files in their own way. `init/self_root.rs` first looks for a cache directory named after the binary version. No plugin version is 0.26, so this lookup misses.
- The cache fallback picks the highest version directory. Claude Code keeps old version directories and marks them with `.orphaned_at`, and the active one is the `installPath` in `~/.claude/plugins/installed_plugins.json`. An old directory with a higher number, or a stale `installPath`, wins.
- `pb` writes a temporary `playbook-variants` plugin with copies of the agent bodies. Claude Code fills `${CLAUDE_PLUGIN_ROOT}` in those copies with the temporary directory, which has no `skills/`.
- Migrations run only from `playbook init`. Nothing runs after `playbook update` or a Homebrew upgrade, and nothing records which binary or plugin version last ran.
- `playbook doctor` compares the binary version with the plugin version. They differ by design now, and the row shows on every run.

## Decision

1. **One resolver.** `plugin_root::resolve` is the only code that finds the plugin files. It returns the directory and its source. The order is:
   1. the `installPath` of `playbook@pragmatic-engineer` in `installed_plugins.json`, preferring a project entry whose `projectPath` holds the current directory, then the user entry;
   2. `CLAUDE_PLUGIN_ROOT`, when it is set;
   3. the newest version directory under `~/.claude/plugins/cache/pragmatic-engineer/playbook`.

   A candidate counts only when it is a directory with `agents/`, `commands/` or `skills/`, has no `.orphaned_at` marker, and its `.claude-plugin/plugin.json`, when present, names `playbook`. The binary version is not part of the lookup. When nothing counts, the error is one line naming each place tried and the install command.
2. **The binary's own copies are the last source.** The binary embeds the four skill files that `playbook pr rules` reads, and the settings template, system prompt and Brewfile `init` seeds from. `pr rules` falls back to the embedded files. `init` seeds from `CLAUDE_PLUGIN_ROOT` when it holds the seed files, else from the embedded copies. It does not seed from a cache directory, because an old plugin there ships an old settings template.
3. **Skills and agents ask the binary.** `playbook path plugin-root` prints the resolved directory, or the one line error with exit code 1. It does not print the embedded directory, because that holds only a few files.
4. **Variant agents point at the real plugin.** `pb` replaces `${CLAUDE_PLUGIN_ROOT}` and `$CLAUDE_PLUGIN_ROOT` in the copied agent bodies with the resolved directory.
5. **A small state file.** `~/.config/playbook/data/state.json` records `schema` (the highest migration number this binary knows), `binary` (the last binary version that ran the migrations) and `plugin` (the last plugin version it saw). The applied migration record stays in the `state` table of `config.db`.
6. **Each migration declares a scope.** `Owned` migrations change only files under `~/.config/playbook`. `Consent` migrations change a file the user owns, such as `~/.claude/settings.json` or a shell rc file, and carry a check that reports without writing. `Report` migrations only warn, which covers Claude Code's own files, such as plugin installs and the plugin cache. Playbook does not delete or edit those.
7. **Four triggers run the same code.**
   1. `playbook init` runs every migration, as it does today, with the consent `init` already asks for.
   2. `playbook update` runs `<new binary> doctor --fix` after the swap, so the new binary runs its own migrations.
   3. The `session-init` hook compares `state.json` with the running binary and the resolved plugin version. When they match, it does nothing more. When they differ, it runs the pending `Owned` migrations under a lock it does not wait for, runs the `Consent` and `Report` checks, records the versions, and adds one line to the session message naming what changed and the one command to run. It asks no question, writes no Claude settings and does not fail the hook. A migration error leaves the record unchanged, so the next session tries again.
   4. `playbook doctor --fix` runs the `Owned` migrations, then prints the command for each pending `Consent` or `Report` item.
8. **Only consent changes settings.** `init`, `uninstall` and `config` write `~/.claude/settings.json`, with consent. The hook and `doctor --fix` only report what needs `playbook init`.
9. **New migrations for the 0.25 to 0.26 gap.** A hook entry in settings.json that runs `~/.claude/hooks/<guard>.sh` or `.py` for a script that no longer exists is removed by `init`, after a backup. An entry whose script exists is the user's and stays. An installed `playbook-hooks` plugin, or a `playbook` plugin from another marketplace, is reported with its `claude plugin uninstall` command. A plugin older than the minimum this binary supports is reported with `claude plugin update playbook@pragmatic-engineer`. Orphaned cache directories are reported and ignored by the resolver. `telemetry-env.json` moves into the data folder, and readers check the old path too.
10. **Doctor compares with a minimum.** The doctor row passes when the plugin version is at or above the minimum this binary supports. It reports the gap only when the plugin is older.

## Alternatives considered

- **Trust `CLAUDE_PLUGIN_ROOT` first.** Rejected: Claude Code exports it only to hooks and MCP servers, and `pb` points it at the variants plugin. `installPath` is what Claude Code loads.
- **Embed the whole plugin in the binary.** Rejected: it adds about 700 KB, and the copy would drift from the installed plugin. Only the files the binary reads itself are embedded.
- **Delete orphaned plugin cache directories.** Rejected: Claude Code owns and removes them, and an old directory holds any skill edits a user made there.
- **Rewrite old `.sh` hook entries to `playbook hook <name>`.** Rejected: an entry whose script exists is the user's own, as ADR 0031 says. A missing script proves the entry is dead.
- **Run migrations on every session start.** Rejected: the version check costs two small file reads, and a full run costs more on every session.

## Consequences

- A binary upgrade through `playbook update`, Homebrew or a copied file heals itself on the next session start, without a run of `init`.
- Settings problems still need the user. The hook names the command and leaves the file alone.
- A new breaking change adds a registry entry with a scope. A wrong scope is the main risk, so each new migration needs a test showing it leaves user content alone.
- The doctor row stops warning about versions that differ by design. The wording in the plugin's `doctor` command changes in the plugin, tracked in issue #1024.
