# Confluence with acli

Read this when you need to find or read Confluence pages. Verified against acli 1.3.22-stable.

## Commands

| Group | Commands |
|---|---|
| `auth` | `login`, `logout`, `status`, `switch` |
| `page` | **`view` only** |
| `blog` | `create`, `list`, `view` |
| `space` | `archive`, `create`, `list`, `restore`, `update`, `view` |

## Discovering pages: space to homepage to children

```bash
# 1. Find the space and its homepage page ID
acli confluence space list --json --expand homepage --limit 100

# 2. Walk down from the homepage, one level at a time
acli confluence page view --id <homepage-id> --json --include-direct-children

# 3. Recurse into whichever children look relevant
acli confluence page view --id <child-id> --json --include-direct-children
```

`--include-labels` on `page view` is the cheapest way to spot runbooks, decision records and onboarding pages without reading every body.

Useful `page view` flags: `--body-format storage|atlas_doc_format|view`, `--include-labels`, `--include-direct-children`, `--include-version`, `--status current,draft,archived`, `--version <n>` for a specific revision.

`space list` filters: `--keys`, `--type global|personal`, `--status current|archived`, `--limit` (default 50), `--expand description,homepage,permissions`.

A page ID from a browser URL sits in the path (`/pages/123456789/Title`); that number is what `--id` wants.

## Using it in /playbook:learn-project

The confluence collector wants pages on setup, onboarding, architecture, runbooks and decisions. With no page search, the sequence is:

1. `acli confluence auth status`. If it fails, mark Confluence unavailable and record the reason rather than retrying.
2. `acli confluence space list --json --expand homepage` and pick the space from the README or repo links.
3. Walk the homepage's direct children, then recurse only into branches whose titles or labels match.
4. Fetch bodies with `--body-format storage` only for pages you keep. It is the expensive part.

Cap the walk. A large space is thousands of pages and there is no server-side filter.
