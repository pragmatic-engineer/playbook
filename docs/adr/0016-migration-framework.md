# ADR-0016: A Registry of Ordered Migrations Run From `playbook init`

- **Status:** Accepted
- **Date created:** 2026-10-07
- **Date modified:** 2026-10-07

## Context

Issue #266 asks for a general way to carry users across breaking changes. The one-off migrations have shipped as separate code paths: the memory store move (`src/init/memory_migrate.rs`) and the repo-local state move (`migrate_legacy_repo_local` in `src/gate/db.rs`). ADR 0010 and ADR 0012 kept a general framework out of scope, and no ADR rejects it. Three questions stayed open: how a breaking change is detected, what the warning looks like, and where migrations are recorded.

## Decision

1. **An ordered registry.** `src/init/migrate.rs` holds a list of `Migration { id, kind, run }`. Ids are unique and append-only; the list order is the apply order.
2. **A persisted record.** `~/.config/playbook/migrations.state` (via `playbook_root_from`) holds one `applied <id>` line per finished Auto migration, and one `shipped <key> <hash>` line per file playbook placed.
3. **Auto migrations run once.** The id is recorded only after success. A failure is not recorded and stops later migrations, so order is preserved. A migration that is idempotent and self-healing may return `Repeat` and run on every init instead; both shipped one-offs do, because their existing call sites (hooks, gate commands) keep calling them directly.
4. **Manual items only warn.** A Manual migration never writes. It returns a message, which `playbook init` prints to stderr.
5. **Detection of user edits is by content hash.** When playbook places a file it records a hash of what it wrote. If the file's current hash differs from the record, the user edited it. A file with no record is never reported as edited. The hash is FNV-1a 64, chosen over a crypto hash because it must be stable across builds and the crate has no sha256 dependency; it detects edits, not tampering.
6. **Locking.** Pending Auto migrations run under a directory lock at `~/.config/playbook/migrations.lock` (`acquire_dir_lock`, `remove_stale_lock_dir`). The record is re-read under the lock, so concurrent sessions apply a migration once. If the lock cannot be taken, migrations are skipped with a warning, never run unguarded.
7. **Only `init` runs it.** Per-tool-call hooks never call the registry. When nothing is pending, the cost is one read of the state file plus the Manual hash checks.

## Alternatives considered

- **A schema version number per store.** Rejected: the stores migrate independently, and one integer cannot say which of them ran.
- **Running from `SessionStart`.** Rejected: it adds work to every session and races with `init`.
- **Overwriting user-edited files after backup.** Rejected: Manual items are warn-only by design.

## Consequences

- A new breaking change adds one registry entry instead of a new ad-hoc call site.
- The system prompt placement still replaces the file on init; the first Manual entry warns before that happens. Making placement skip an edited file is a follow-up.
- Per-repo migrations (the gate move) need a repo context, which `init` does not supply yet, so that entry is a no-op there and its call sites stay as they are.
- A `playbook doctor` row for pending Manual items is not included.
