---
name: atlassian-cli
description: Drives the acli CLI to read and write Jira work items and Confluence pages. Use when Jira or Confluence access is needed from the shell, such as /playbook:learn-project with no Atlassian MCP server connected.
---

# Atlassian CLI (acli)

Command surface verified against **acli 1.3.22-stable**. The published reference at developer.atlassian.com lagged this binary (it listed no Confluence commands), so trust `acli <cmd> --help` over the docs, and re-check against `--help` if a command misbehaves.

Install: `brew tap atlassian/homebrew-acli`, `brew trust atlassian/acli` (Homebrew refuses untrusted third-party taps), then `brew install acli`.

## Is it usable right now?

```bash
command -v acli >/dev/null || echo "acli absent"
acli confluence auth status
acli jira auth status
```

Auth is per product: being logged into Jira does not mean Confluence works. Both support OAuth or an API token via `acli <product> auth login`.

Top-level commands: `admin`, `auth`, `confluence`, `guard`, `jira`, `rovodev`, plus `config`, `completion`, `feedback`, `help`. `--json` is on the read commands and is what you want for scripting. There is no global `--output` flag; check each command's `--help`.

## Confluence

**Gotcha: there is no page search and no page list.** `acli confluence page view` requires `--id`. You cannot search by title, text or CQL, or list a space's pages (`blog list` exists, `page list` does not). To find a page you walk the tree from the space homepage.

Read [confluence.md](confluence.md) for the command table, the walk recipe, `page view` and `space list` flags, and the /playbook:learn-project sequence.

## Jira

| Group | Commands |
|---|---|
| `workitem` | `search`, `view`, `create`, `create-bulk`, `edit`, `clone`, `assign`, `transition`, `archive`, `unarchive`, `delete`, `comment`, `attachment`, `link`, `watcher`, `list-watchers` |
| others | `board`, `dashboard`, `field`, `filter`, `project`, `sprint`, `auth` |

Unlike Confluence, Jira **has** `workitem search`, so start there rather than walking anything.

### Posting a comment or description

`workitem comment`, and any `edit` or `create` that writes a description, posts as the identity `acli` is authenticated as, typically the user's own account. Load `playbook:writing-style` and write in first person as that account holder: "I need clarification on X", never "the user needs clarification on X" or the user's name in third person.

## Read-only versus write

`/playbook:learn-project` and any research flow are read-only on the remote. Safe: `*/auth status`, `space list`, `space view`, `page view`, `blog list`, `blog view`, `jira workitem search`, `jira workitem view`, and the `board`, `project`, `sprint`, `filter`, `dashboard` read commands.

Never run these during research, they mutate Atlassian state: `space create`, `space update`, `space archive`, `space restore`, `blog create`, and every `jira workitem` verb other than `search` and `view`.

## Failure modes

- **`unknown command "confluence"`**: the installed acli predates Confluence support. Check `acli --version` and upgrade first.
- **Auth OK for Jira, failing for Confluence**: they authenticate separately. Run `acli confluence auth login`.
