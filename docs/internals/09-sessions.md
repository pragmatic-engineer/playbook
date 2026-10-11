# Sessions

`playbook sessions` lists your recent agent sessions in one table, so you can see which ones still matter. It is read only. The same records show in the usage terminal view (key `p`). Design: [ADR-0024](../adr/0024-session-management-view-and-lifecycle.md), stage 1.

```
playbook sessions
playbook sessions --json
```

## Columns

| Column | Where it comes from |
|---|---|
| State | `working` (a message in the last 2 minutes), `idle`, `stale` (no activity for `sessions.staleAfterDays`), `done` (the pull request of the branch is merged or closed). |
| Session | The first 8 characters of the session id. |
| Repo, Branch | The newest message of the session in the usage store. |
| Worktree | Whether the directory the session started in is a linked git worktree, with the playbook convention that made it (`ccc`, `wu`, `agent`, `review`), `main checkout`, or `gone` when the folder no longer exists. |
| Link | The PR of the branch from `gh pr list --head`, or `issue #N?` when only the branch name has a number. The question mark means the issue is a guess. A session on `main` has no PR link. |
| Last | Time since the last message. |
| Context | Input plus cache read plus cache write tokens of the newest request, the size of the prompt the model last read. |
| Cost | The whole session's cost, as in `playbook usage`. |

It reads the same store as `playbook usage` (and ingests new history first), the first transcript lines of each session for its working directory, `git` for the worktree and `git status`, and `gh` for the PR. `gh` and `git` calls have timeouts. A missing `gh` or no network leaves the link empty.

## The sessions view

In `playbook usage`, press `p` for a full screen table of the same records, in the usage theme and in UTC. `j` and `k` move, `v` changes the order (newest activity, grouped by repo, or by state with the ones that need attention first), `r` reads again now, `p` or `Esc` goes back and `q` quits. It reads again every 30 seconds, off the drawing loop, so a slow `gh` never freezes the screen. The selected session's suggestion shows under the table. The view has no key that closes, removes or stops anything.

## What it never does

- It writes nothing, removes nothing and stops nothing.
- A finished or stale session gets a printed suggestion. When its worktree has uncommitted changes, the suggestion starts with `!` and says nothing is removed.
- It stores and prints no prompt text. The `--json` output holds ids, names, paths, counts and times (epoch seconds, UTC).
- `waiting` is not shown yet. Claude Code only reports it through the `Notification` hook, which playbook does not use yet (ADR-0024, stage 3).

## Limits

- The working directory is the first one the transcript records. A session that moves into a worktree later shows where it started.
- The state `working` is a guess from the last message time. A session that is thinking and has not written a message looks idle.
- Codex sessions show repo, branch, context and cost, but no directory, so no worktree or PR.

Keys: `sessions.staleAfterDays` and `sessions.recentDays` in [config keys](../guides/04-config-keys.md).
