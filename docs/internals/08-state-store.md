# State store

Playbook keeps small bits of internal state in the `state` table of its SQLite store, `config.db` in the [data folder](11-data-folder.md) (earlier versions called it `playbook.db`). This is the same file that holds your config. It replaces a set of loose files.

## What lives there

| Key prefix | Holds | Used by |
|---|---|---|
| `migrations/applied/<id>` | One row per finished Auto migration | `playbook setup` |
| `migrations/shipped/<key>` | The hash of each file playbook placed | `playbook setup` edit detection |
| `worktree-sweep/<repo slug>` | Epoch seconds of the last worktree sweep | The session-start hook |
| `notices/items/<slug>` | For each item that needs you: a hash of its text, when it last showed, and whether `playbook migrate` printed it | The session-start hook and `playbook migrate` |

Each row has a key, a text value and an update time. Writes that belong together (for example, rewriting all migration rows) run in one transaction. A failed write rolls back and leaves the old rows.

## Look at it

```
playbook state list
playbook state list migrations/
playbook state list --json
```

`playbook config export` and `import` cover config only. To copy state to another machine, copy `~/.config/playbook/data/config.db`.

## The one-time import

The first time the store opens, playbook reads the old files and moves them into the table:

- `~/.config/playbook/migrations.state` (lines `applied <id>` and `shipped <key> <hash>`)
- every `worktree-sweep-marker-*` file

The import is one transaction. After it commits, each old file is renamed to `<name>.migrated`, and a renamed marker is not imported again. If any file cannot be read, nothing is imported and nothing is renamed, and the next run tries again. The `state-files-to-sqlite` migration makes sure this has happened and `playbook setup` reports how many files it moved.

## Safety

The auto-mode guard blocks agents from touching `config.db`, `memory.db`, their `-wal` and `-shm` files, the data folder and the old `playbook.db`, so the state is covered by the same rule as config.
