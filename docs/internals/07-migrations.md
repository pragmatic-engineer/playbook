# Internals: Migrations

The migration registry carries an existing install across breaking changes. It lives in `crates/cli/src/init/migrate.rs`. The reasoning is in [ADR 0016](../adr/0016-migration-framework.md), [ADR 0034](../adr/0034-self-healing-upgrades.md) and [ADR 0046](../adr/0046-migrations-by-release-version.md). This page says how it behaves, so you know what runs when you update.

## When it runs

Each migration also has a scope: `Owned` writes only playbook's own files, `Consent` writes a file you own, and `Report` writes nothing and reports on files Claude Code owns. Only `Owned` migrations run on their own:

- `playbook migrate` runs them and prints the command for each item that needs you. `playbook doctor --fix` is the same step. `--status` and `--dry-run` show what stands and what would run, and write nothing.
- The first command after an upgrade runs them once, before the command, and prints one notice on stderr. Hot paths skip this so they stay fast: hooks, the status line, `version` and the hidden `json` helpers. So do `usage`, which the session hooks start in the background, the commands that run their own heal (`migrate`, `doctor --fix`, `setup` or its old name `init`), `update`, `uninstall`, and `doctor` and `state`, which show the home as it is.
- The `session-init` hook runs them once per session when the binary is newer than the record, and adds one line to the session message.
- `playbook update` runs the new binary's `migrate` after the swap. The install script runs it after an install when it has a terminal or `--yes`, and not with `--no-init`.

`playbook setup` runs every migration, `Consent` ones included, because it asks first. Nothing else writes Claude Code settings or your shell files, with one exception: `playbook doctor --fix` adds the missing `attribution` keys after a yes at a terminal ([ADR 0042](../adr/0042-attribution-off-in-setup.md)). The version record in the `state` table says which playbook version last ran the `Owned` migrations cleanly, so the first-run step and the session hook run once per version. The record is compared with the running build by version alone, and a record newer than the build means a downgrade, where nothing runs. `migrate` and `doctor --fix` run every time you call them.

## The three kinds

Each migration has an id, a slug such as `statusline-rust-command`, and a `since`: the release that ships it. A build runs and checks only the migrations whose `since` is at or below its own version, and `playbook migrate --status` lists a later one as waiting for the next release. A migration that waits for the next release says `next`, and the release pull request writes the version in its place, so a dev build of `main` never runs it early. Ids are never renamed or reused. A migration is one of three kinds:

| Kind | Runs | Recorded | What it does |
|---|---|---|---|
| Auto | Once, until it succeeds | Yes, as `applied <id>` (`migrations/applied/<id>`) | Moves or rewrites state, then never runs again. A failure is not recorded and stops the later Auto migrations, so order holds. |
| Idempotent | On every `setup`, `migrate` and heal | No | Repairs state and does nothing when there is nothing to repair. It takes no lock, and a failure in one does not block the others. |
| Manual | On every `setup` and in `playbook doctor pending-migrations` | No | Never changes anything. It prints a warning that tells you what to do. |

`auto-budget-defaults` is the first Auto migration. The current entries include:

| Id | Since | Kind | What it does |
|---|---|---|---|
| `memory-store-move` | 0.18.0 | Idempotent | Moves the old memory store to `~/.config/playbook`. |
| `gate-repo-local-move` | 0.18.0 | Idempotent | Moves repo-local gate state to the per-repo store, only when setup runs inside a repo. |
| `system-prompt-edited` | 0.18.0 | Manual | Warns when you edited the installed system prompt. |
| `skills-edited` | 0.18.0 | Manual | Warns when you edited a cached plugin skill. |
| `statusline-edited` | 0.18.0 | Manual | Warns when you edited the old status line script. |
| `statusline-rust-command` | 0.18.0 | Idempotent | Points `statusLine.command` at `playbook statusline`. |
| `shell-init-rc-line` | 0.18.0 | Idempotent | Retired by [ADR 0044](../adr/0044-drop-the-shell-launcher.md). It reports nothing and changes nothing. |
| `remove-config-hash-script` | 0.20.0 | Idempotent | Removes the `hooks/lib/config-hash.sh` copy older installs placed under `~/.config/playbook`. The hash is computed inside the binary now. |
| `adopt-late-config-json` | 0.20.0 | Idempotent | Adopts a `config.json` written after the SQLite import when it agrees with the store, and warns with its path when it does not. |
| `state-files-to-sqlite` | 0.20.0 | Idempotent | Moves `migrations.state` and the worktree sweep markers into the `state` table. See [State store](08-state-store.md). |
| `dead-hook-scripts` | 0.34.0 | Idempotent, Consent | Removes hook entries in `settings.json` whose script under `~/.claude/hooks/` is gone, after a backup. `playbook migrate` lists it with `playbook setup --hooks`. |
| `other-playbook-plugins` | 0.34.0 | Manual, Report | Reports the retired `playbook-hooks` plugin, or a `playbook` plugin from another marketplace next to this one, with its `claude plugin uninstall` command. |
| `plugin-version-range` | 0.34.0 | Manual, Report | Reports a plugin outside the minor line of this binary, with the command that fixes it. |
| `orphaned-plugin-caches` | 0.34.0 | Manual | Notes old plugin folders Claude Code marked for removal. Playbook ignores them and never deletes them. |
| `attribution-off` | 0.36.0 | Idempotent, Consent | Open while `settings.json` lacks the `attribution` keys and you have not said no. `playbook migrate` lists it with `playbook setup --only attribution-off`, which asks and writes them. See [ADR 0042](../adr/0042-attribution-off-in-setup.md). |
| `collector-service` | 0.36.0 | Idempotent, Consent | Reminds you once when telemetry points at the local collector and no service keeps it running. `playbook migrate` lists it with `playbook setup --only collector-service`, which asks and installs the service. See [ADR 0043](../adr/0043-collector-service-in-setup.md). |
| `auto-budget-defaults` | 0.36.0 | Auto | Moves a stored `auto.budgetUsd` or `auto.agentBudgetUsd` of exactly 5, the old default, to the new default once, held at a set global `auto.sessionCeilingUsd`. See [ADR 0028](../adr/0028-per-agent-auto-budget.md). |
| `retired-launcher-rc-line` | 0.37.0 | Idempotent, Consent | Removes the launcher lines older setups wrote to `~/.zshrc` and `~/.bashrc`, after a copy to `<rc>.bak-<time>`. `playbook migrate` lists it with `playbook setup`, which applies it. A hand-written `eval "$(playbook shell-init)"` at the start of a line goes too, since the command is removed. See [ADR 0044](../adr/0044-drop-the-shell-launcher.md). |
| `system-prompt-default-on` | 0.37.0 | Idempotent, Consent | Open on an earlier install that has playbook hooks or the status line, no system prompt file and no stored `systemPrompt.enabled`. `playbook migrate` lists it with `playbook setup --only system-prompt`. A yes or a no in setup stores the key and closes it. See [ADR 0044](../adr/0044-drop-the-shell-launcher.md). |
| `collector-service-label` | next | Idempotent | On macOS, moves the collector agent that 0.37.0 installed as `io.github.pragmatic-engineer.playbook.collector` to `com.github.pragmatic-engineer.playbook.collector`: boots out the old job, loads the new agent with the same program and log, deletes the old file, and checks that the collector runs. A failed start puts the new agent file back as it was, loads the old agent again if it was loaded before, and names `playbook migrate`. Setup moves the old agent the same way before it installs. A file playbook did not write is only reported. See [ADR 0043](../adr/0043-collector-service-in-setup.md#amendment-2026-10-18-the-launchd-label). |
| `disk-guard-hook` | next | Idempotent, Consent | Open while `settings.json` has playbook's guards and no `playbook hook disk-guard`. `playbook migrate` lists it with `playbook setup --hooks`, which wires it. An install without the guards declined the hooks and stays quiet. See [ADR 0047](../adr/0047-disk-guard.md). |

## How playbook knows you edited a file

When playbook places a file (the system prompt, a status line script), it records a hash of what it wrote in the `state` table (see [State store](08-state-store.md)), under a `migrations/shipped/<key>` key. If the file's hash later differs, you edited it. A file with no record is never reported as edited. The hash is FNV-1a 64. It detects edits and does not protect against tampering.

Playbook does not overwrite a file you edited. It leaves it, prints a warning, and names the command that takes the shipped copy: delete the file, then run `playbook setup --system-prompt` for the system prompt. The old status line script has no shipped copy, so the warning only says to delete it once nothing runs it. Skills are checked per plugin version folder, and a dev checkout (a folder with `.git`) is skipped.

## State and locking

- The record is the `state` table: one `migrations/applied/<id>` row per finished Auto migration, one `migrations/shipped/<key>` row per placed file, and the `versions/` keys: `versions/binary` (the version that last ran clean), `versions/schema` (always 20, for older binaries), `versions/plugin`, `versions/pending` (the ids still open) and `versions/failed-at`. `state-files-to-sqlite` imports the old `migrations.state` file.
- Releases up to 0.37.0 named migrations by number, such as `0017-auto-budget-defaults`. A row or a pending id under one of those nineteen old ids reads as its slug. Every run that runs migrations adds a slug row next to each old applied row and keeps the old row, and applying one of those migrations writes both rows, so an older playbook still finds the id it knows. The hook, the first-run step, `migrate` and `doctor --fix` of an older playbook read the record as newer and run nothing. Its `setup` still runs its own registry, and finds the numbered row of `0017-auto-budget-defaults`, so that one does not run again.
- Pending Auto migrations run under the directory lock `~/.config/playbook/migrations.lock`. The record is read again under the lock, so two sessions apply a migration once. A lock older than 300 seconds counts as stale.
- If the lock cannot be taken, Auto migrations are skipped and `setup` says so. If the record cannot be read (anything except a missing file), the run is skipped and never treated as empty.

## Add a migration

1. Append an entry to `registry()` in `crates/cli/src/init/migrate.rs` with a new slug and `since: "next",` on its own line, in exactly that form. `scripts/release-prepare.sh` replaces it with the release version and fails the release when one is left. Never rename or move an entry.
2. Pick the kind: Auto for a one-time move, Idempotent for a self-healing repair, Manual for something only you can resolve.
3. Pick the scope: `Owned` when it writes only playbook's own files, `Consent` with a check and a `setup` command when it writes a file the user owns, `Report` when it only reports.
4. Add a test next to the existing ones in that file. A dev build skips a `next` migration, so the test calls it directly or passes a version to `shipped`.
5. A change that needs the user is a breaking change: mark its commit with `!` or a `BREAKING CHANGE:` footer that says what to do ([ADR 0037](../adr/0037-release-notes-from-conventional-commits.md)).

See also: [Launcher and hooks](01-launcher-and-hooks.md), [Docs index](../index.md).
