---
name: playbook-usage
description: Use when a session doesn't have prompts/SYSTEM_PROMPT.md installed and needs to discover what /playbook:* commands exist and when to reach for one, or when asked what this plugin does, how to use it, or which command fits a task.
---

# Playbook Usage

`pragmatic-engineer/playbook` is a Claude Code plugin: 13 slash commands under
`/playbook:*` plus supporting skills and hooks. Its primary flow is a four-stage
planning pipeline, `/playbook:brainstorm` then `/playbook:scope` or
`/playbook:adr` then `/playbook:implement`, that takes a raw idea through a
verified plan to delivered code. Some pipeline commands run the moment the
intent matches, no confirmation asked; others only offer in one line and wait
for a yes before doing anything (see Trigger phrases, below).

## Command reference

### Planning pipeline

| Command | One-line purpose | When to reach for it |
|---|---|---|
| `/playbook:brainstorm` | Divergent discovery on a raw idea: challenges the premise, weighs 2-3 approaches, produces an approved PRD and design doc. | The direction is not settled yet: an idea seed, open-ended exploration, or genuine uncertainty about approach. |
| `/playbook:scope` | Interview-driven planning that turns a settled direction into a verified implementation plan, its Work Units grouped into PR-sized Segments. | The direction is settled and the work needs a concrete plan to hand to `/playbook:implement`. |
| `/playbook:adr` | Records a hard-to-reverse architectural decision as a fact-checked ADR, with an optional execution blueprint, saved to `docs/adr/`. | Choosing between named alternatives, a decision that is expensive to undo, or a request to document the reasoning behind a choice. |
| `/playbook:implement` | Executes an approved plan or ADR blueprint: delegates Work Units to subagents, commits each as a savepoint, delivers PR-sized Segments, ends with a refinement pass and adversarial review. `--boundary=land` opts in to merging each Segment autonomously before starting the next. | An approved plan or blueprint already exists and it's time to build it. Execute-only, never designs new scope. |

### Review

| Command | One-line purpose | When to reach for it |
|---|---|---|
| `/playbook:quick-review` | Single-pass PR review using grounding-review discipline and Conventional Comments; posts a pending GitHub review, or reports only when nothing was named to post to, `--self` was passed, or the resolved PR is yours. | A routine PR review, including a quick self-review of your own branch. |
| `/playbook:deep-review` | Parallel swarm of specialist reviewer subagents (logic, test, security, data, types, perf, plus conditional lenses), consolidated and fact-checked into one pending review. | A substantial, risky, or cross-cutting PR that needs more scrutiny than `/playbook:quick-review`. |
| `/playbook:address-pr-comments` | Walks unresolved PR review comments one at a time, applies fixes or drafts replies, commits and pushes, then posts replies with the new SHA. | Working through open review feedback on an existing PR. |

### Delivery

| Command | One-line purpose | When to reach for it |
|---|---|---|
| `/playbook:commit-and-push` | Stages, formats, and commits with a generated message (signed and signed off), then pushes, with optional rebase. | Delivering staged changes as a commit, instead of hand-running `git commit`/`git push`. |
| `/playbook:create-pull-request` | Runs pre-flight readiness checks and opens a PR with a conventional-commit title and the team template. | Opening a pull request, instead of hand-running `gh pr create`. |

### Utilities

| Command | One-line purpose | When to reach for it |
|---|---|---|
| `/playbook:repo-audit` | Read-only four-phase audit: discovery and mapping, evidence-based severity-rated findings, an improvement strategy, and a milestone task plan. Every claim cited to file:line. | Assessing the health of an unfamiliar or long-lived repo without changing anything. |
| `/playbook:learn-project` | Reads git history, PRs, and JIRA/Confluence, distills what it finds into memory facts (project-scoped or global), and exports a navigable `memory.graph.json`. | Building durable project knowledge the session (and future sessions) can recall. |
| `/playbook:doctor` | Checks the seven playbook layers and prints a status table with a remediation hint for each miss. | Confirming the plugin itself is wired correctly. |
| `/playbook:setup` | Interactive setup: wires safety guards, seeds or merges `settings.json`, and asks whether to install the shell launchers and system prompt. | First-time install or reconfiguring the plugin's own settings. |

## Trigger phrases for the planning pipeline

`/playbook:brainstorm` runs silently, no confirmation asked, when the
direction isn't settled: an idea seed ("I have an idea about X", "what if we
did X"), open-ended exploration ("let's brainstorm this", "what are our
options"), or genuine uncertainty ("not sure how to approach X").

`/playbook:scope` also runs silently, once the direction is settled: a
defined feature ready for a plan ("let's plan this out", "let's scope this",
"break this down"), or a continuation of an already-settled idea ("let's turn
that into a plan").

`/playbook:adr` never runs silently: it offers in one line and waits for a
yes, on a choice between named alternatives ("should we use X or Y", "X vs
Y"), a reversibility signal ("this is a big call", "let's not rush this"), or
a request to record the reasoning ("let's document why we picked X").

`/playbook:implement` also always offers and waits for a yes, and only fires
when an approved plan or ADR blueprint already exists: a green light ("let's
do this", "go ahead", "ship it"), a direct build request ("implement this",
"start building it"), or a resume ("let's pick this back up"). Without an
approved plan or blueprint, it says so and offers `/playbook:scope` (or
`/playbook:brainstorm` first, if the direction isn't settled either) instead.

## Delivery pattern

Some commands run in an isolated forked context with no bleed from the
current conversation: `/playbook:commit-and-push`,
`/playbook:create-pull-request`, and `/playbook:repo-audit` all carry
`context: fork` in their frontmatter. `/playbook:implement` goes further, delegating each plan Work
Unit to `implementer` subagents, some running in isolated git worktrees so
independent Work Units execute in parallel. A subagent's return value alone
is not a reliable signal of what it did. See `playbook:delegating-subagents`
for the full mechanics: why a return value is only a courtesy, and the
file-based handoff discipline that governs every dispatch.

## Memory

The plugin keeps a durable fact store at `~/.config/playbook/memory/`, in
three scopes: global facts sit flat alongside its `MEMORY.md` index,
org-scoped facts live under `~/.config/playbook/memory/<owner>/`, and
project-scoped facts live under `~/.config/playbook/memory/<owner>/<repo>/`.
`/playbook:adr` and `/playbook:learn-project` both read from and write to
this store. See
`docs/guides/03-decisions-and-memory.md` for the full frontmatter schema, the
typed-edges table (`supersedes`, `depends_on`, `relates_to`, `contradicts`),
and the rule for where a given fact belongs.
