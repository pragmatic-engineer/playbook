# twg (Atlassian Teamwork Graph CLI)

Sources: [overview](https://developer.atlassian.com/cloud/twg-cli/), [install and setup](https://developer.atlassian.com/cloud/twg-cli/getting-started/installation/), [FAQ](https://developer.atlassian.com/cloud/twg-cli/faq/) and the [command catalog](https://developer.atlassian.com/cloud/twg-cli/commands/commands-catalog/). `twg` was not installed where this was written, so no command was run locally. Names below come from those pages; run `twg --help`, `twg jira --help` and `twg confluence --help` to confirm on your machine. Anything the pages do not state is tagged [unverified].

`twg` is a command line interface to the Atlassian Teamwork Graph and cloud services. It is agent-first and covers Jira, Confluence, Bitbucket, Jira Service Management, Assets, Atlas goals and projects, teams and more, with cross-product graph queries.

## Install and auth

- Install (macOS, Linux): `curl -fsSL --retry 2 https://teamwork-graph.atlassian.com/cli/install | bash`. It signs in through the browser with OAuth and runs `twg doctor`. Repair later with `twg setup`.
- Auth is OAuth 2.1 only. Bitbucket is the exception and needs a separate token (`twg setup bitbucket`).
- Check: `twg doctor` verifies auth, connectivity and build info. `twg whoami` shows the signed-in user.
- Permissions match the signed-in user's own Atlassian permissions.
- Discover syntax: `twg --help`, `twg help describe <command>`.

## Cost

Commands labeled `[Enriched]` in `twg help <command>` spend Rovo credits (for example `twg context ...`, `twg docs ...`, `twg collaborators`, `twg jira workitem similar`). Connecting, metadata and writes are free; basic single-product reads are included with the plan. Prefer plain reads unless you need graph context.

## Commands by job (from the catalog)

| Job | Command |
|---|---|
| Jira work item by key | `twg jira workitem get` |
| Jira fuzzy text search | `twg jira workitem search` |
| Jira structured JQL | `twg jira workitem query` |
| Confluence content by reference | `twg confluence content get` |
| Confluence list in a space | `twg confluence content list` |
| Confluence search, natural text | `twg confluence search text` |
| Confluence search, CQL | `twg confluence search query` |
| Confluence spaces | `twg confluence space list`, `twg confluence space get` |
| Cross-product context (Enriched) | `twg context get`, `twg context jira workitem`, `twg context confluence page` |

Writes exist (for example `twg confluence content create`, `twg confluence content comments create`, `twg confluence space delete`). Treat any command outside the table above as a write until its help says otherwise.
