---
name: playbook-usage
description: Lists the /playbook:* commands and when to use each. Use when asked what the playbook plugin does, what commands exist, or which command fits a task.
---

# Playbook Usage

`pragmatic-engineer/playbook` is a Claude Code plugin: slash commands under `/playbook:*` plus supporting skills and hooks. The main flow is a planning pipeline: `/playbook:plan` (or `/playbook:adr` for a hard-to-reverse decision), then `/playbook:implement`, which takes a raw idea through a verified plan to delivered code. `/playbook:plan` runs the moment the intent matches. `/playbook:adr` and `/playbook:implement` only offer in one line and wait for a yes. Each command's own description lists its trigger phrases.

## Planning pipeline

| Command | Use when |
|---|---|
| `/playbook:plan` | An idea needs to become a plan, raw or settled. One interview-driven session that challenges the premise, weighs 2-3 approaches, then produces a verified plan of Work Units grouped into PR-sized Segments. |
| `/playbook:adr` | Choosing between named alternatives, a decision that is expensive to undo, or recording why a choice was made. Saves a fact-checked ADR with an optional blueprint to `docs/adr/`. |
| `/playbook:implement` | An approved plan or blueprint exists. Delegates Work Units to subagents, commits each, delivers PR-sized Segments, ends with a refinement pass and an adversarial review. `--boundary=land` merges each Segment before starting the next. Never designs new scope. |

## Review

| Command | Use when |
|---|---|
| `/playbook:quick-review` | A routine PR review, or a quick check of your own branch. Single pass, posts a pending GitHub review, or only reports with `--self` or when the PR is yours. |
| `/playbook:deep-review` | A substantial, risky or cross-cutting PR. A parallel swarm of specialist reviewers, fact-checked into one pending review. |
| `/playbook:address-pr-comments` | Working through open review feedback on a PR, one comment at a time. |

## Delivery

| Command | Use when |
|---|---|
| `/playbook:commit-and-push` | Committing staged changes with a generated, signed message and pushing, instead of hand-running `git commit` and `git push`. |
| `/playbook:create-pull-request` | Opening a PR with pre-flight checks, a conventional-commit title and the team template, instead of hand-running `gh pr create`. Then run the `playbook:finish-pull-request` skill. |
| `/playbook:fix` | A small, well understood bug. A failing test, the smallest fix, one PR. Escalates to `/playbook:plan` when the fix is not small. |

## Utilities

| Command | Use when |
|---|---|
| `/playbook:repo-audit` | Assessing an unfamiliar or long-lived repo without changing it. Read-only, every claim cited to `file:line`. |
| `/playbook:learn-project` | Building durable project knowledge in the memory store from git history, PRs, Jira and Confluence. |
| `/playbook:doctor` | Checking the seven playbook layers after install or update, or when hooks seem not to run. |
| `/playbook:session-start` | After `/clear`, when the saved handoff did not load. The `playbook:session-handoff` skill writes the handoff. |
| `/playbook:setup` | First install, or repairing the plugin's own settings. |

## Delivery pattern

`/playbook:commit-and-push`, `/playbook:create-pull-request` and `/playbook:repo-audit` run in an isolated forked context (`context: fork`). `/playbook:implement` delegates each Work Unit to `implementer` subagents, some in isolated git worktrees. A subagent's return value alone is not a reliable signal of what it did. See `playbook:delegating-subagents` for the file-based handoff discipline.

The memory store, its scopes and its edge types are described in `docs/guides/03-decisions-and-memory.md`.
