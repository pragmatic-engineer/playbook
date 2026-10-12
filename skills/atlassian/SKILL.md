---
name: atlassian
description: Use when reaching Jira work items, Confluence pages or Teamwork Graph data, choosing between the Atlassian MCP server, acli and twg, or when /playbook:learn-project or a ticket-driven plan needs Atlassian access.
---

# Atlassian access

Three routes reach Atlassian. Use the first route that works, in the order below. This section is the only place the order is defined; other skills and commands point here. Stay read-only unless the task is a write.

## Selection order

1. **MCP server.** Run `ToolSearch` for the Atlassian tools. If they are connected, use them. Details in [references/mcp.md](references/mcp.md).
2. **acli.** Else if `command -v acli` succeeds and the product's auth status passes, use it. Best at Jira work items. Commands in [references/acli-jira.md](references/acli-jira.md) and [references/acli-confluence.md](references/acli-confluence.md).
3. **twg.** Else if `command -v twg` succeeds and `twg doctor` passes, use it. Best at Confluence search and cross-product Teamwork Graph queries, which acli cannot do. Commands in [references/twg.md](references/twg.md).
4. Else mark Atlassian unavailable and say so. Never guess at contents.

Strengths, as notes on the order above: acli is strongest on Jira work items; it has no Confluence page search, so page discovery by walking the tree is slow and twg search is the useful fallback there. twg alone offers cross-product Teamwork Graph queries.

## Auth checks

```bash
command -v acli >/dev/null && { acli jira auth status; acli confluence auth status; }
command -v twg >/dev/null && twg doctor
```

- acli: Jira and Confluence authenticate separately. Jira working says nothing about Confluence. Fix with `acli <product> auth login`. Treat each probe independently; one product reachable and the other not is a normal state.
- twg: OAuth 2.1 through `twg setup` (the installer runs it). `twg doctor` checks auth. Bitbucket needs its own token.
- MCP: OAuth handled by the host.

## acli quirks

- **No Confluence page search and no page list.** `page view` needs `--id`. Find pages by walking from the space homepage; the recipe is in [references/acli-confluence.md](references/acli-confluence.md).
- `unknown command "confluence"` means acli predates Confluence support. Check `acli --version` and upgrade.
- The published acli docs lagged the binary. Trust `acli <cmd> --help`.
- Install: `brew tap atlassian/homebrew-acli`, `brew trust atlassian/acli`, then `brew install acli`.

## Read-only versus write

`/playbook:learn-project` and any research flow are read-only on the remote. Read verbs: auth and status checks, searches, views, gets and lists. Never run creates, edits, transitions, comments, archives or deletes during research, whichever route you use. When a task does require a write, follow `playbook:writing-style`, and write in first person as the signed-in account holder ("I need clarification on X"), since the post goes out under that identity.
