# Internals: Migrations

The migration registry carries an existing install across breaking changes. It lives in `crates/cli/src/init/migrate.rs`. The reasoning is in [ADR 0016](../adr/0016-migration-framework.md) and [ADR 0034](../adr/0034-self-healing-upgrades.md). This page says how it behaves, so you know what runs when you update.

## When it runs

Each migration also has a scope: `Owned` writes only playbook's own files, `Consent` writes a file you own, and `Report` writes nothing and reports on files Claude Code owns. Only `Owned` migrations run on their own:

- `playbook migrate` runs them and prints the command for each item that needs you. `playbook doctor --fix` is the same step. `--status` and `--dry-run` show what stands and what would run, and write nothing.
- The first command after an upgrade runs them once, before the command, and prints one notice on stderr. Hooks, the status line and `shell-init` skip this, so they stay fast.
- The `session-init` hook runs them once per session when the binary is newer than the record, and adds one line to the session message.
- `playbook update` runs the new binary's `migrate` after the swap. The install script runs it after an install.

`playbook init` runs every migration, `Consent` ones included, because it asks first. Nothing else writes Claude Code settings or your shell files. The version record in the `state` table says which binary last ran the `Owned` migrations cleanly, so each of these runs once per version.

## The three kinds

Each migration has an id such as `0006-statusline-rust-command`. Ids are append only. A migration is one of three kinds:

| Kind | Runs | Recorded | What it does |
|---|---|---|---|
| Auto | Once, until it succeeds | Yes, as `applied <id>` | Moves or rewrites state, then never runs again. A failure is not recorded and stops the later Auto migrations, so order holds. |
| Idempotent | On every `init`, `migrate` and heal | No | Repairs state and does nothing when there is nothing to repair. It takes no lock, and a failure in one does not block the others. |
| Manual | On every `init` and in `playbook doctor pending-migrations` | No | Never changes anything. It prints a warning that tells you what to do. |

No Auto migration ships today. The current entries are:

| Id | Kind | What it does |
|---|---|---|
| `0001-memory-store-move` | Idempotent | Moves the old memory store to `~/.config/playbook`. |
| `0002-gate-repo-local-move` | Idempotent | Moves repo-local gate state to the per-repo store, only when init runs inside a repo. |
| `0003-system-prompt-edited` | Manual | Warns when you edited the installed system prompt. |
| `0004-skills-edited` | Manual | Warns when you edited a cached plugin skill. |
| `0005-statusline-edited` | Manual | Warns when you edited the old status line script. |
| `0006-statusline-rust-command` | Idempotent | Points `statusLine.command` at `playbook statusline`. |
| `0007-shell-init-rc-line` | Idempotent | Rewrites an old `source .../cc.sh` rc line to `eval "$(playbook shell-init)"`. |
| `0008-remove-config-hash-script` | Idempotent | Removes the `hooks/lib/config-hash.sh` copy older installs placed under `~/.config/playbook`. The hash is computed inside the binary now. |
| `0009-adopt-late-config-json` | Idempotent | Adopts a `config.json` written after the SQLite import when it agrees with the store, and warns with its path when it does not. |
| `0010-state-files-to-sqlite` | Idempotent | Moves `migrations.state` and the worktree sweep markers into the `state` table. See [State store](08-state-store.md). |

## How playbook knows you edited a file

When playbook places a file (the system prompt, a status line script), it records a hash of what it wrote in the `state` table (see [State store](08-state-store.md)), under a `migrations/shipped/<key>` key. If the file's hash later differs, you edited it. A file with no record is never reported as edited. The hash is FNV-1a 64. It detects edits and does not protect against tampering.

Playbook does not overwrite a file you edited. It leaves it, prints a warning, and tells you how to take the shipped copy (delete the file, then run `playbook init` again). Skills are checked per plugin version folder, and a dev checkout (a folder with `.git`) is skipped.

## State and locking

- The record is the `state` table: one `migrations/applied/<id>` row per finished Auto migration, and one `migrations/shipped/<key>` row per placed file. Migration 0010 imports the old `migrations.state` file.
- Pending Auto migrations run under the directory lock `~/.config/playbook/migrations.lock`. The record is read again under the lock, so two sessions apply a migration once. A lock older than 300 seconds counts as stale.
- If the lock cannot be taken, Auto migrations are skipped and `init` says so. If the record cannot be read (anything except a missing file), the run is skipped and never treated as empty.

## Add a migration

1. Append an entry to `registry()` in `crates/cli/src/init/migrate.rs` with the next id. Never reorder or reuse an id.
2. Pick the kind: Auto for a one-time move, Idempotent for a self-healing repair, Manual for something only you can resolve.
3. Pick the scope: `Owned` when it writes only playbook's own files, `Consent` with a check and an `init` command when it writes a file the user owns, `Report` when it only reports.
4. Add a test next to the existing ones in that file.
5. A change that needs the user also gets an entry in `breaking-changes.json` ([ADR 0035](../adr/0035-breaking-change-notices.md)).

See also: [Launcher and hooks](01-launcher-and-hooks.md), [Docs index](../index.md).
