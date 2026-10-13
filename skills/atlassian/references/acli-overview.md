# acli top-level surface

Verified with `acli --help` and the group helps on 1.3.39-stable.

| Command | What it is | Playbook scope |
|---|---|---|
| `jira` | Jira Cloud commands, see acli-jira.md | In scope |
| `confluence` | Confluence Cloud commands (`auth`, `blog`, `page`, `space`), see acli-confluence.md | In scope |
| `auth` | OAuth to multiple Atlassian accounts: `login`, `logout`, `status`, `switch` | In scope |
| `admin` | Organization admin: `auth`, and `user` with `activate`, `cancel-delete`, `deactivate`, `delete` (all write) | In scope only when the user asks; it manages users |
| `guard` | Atlassian Guard CLI. On this machine it answers `Plugin guard not found`, so it is not installed here | In scope only when the user asks and the plugin exists |
| `rovodev` | Rovo Dev, Atlassian's AI coding agent (Beta). Needs its own scoped API token and `acli rovodev auth login` | Out of scope |
| `config` | Configuration; only `gov-cloud` was seen | Out of scope |
| `feedback` | Submits a request or problem report to Atlassian | Out of scope |
| `help` | Help about any command | Use freely |

## Authentication

- `acli auth login` authenticates Jira and Confluence sites with OAuth and supports several accounts. `acli auth status` shows OAuth status and `acli auth switch` changes the active account. `acli auth logout` logs out of all OAuth accounts.
- Each product also has its own `acli jira auth` and `acli confluence auth`, with `login` (OAuth or API token), `logout`, `status` and `switch`. Jira and Confluence authenticate separately, so a passing Jira status says nothing about Confluence.
- `acli admin auth` is a third, separate login for organization admin tasks.
