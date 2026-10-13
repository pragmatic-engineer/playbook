# Data folder

Every database a playbook tool creates lives in one folder, `~/.config/playbook/data`. The folder is owner only (mode 0700) and is made the first time a database is opened. It is the same folder for `playbook`, `playbook-telemetry` (`pbtop`) and `playbook-code`. `playbook path data` prints it.

## What is in it

| File | Holds | Safe to delete |
|---|---|---|
| `config.db` | All settings: the global, org, repo and per-app tiers, plus the internal `state` table | No: it is your configuration |
| `memory.db` | The memory graph: one row per fact and per link, built from the fact files | Yes: `playbook memory rebuild` makes it again |
| `usage.db` | Token and cost events from the usage view | No: events outlive Claude Code's own history |
| `gate/<owner>/<repo>/<worktree-id>.db` | Gate records of one worktree (`gate record`, `gate check`) | Only when the worktree is gone |
| `code/<owner>/<repo>/<worktree-id>.db` | The code index of one worktree | Yes: `playbook code index` rebuilds it |

The per-worktree files keep two worktrees of the same repo apart, as the old `repos/<owner>/<repo>/<worktree-id>/` folders did. Memory facts stay Markdown files under `~/.config/playbook/memory`, and they are the source of truth for `memory.db`. Plans, implement notes and worktree folders stay under `repos/`.

## How databases got here

Earlier versions kept these files elsewhere:

| Now | Before |
|---|---|
| `data/config.db` | `~/.config/playbook/playbook.db` |
| `data/memory.db` | `~/.config/playbook/memory/memory.graph.json` |
| `data/gate/<owner>/<repo>/<worktree-id>.db` | `~/.config/playbook/repos/<owner>/<repo>/<worktree-id>/state.db` |
| `data/code/<owner>/<repo>/<worktree-id>.db` | `~/.config/playbook/repos/<owner>/<repo>/<worktree-id>/code/index.sqlite3` |

The first command that needs a database moves the old file. The move never deletes anything:

- The old file is copied with SQLite's `VACUUM INTO`, which reads a consistent snapshot including the write-ahead log. The copy is checked table by table and renamed into place only if the row counts match. While it runs, playbook holds the old file's write lock, so no writer can add a row between the copy and the rename.
- The old file, with its `-wal` and `-shm` files, is renamed to `<name>.migrated` (`.migrated.1`, `.migrated.2` if that name is taken). The code index is a cache, so it moves by a plain rename instead.
- A lock folder next to the new file (`<name>.db.migrate.lock`) keeps two processes from moving the same file. A lock older than five minutes is treated as left behind by a killed process.
- One notice line goes to stderr, and only when stderr is a terminal. Hooks never print it.

If the move cannot finish (another process holds the lock, or the old file is busy), the command keeps using the old file for that call, so a setting or a gate verdict is never hidden behind an empty new file. The next call tries again.

If both files exist, because an older playbook recreated the old path, the rows are merged by their primary key and the old file is renamed again:

- `config.db`: the row with the later `updated_at` wins. Values imported from a leftover `config.json` count as oldest.
- Gate phase rows: the later `recorded_at` wins. The rounds of one plan and phase come whole from one file, never mixed.
- Rows the old file has and the new one lacks are added.

The graph is the one exception to copying. `memory.graph.json` is derived from the fact files, so `memory.db` is rebuilt from them and the file is renamed to `memory.graph.json.migrated`. An older playbook that writes the file again gets it folded in the same way the next time a tool reads the graph. `playbook memory export --json` prints the graph in the shape the file had.

A file that is not a database is left where it is, or renamed to `.migrated` for the settings database, and the notice names it. Older playbook versions (0.25.0 and earlier) keep working but read and write the old paths, so upgrade every installed copy.

## Look at it

`playbook doctor` has a row for the folder: its path, how many files and how many kilobytes, any `*.migrated` leftovers or `.tmp-` copies, and per-worktree databases that match no worktree of this clone. Those belong to removed worktrees or to another clone of the same repo, so check before deleting.

`playbook worktree sweep` and `playbook worktree remove` delete the gate and code databases of the worktree they remove, unless another registered worktree still maps to the same id.

## Uninstall

`playbook uninstall` leaves the data folder alone, like memory and every other piece of state.

## Repo folders

Files that are not databases live under `~/.config/playbook/repos/<owner>/<repo>/`. `<owner>/<repo>` comes from the `origin` remote, and the folder is the same from every checkout and every worktree of the repo, so anyone can work the path out with `git remote get-url origin`.

| Folder | Holds | Printed by |
|---|---|---|
| `plans/` | Plans, their quality reports and checkpoints | `playbook path plans` |
| `implement/` | Per Work Unit briefs and progress notes | `playbook path implement` |
| `worktrees/` | Worktrees `/playbook:implement` creates | `playbook path worktrees` |
| `handoff/` | Handoffs saved with `playbook handoff save` | `playbook path handoff` |

Earlier versions kept `plans/`, `implement/` and `worktrees/` one level deeper, in `<worktree-id>/`, and handoffs for every directory in `runtime/handoff/`. The first call that needs a folder copies the old files into the new one and never deletes or overwrites anything:

- `plans/` and `implement/` are copied from every old `<worktree-id>/` folder. A name that already exists in the new folder is left alone, and the clash is counted in the note.
- `worktrees/` is not copied. It holds live git checkouts, which a copy would break. Old ones stay where they are and keep working.
- Handoffs for the current directory, and for the directories under it, are copied from `runtime/handoff/` the first time a session starts there. A marker in `handoff/.copied/` stops a handoff that a session already read from coming back.
- A `MOVED.txt` beside the old files says where the copy went and marks the copy as done. One line goes to stderr when files were copied.

Outside a repo with an `origin` remote, handoffs stay in `runtime/handoff/` and the other kinds report that no folder can be resolved.
