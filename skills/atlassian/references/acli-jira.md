# Jira with acli

Verified against acli 1.3.22-stable. Trust `acli jira --help` over this table if they differ.

| Group | Commands |
|---|---|
| `workitem` | `search`, `view`, `create`, `create-bulk`, `edit`, `clone`, `assign`, `transition`, `archive`, `unarchive`, `delete`, `comment`, `attachment`, `link`, `watcher`, `list-watchers` |
| others | `board`, `dashboard`, `field`, `filter`, `project`, `sprint`, `auth` |

Unlike Confluence, Jira has `workitem search`, so start there rather than walking anything. Add `--json` to read commands for scripting.

Read-only: `workitem search`, `workitem view`, and the read commands under `board`, `project`, `sprint`, `filter`, `dashboard`. Every other `workitem` verb writes.
