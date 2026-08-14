---
name: atlassian-cli
description: Use when reading or writing Jira work items or Confluence pages from the command line with acli, especially during /playbook:learn-project when no Atlassian MCP server is connected. Covers the Confluence page-discovery workaround, since acli has no page search.
---

# Atlassian CLI (acli)

Command surface verified against **acli 1.3.22-stable**. The published command
reference at developer.atlassian.com lagged this binary (it listed no Confluence
commands at all), so trust `acli <cmd> --help` over the docs, and re-check this
skill against `--help` if a command behaves unexpectedly.

Install: `brew tap atlassian/homebrew-acli` then `brew install acli`. Homebrew
refuses untrusted third-party taps, so it also needs `brew trust atlassian/acli`.

## Is it usable right now?

```bash
command -v acli >/dev/null || echo "acli absent"
acli confluence auth status
acli jira auth status
```

Auth is per product. Being logged into Jira does not mean Confluence works. Both
support OAuth or an API token via `acli <product> auth login`.

## Top-level commands

`admin`, `auth`, `confluence`, `guard`, `jira`, `rovodev`, plus `config`,
`completion`, `feedback`, `help`.

`--json` is available on the read commands and is what you want for anything
scripted. There is no global `--output` flag; check each command's `--help`.

## Confluence

| Group | Commands |
|---|---|
| `auth` | `login`, `logout`, `status`, `switch` |
| `page` | **`view` only** |
| `blog` | `create`, `list`, `view` |
| `space` | `archive`, `create`, `list`, `restore`, `update`, `view` |

### The gotcha: there is no page search and no page list

`acli confluence page view` requires `--id`. You cannot search pages by title,
by text, or by CQL, and you cannot list the pages in a space. Note the asymmetry:
`blog list` exists, `page list` does not.

So you cannot answer "find the onboarding page" directly. You have to walk the
tree.

### Discovering pages: space to homepage to children

```bash
# 1. Find the space and its homepage page ID
acli confluence space list --json --expand homepage --limit 100

# 2. Walk down from the homepage, one level at a time
acli confluence page view --id <homepage-id> --json --include-direct-children

# 3. Recurse into whichever children look relevant
acli confluence page view --id <child-id> --json --include-direct-children
```

`--include-labels` on `page view` is the cheapest way to spot runbooks, decision
records, and onboarding pages without reading every body.

Useful `page view` flags: `--body-format storage|atlas_doc_format|view`,
`--include-labels`, `--include-direct-children`, `--include-version`,
`--status current,draft,archived`, `--version <n>` for a specific revision.

`space list` filters: `--keys`, `--type global|personal`,
`--status current|archived`, `--limit` (default 50), `--expand
description,homepage,permissions`.

## Jira

| Group | Commands |
|---|---|
| `workitem` | `search`, `view`, `create`, `create-bulk`, `edit`, `clone`, `assign`, `transition`, `archive`, `unarchive`, `delete`, `comment`, `attachment`, `link`, `watcher`, `list-watchers` |
| others | `board`, `dashboard`, `field`, `filter`, `project`, `sprint`, `auth` |

Unlike Confluence, Jira **does** have `workitem search`, so start there rather
than walking anything.

## Read-only versus write

`/playbook:learn-project` and any research flow are read-only on the remote. Safe:
`*/auth status`, `space list`, `space view`, `page view`, `blog list`,
`blog view`, `jira workitem search`, `jira workitem view`, and the `board`,
`project`, `sprint`, `filter`, `dashboard` read commands.

Never run these during research, they mutate Atlassian state: `space create`,
`space update`, `space archive`, `space restore`, `blog create`, and every
`jira workitem` verb other than `search` and `view`.

## Using it in /playbook:learn-project

The confluence collector wants pages on setup, onboarding, architecture,
runbooks, and decisions. Given no page search, the practical sequence is:

1. `acli confluence auth status`, and if it fails, mark Confluence unavailable
   and record the reason rather than retrying.
2. `acli confluence space list --json --expand homepage` and pick the space from
   the README or repo links.
3. Walk the homepage's direct children, then recurse only into branches whose
   titles or labels match what you are after.
4. Fetch bodies with `--body-format storage` only for the pages you actually
   keep. It is the expensive part.

Cap the walk. A large space is thousands of pages and there is no server-side
filter to lean on.

## Failure modes worth recognising

- **`unknown command "confluence"`**: the installed acli predates Confluence
  support. Check `acli --version` and upgrade before concluding anything else.
- **Auth OK for Jira, failing for Confluence**: they authenticate separately.
  Run `acli confluence auth login`.
- **A page ID from a browser URL**: Confluence URLs carry the ID in the path
  (`/pages/123456789/Title`); that number is what `--id` wants.
