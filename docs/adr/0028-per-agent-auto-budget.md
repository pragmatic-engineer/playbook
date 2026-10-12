# ADR-0028: Budget Auto Mode Per Agent, With the Closing Steps Allowed at a Cap

- **Status:** Accepted
- **Date created:** 2026-10-12
- **Date modified:** 2026-10-12

## Context

The `auto-cost` hook first held the whole session to one cap, `auto.budgetUsd`, summed over the main transcript and every subagent transcript. When the sum reached the cap, every caller was parked at once, including the agent that was about to commit, push and open its pull request. Work was left undelivered, and one chatty subagent stopped all the others.

Claude Code puts `agent_id` in the hook input of a tool call made inside a subagent, and leaves it out for the main session. The `transcript_path` is the main transcript in both cases. A subagent's own transcript is at `<main minus .jsonl>/subagents/agent-<id>.jsonl`. This was read from the hooks documentation and has not been checked against a live subagent call.

## Decision

1. **One cap per caller.** The main session is held to `auto.budgetUsd` on its own transcript. Each subagent is held to `auto.agentBudgetUsd` (default 5) on its own transcript. Spend is counted in integer cents from a baseline, with saturating arithmetic.
2. **A tree ceiling that parks everyone.** `auto.sessionCeilingUsd` limits the sum over the tree. Its default, 0, means ten times the main cap, so raising the main cap does not make the ceiling bind first. At the ceiling every caller is parked. A forked skill is a spawn, so denying only `Agent` calls would leave it open.
3. **Parked means no new work, and delivery stays open.** A parked caller may read, write its own park note, and run the closing steps: the two delivery skills with the skills they load, and a short list of commands (`git add`, `git commit`, `git push`, `git status`, `git diff`, `git log`, read-only `gh`, `playbook pr create|prepare|rules`, `playbook commit run --auto|prepare`, `playbook config get`, `playbook mode status`). The skills' own literal blocks are allowed: the `--help` probe and the quoted-heredoc write of `pr-body.md`.
4. **The command list is strict.** Every simple command in a Bash call must be on the list. These are denied: chained commands that are not listed, `$VAR`, `$(...)` and backticks, an unquoted heredoc whose body holds one, any redirect to a file, a word with a glued redirect, and `gh api` with a body, a non-GET method (including attached forms such as `-fkey=v` and `-XPOST`) or `graphql`. `git diff` and `git log` with `--output` are denied. `git push` must name a branch, must not force, delete, mirror, push all branches or tags, pass `--receive-pack` or `--exec`, and must not target `main`, `master`, `trunk` or `develop`. `playbook commit run` needs `--auto`, since without it the command may force push after an amend or rebase.
5. **Fail closed.** If the caller's spend cannot be read, the caller is treated as parked. An agent whose transcript does not exist yet counts as zero only while the main transcript is readable and the path can be derived.
6. **The guard check covers parked callers.** A call that changes `mode`, `auto.*` or `fix.*` or a config file is denied for a parked or unreadable caller as well as under the cap.
7. **Denials name the unblock commands.** `playbook config set` for the key that applies, and `playbook mode ask`.

## Limits that remain

- Without `agent_id` (an older Claude Code), a subagent counts as the main session. The ceiling is the backstop.
- The command check is a guardrail against drift and gives no security boundary. It does not look into `bash -c`, `eval` or variables.
- A commit message with `>` or `<` in it is denied at a cap, since a glued redirect cannot be told from text. Use `playbook commit run --auto` with a heredoc.
- The telemetry fallback covers the whole tree, so it overstates one caller's spend.

## Alternatives considered

- **Keep one session cap.** Rejected: it parks the agent that is delivering.
- **Pause the whole tree at the cap and ask.** Rejected: the hook cannot ask a question, and auto mode exists to avoid asking.
- **Hooks for SubagentStart and SubagentStop.** Rejected for now: SubagentStart cannot block, and PreToolUse already carries `agent_id`.

## Consequences

- Two config keys are added, and both are locked like the other `auto.*` keys.
- Skills that change their Bash blocks must keep the closing list in step. A test runs the literal blocks.
