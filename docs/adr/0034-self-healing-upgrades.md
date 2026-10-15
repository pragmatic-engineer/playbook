# ADR-0034: One Plugin Root Resolver and Migrations Between Versions

- **Status:** Accepted. Decisions 13 and 14 are amended by [ADR 0037](0037-release-notes-from-conventional-commits.md), which replaces the ADR 0035 record. Decisions 9, 10 and 11 are amended by [ADR 0038](0038-one-release-line.md): the supported plugin range is the minor line of the binary, and `minBinaryVersion` is the `X.Y.0` of the release.
- **Date created:** 2026-10-15
- **Date modified:** 2026-10-16
- **Amends:** decision 7 of [ADR 0016](0016-migration-framework.md), which ran the registry only from `playbook init` and rejected running it from `SessionStart`. Decision 3 of [ADR 0031](0031-hooks-in-settings-json.md), which kept every hook entry that is not a bare `playbook hook <name>` command.

## Context

A user's skill failed with "could not find the plugin root folder". The binary printed "skill reference not found at skills/writing-style/SKILL.md", a message only binaries up to 0.25 have. The plugin had updated itself to 0.30, and the old binary looked for a cache directory named after its own version, which no longer existed. The binary and the plugin now have their own version lines (binary 0.26, plugin 0.27 to 0.30), and an upgrade leaves old state behind. Five problems on the binary side made this worse:

- Five call sites find the plugin files in their own way. `init` first looks for a cache directory named after the binary version, then falls back to the newest one. No plugin version is 0.26, so the first lookup misses on every run.
- The fallback picks the highest version directory under `~/.claude/plugins/cache/pragmatic-engineer/playbook`. Claude Code keeps old version directories there and marks them with `.orphaned_at`. The active one is the `installPath` in `~/.claude/plugins/installed_plugins.json`, which no code reads. An orphaned directory with a higher number wins.
- `pb` writes a temporary `playbook-variants` plugin with copies of the agent bodies. Claude Code fills `${CLAUDE_PLUGIN_ROOT}` in those copies with the temporary directory, which has no `skills/`.
- The migration registry runs only from `playbook init`. Nothing runs it after `playbook update` or a Homebrew upgrade, and nothing records which binary version last ran it.
- `playbook doctor` compares the binary version with the plugin version. They differ by design now, and the row shows on every run.

## Decision

1. **One resolver.** `plugin_root::resolve` finds the plugin's files. It takes the Claude home, the environment value and the current directory as inputs, and returns the directory and its source (`InstallPath`, `Env` or `Cache`). Callers read the version from the directory's `plugin.json`. The order is:
   1. `PLAYBOOK_PLUGIN_DIR`, an explicit override for a dev checkout. The name differs from `PLAYBOOK_PLUGIN_ROOT`, which the repository's cargo config sets for every test run;
   2. the `installPath` of `playbook@pragmatic-engineer` in `installed_plugins.json`. Both the list shape (version 2) and the single object shape (version 1) are read. An entry whose `projectPath` holds the current directory wins, the deepest one first, then the user entry. A missing or malformed file skips this step;
   3. `CLAUDE_PLUGIN_ROOT`, when it is set;
   4. the newest version directory in the cache.

   A candidate counts only when it is a directory with `agents/`, `commands/` or `skills/`, has no `.orphaned_at` marker, and its `.claude-plugin/plugin.json`, when present, names `playbook`. The binary version is not part of the lookup. A stale `installPath` fails these checks and the next step runs. When nothing counts, the error is one line naming each place tried and the install command. The old lookups in `init/self_root.rs` are deleted.
2. **The binary's own copies are a fallback outside the resolver.** The binary embeds the four skill files that `playbook pr rules` reads. `pr rules` uses them when the resolved plugin lacks one of the files, and says so on stderr. `init` seeds the settings template, system prompt and Brewfile from `CLAUDE_PLUGIN_ROOT` when that directory holds them and is not another plugin, else from the embedded copies. Seeds do not come from the cache, because an old plugin there ships an old settings template.
3. **Skills and agents ask the binary.** `playbook path plugin-root` prints the resolved directory, or the one line error with exit code 1. It does not print the embedded directory, because that holds only a few files. `intents.json` gets a `plugin-root` intent, so the contract check from ADR 0032 notices if the command goes away.
4. **Variant agents point at the real plugin.** `pb` replaces `${CLAUDE_PLUGIN_ROOT}` and `$CLAUDE_PLUGIN_ROOT` in the copied agent bodies with the resolved directory.
5. **A version record.** The `state` table of `config.db` in the data folder gets four keys next to the applied migration record: `versions/schema` (the highest migration number of the binary that last ran clean), `versions/binary` (its version), `versions/plugin` (the plugin version it saw, for doctor) and `versions/pending` (the ids of `Consent` and `Report` items still open). They are written in one transaction.
6. **Scope sits beside `Kind`.** `Kind` (`Auto`, `Idempotent`, `Manual`) still says how often a migration runs. A new `Scope` says what it writes. `Owned` writes only playbook's own files. `Consent` writes a file the user owns, has a check that reports without writing, and names the `init` command that applies it. `Report` writes nothing and reports on files Claude Code owns, and the hook repeats it until it clears. The ten existing entries get these scopes:

   | Id | Kind | Scope |
   |---|---|---|
   | 0001 memory store move | Idempotent | Owned (it moves playbook's memory files out of `~/.claude`, and `session-init` already runs it) |
   | 0002 gate repo local move | Idempotent | Owned (runs only with a repo, and the hook passes none) |
   | 0003, 0004, 0005 edited files | Manual | Owned (they write nothing. `init` and `doctor --fix` show the note, and the hook does not repeat it, because the user made the edit on purpose) |
   | 0006 statusline command | Idempotent | Consent (writes `settings.json`) |
   | 0007 shell rc line | Idempotent | Consent (writes a shell rc file) |
   | 0008, 0009, 0010 | Idempotent | Owned |

   One test runs every `Owned` and `Manual` migration against a scratch home and checks that no file outside `~/.config/playbook` changed.
7. **Four triggers run the same code.** Decision 13 adds a fifth, the first-run step.
   1. `playbook init` runs every migration, as it does today, with the consent `init` already asks for.
   2. `playbook update` runs `<new binary> migrate` after the swap, so the new binary runs its own migrations (decision 12). A new binary without `migrate` exits 2, and the update runs its `doctor --fix` instead. The step is skipped when the new version is older than the first release with `--fix`, and a failure is a warning, not an update failure.
   3. The `session-init` hook reads the version record. When the running binary's version and schema match it and `versions/pending` is empty, it does nothing more. When `versions/pending` is not empty, it reruns only those checks. When the binary is newer, it runs the `Owned` migrations under the migration lock, without waiting for it, then the `Consent` and `Report` checks. It adds one line to the session message naming what changed and the command to run. It asks no question, writes no Claude settings and does not fail the hook. When the record is newer than the running binary, a downgrade, the hook runs nothing and leaves the record alone.
   4. `playbook migrate` and `playbook doctor --fix` run the `Owned` migrations, then print the command for each open `Consent` or `Report` item and the notes of the `Owned` `Manual` checks. They exit 1 when a migration failed, the lock was busy or the record was not written, so `playbook update` shows a warning.

   Every trigger writes the version record only when it held the lock and every `Owned` migration finished. A busy lock or a failed migration leaves the record unchanged. After a failure the hook and the first-run step retry at most once a day, and `init`, `migrate` and `doctor --fix` retry at once. The per session cost on the fast path is one read of the `state` table, which the hook already opens for the worktree sweep.
8. **Only consent changes settings.** `init`, `uninstall` and `config` write `~/.claude/settings.json`, with consent. The hook, `migrate` and `doctor --fix` only report what needs `playbook init`.
9. **New migrations for the 0.25 to 0.26 gap.**
   - A hook entry in settings.json whose command runs a script under `~/.claude/hooks/`, with an optional `bash`, `sh` or `python3` in front, where the script no longer exists, is dead. A command with a shell operator or `$(` does not count as dead, because a later part may still run. `init` removes the entry after a backup when the hooks are on (`Consent`, `playbook init --hooks`), and the doctor's missing file row names that command. An entry whose script exists stays. This amends ADR 0031 decision 3 for the dead case only.
   - An installed `playbook-hooks` plugin, or a `playbook` plugin from another marketplace, is reported with its `claude plugin uninstall` command (`Manual`, `Report`). A `playbook` plugin from another marketplace counts only next to this one, and an install scoped to another project does not count. The doctor row uses the same check.
   - A plugin older than the minimum this binary supports is reported with `claude plugin update playbook@pragmatic-engineer`, and one with a major version above the one the binary knows is reported with the binary upgrade command (`Manual`, `Report`). The minimum is a constant in the binary, raised when the binary needs something new from the plugin. It starts at 0.30.1, the first plugin that no longer reads a bare `$CLAUDE_PLUGIN_ROOT` path.
   - Orphaned cache directories are reported and ignored by the resolver (`Manual`, `Owned`). The note shows in `init` and `doctor --fix`, and the hook does not repeat it, because Claude Code removes those folders itself. Playbook does not delete them.
10. **The plugin names the oldest binary it works with.** `plugin.json` holds `metadata.minBinaryVersion`. Claude Code accepts the `metadata` object without a warning. The `session-init` hook and the doctor read it from the resolved plugin. When the running binary is older, the hook adds one line to the session message: `playbook binary X is older than this plugin needs (Y). Run: brew upgrade playbook` on a Homebrew install, else `playbook update`.
11. **Doctor compares with the supported range.** The doctor row passes when the plugin version is inside the range from decision 9. It reports the gap only when the plugin is outside it.
12. **`playbook migrate` is the one front door.** It runs the `Owned` migrations with the code `doctor --fix` runs, and `doctor --fix` stays as the same step under its old name. An `Auto` migration starts once, after the upgrade that ships it. An `Idempotent` one runs on every run, because it repairs a file that may have come back. Nothing runs after a downgrade.
    - `--status` lists each migration with its scope and state, and the recorded and running versions. A state is applied, pending, runs on every run, nothing to do, needs you with its command, or not checked while old state files wait to be imported.
    - `--dry-run` names the migrations a run would start and the items that need the user. It writes nothing. It reads the record once through read-only connections, and every check sees that copy, so no store is created or imported.
    - `--json` prints the same as JSON for tools.
    - It exits 0 when it finished or had nothing to do, and 1 when a migration failed, the lock was busy or the record was not written. `--status` and `--dry-run` exit 0 unless the record cannot be read.
    - `Consent` and `Report` items print their command and are not applied.

    Each channel runs it after it installs a binary. `playbook update` runs the new binary's `migrate` and warns when it takes over 120 seconds. The install script runs it when it has a terminal or `--yes`, and skips it with `--no-init` or on a binary that has no `migrate`. Neither writes Claude settings. The Homebrew formula has no `post_install` step, because the first-run step in decision 13 heals on the first command after `brew upgrade`.
13. **The first run of a new binary heals, before most commands.** One step in `main()`, after the arguments parse and before the command runs, reads the version record. When it names an older binary, the step runs the `Owned` migrations once with the code `migrate` runs, then the command goes on.
    - It asks no question, writes no Claude settings and does not fail the command: a panic or an unreadable store is ignored.
    - It does not wait for the migration lock. A busy lock skips the step, and the next command tries again.
    - It starts no migration after 2 seconds; one that is already running finishes. It then leaves the record unchanged and sets no failure time. The next command runs the `Idempotent` migrations again from the start and skips the `Auto` ones already applied.
    - After a failed migration it records the failure time and, like the hook, retries at most once a day. While it waits it prints nothing and costs one read of the record.
    - It prints one short notice on stderr: how many files changed, the `init` commands for changes to settings or shell files, and `playbook migrate` for the other items. Standard output stays the command's own, so `--json` output is unchanged. The notice shows once per version change, because a clean run records the new binary.
    - Like every step that runs these migrations, it shows the ADR 0035 breaking changes block for the versions the user crossed, on stderr. The block shows once, because `notices/binary` records it whether the heal succeeds or not.
    - It does nothing after a downgrade, on a home with no playbook folder yet, or when the record already names the running binary. That last check is one read of the record through a read-only connection.

    Hot paths skip the step. Hooks fire on every tool call, and the status line redraws every few hundred milliseconds, so `playbook hook` and `playbook statusline` return through the fast path before the arguments parse. `shell-init` runs on every new shell, and the hidden `json` helpers run inside hook scripts, so they skip it too, as does `version`. One list, `Command::is_hot_path`, names them. The session hooks start `usage` in the background, so `usage` skips the step as well. On that side, the `session-init` hook is the trigger, once per session (decision 7, trigger 3). Commands that run their own heal (`migrate`, `doctor --fix`, `init`), replace the binary (`update`) or remove playbook (`uninstall`) skip it. So do `doctor` and `state`, which show the home as it is; the doctor row names `playbook migrate` when a heal is due.
14. **Every release is backward compatible or migrates.** A change to playbook's config, its data files, the entries it writes into Claude Code settings, or the contract between the binary and the plugin has to keep working for a user who upgrades. When it cannot, it ships a registry entry that the first-run step runs. Either way the change is listed in `breaking-changes.json`, as ADR 0035 decision 2 requires: one a migration handles names the migration with the action `none, migrated automatically`, and one that needs the user says what to do. ADR 0035 turns the file into the notice every update channel shows.

## Alternatives considered

- **Trust `CLAUDE_PLUGIN_ROOT` first.** Rejected: Claude Code exports it only to hooks and MCP servers, and `pb` points it at the variants plugin. `installPath` is what Claude Code loads. A dev checkout uses `PLAYBOOK_PLUGIN_DIR`.
- **Embed the whole plugin in the binary.** Rejected: it adds about 700 KB. Only the four files the binary reads itself are embedded.
- **Delete orphaned plugin cache directories.** Rejected: Claude Code owns and removes them, and an old directory holds any skill edits a user made there.
- **Rewrite old `.sh` hook entries to `playbook hook <name>`.** Rejected: an entry whose script exists is the user's own. A missing script proves the entry is dead.
- **A separate `state.json` file.** Rejected: the version record and the applied record would live in two stores and could disagree. The `state` table writes both in one transaction.
- **Move `telemetry-env.json` into the data folder.** Rejected for now: an older binary's `uninstall` reads the old path, and a downgrade would leave the telemetry keys in settings.json.

## Consequences

- A binary upgrade through `playbook update` or the install script heals the playbook files right after the install. One through Homebrew or a copied file heals them on the first command or session start after it. None of them needs a run of `init`.
- The first command after an upgrade can take about 2 seconds longer, plus the migration still running at that mark and the checks that follow. Every later command pays one read of the record.
- A binary released before this change cannot heal itself or read `minBinaryVersion`. For those, the only fix is an upgrade, and the plugin's own text has to tell the user, because the old binary runs the hooks.
- Settings problems still need the user. The hook repeats the command each session until the item is fixed.
- A dev checkout no longer wins over an installed plugin through `CLAUDE_PLUGIN_ROOT`. It needs `PLAYBOOK_PLUGIN_DIR`.
- The embedded rule files follow the binary's release, so they can lag the plugin. `pr rules` uses them only when the plugin files are missing, and says so. This replaces the error ADR 0030 describes.
- A new breaking change adds a registry entry with a `Kind` and a `Scope`. A wrong scope is the main risk, and the registry wide test catches an `Owned` migration that writes outside playbook's files.
- The doctor row stops warning about versions that differ by design. The wording in the plugin's `doctor` command changes in the plugin, tracked in issue #1024.
