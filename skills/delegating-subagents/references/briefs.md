# Briefs and forks

Why `SKILL.md` says to pass a `subagent_type` and write a stand-alone brief, and how each place playbook spawns a subagent is classified.

## Contents

- Fork cost
- Classification of spawn sites
- Brief template
- Checks before dispatch

## Fork cost

An `Agent` call that omits `subagent_type` forks the parent: the child inherits the whole conversation. Over 30 days on one machine, 117 such forks averaged 372K tokens of first-turn context and cost $2.90 each, about $339 in total. The same work on a fresh agent with a brief starts at a few thousand tokens. Forks were not stalling or returning less; the cost is the inherited history, nothing else.

`context: fork` in a command's frontmatter is a different mechanism. It runs the command body in the named agent without the parent history, so it is already a fresh start and stays.

## Classification of spawn sites

| Site | Needs parent history? | Mechanism |
|---|---|---|
| `commit-and-push`, `create-pull-request` | No, the staged diff is read from git | `context: fork`, `agent: playbook:git` (fresh) |
| `repo-audit` | No | `context: fork`, `agent: auditor` (fresh) |
| `deep-review`, `quick-review`, `implement` Step 9 reviewers, `review-triage`, `cheap-checker` | No, diff and excerpts go in the brief | `subagent_type: playbook:<agent>` |
| `implement` implementers | No, the Work Unit brief is self-contained | `subagent_type: playbook:implementer` |
| `plan`, `adr` critic, fact-checker, test-reviewer | No, they get the full artifact | `subagent_type: playbook:<agent>` |
| `plan` Step 2 and discovery agents | No, they get a question and paths | `subagent_type: Explore` or `general-purpose` |
| `learn-project` collectors and analysts | No | `subagent_type: playbook:<agent>` |
| Ad hoc fan-out from the system prompt | No by default | an explicit `subagent_type`, never omitted |

Nothing in this table needs history. A fork is justified only when a child must act on corrections the user gave earlier in the session and the corrections cannot be stated in the brief in a few lines.

## Brief template

```
Goal: <one sentence>
Inputs: <absolute paths with line ranges>
Output: write your full report to <absolute path>
Constraints: <scope, do not touch, read-only or not>
```

Agents that cannot write (the read-only roster rows) have no `Output` line; they return their findings instead, and the orchestrator treats silence as NOT RUN.

## Checks before dispatch

- Is `subagent_type` set, with the `playbook:` prefix for plugin agents?
- Could the child do this task if it saw only the brief? If not, add what it lacks, by path first.
- Is the output file named, for an agent that can write?
