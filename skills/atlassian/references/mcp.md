# Atlassian MCP server

Atlassian offers the Rovo MCP server. Hosts expose its tools as `mcp__atlassian__*` or under the name the user gave the server. Tool names vary by server version, so discover them instead of assuming.

## Discover

Use `ToolSearch` with a query such as `atlassian jira confluence` (or `select:` for names you already know). If nothing matches, the server is not connected: fall through to acli, then twg.

## Notes

- Auth is OAuth handled by the host. Do not ask for tokens.
- The server covers Jira, Confluence and search, and is the only route that works with no local binary.
- Read tools versus write tools: judge from the tool name and description. Create, edit, transition, comment and delete tools write.
