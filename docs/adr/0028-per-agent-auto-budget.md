# ADR-0028: Budget Auto Mode Per Agent, With the Closing Steps Allowed at a Cap

- **Status:** Accepted, amended on 2026-10-17 (new default caps, below)
- **Date created:** 2026-10-12
- **Date modified:** 2026-10-17

## Context

The `auto-cost` hook first held the whole session to one cap, `auto.budgetUsd`, summed over the main transcript and every subagent transcript. When the sum reached the cap, every caller was parked at once, including the agent that was about to commit, push and open its pull request. Work was left undelivered, and one chatty subagent stopped all the others.

Claude Code puts `agent_id` in the hook input of a tool call made inside a subagent, and leaves it out for the main session. The `transcript_path` is the main transcript in both cases. A subagent's own transcript is at `<main minus .jsonl>/subagents/agent-<id>.jsonl`. This was read from the hooks documentation and has not been checked against a live subagent call.

## Decision

1. **One cap per caller.** The main session is held to `auto.budgetUsd` on its own transcript. Each subagent is held to `auto.agentBudgetUsd` (default 5, raised to 50 by the 2026-10-17 amendment) on its own transcript. Spend is counted in integer cents from a baseline, with saturating arithmetic.
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

## Amendment, 2026-10-17: default caps from measured spend

The default of 5 for both budgets parked normal work. A real user's sessions cost tens to hundreds of dollars a day, so a session in auto mode hit its cap early and stopped.

**What is counted.** The main session's cap is checked against its own transcript only. Two paths still count the whole tree: when the main transcript holds a model missing from the price table or has not been read, the hook falls back to the status line cost, which covers every agent; and the ceiling, which was ten times 5, so $50 for the tree.

**Measured basis.** 30 days of one heavy user's transcripts (207 sessions, $5,234 in total by `playbook usage --range 30d`), priced with playbook's own price table and the hook's own rule (the largest cost per message id):

| Measure | p50 | p90 | p95 | p99 | max |
|---|---|---|---|---|---|
| Main transcript, sessions of $1 or more (30) | $33 | $131 | $199 | $284 | $284 |
| Whole tree, sessions of $1 or more (30) | $73 | $300 | $462 | $1,358 | $1,358 |
| One subagent (3,083) | $0.42 | $1.33 | $2.32 | $10.63 | $99 |

**New defaults.**

1. `auto.budgetUsd` is 200, the p95 of the main transcript of a real session, rounded up. On the status line fallback the main cap is compared with the whole tree, and 7 of the 30 real sessions spent more than $200 over the whole tree. That path is rare, since it needs an unpriced model, and the denial names the command that raises the cap.
2. `auto.agentBudgetUsd` is 50. Only 3 of the 3,083 subagents cost more, so the cap stops a runaway agent and leaves normal ones alone.
3. `auto.sessionCeilingUsd` keeps 0, meaning ten times `auto.budgetUsd`, so $2,000. That is above the largest measured tree.
4. `playbook config set` refuses a set ceiling below `auto.budgetUsd` or `auto.agentBudgetUsd`, and a budget above a set ceiling, since such a ceiling parks every agent before its own cap. Setting a budget checks only that budget. Values are read the way the hook reads them, for the current repo and, on a global write, outside any repo too. A ceiling of 0 is not checked, and `playbook config import` does not run the check.
5. A denial says which cap was hit and gives the exact command to raise it, with double the current cap, for example `playbook config set auto.budgetUsd 400`. When a set ceiling is below that value, the command raises the ceiling first.
6. Migration `0017-auto-budget-defaults` (`Auto`, `Owned`, ADR 0034) moves a stored budget of exactly 5, in any tier, to the new default once. It runs at session start, on the first run of most commands after the upgrade (ADR 0034 decision 13), and through `playbook migrate` or `playbook doctor --fix`. When a global `auto.sessionCeilingUsd` is set below the new default, the moved budget is held at the ceiling, so the ceiling stays at or above it, and the step says so. Any other stored value stays. A store with no budget gets no write, since the key already resolves to the new default. The config store is playbook's own file, so the migration is `Owned`. A user who chose 5 on purpose before this change gets the new default too; after the migration has run, a value of 5 set again stays.

These numbers come from one user. A team with cheaper sessions can lower the caps with `playbook config set`.
