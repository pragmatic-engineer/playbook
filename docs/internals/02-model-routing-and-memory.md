# Internals: Model Routing and Memory

The session model defaults to Sonnet. Two systems shape which model handles a given task: a static policy in the system prompt, and a hook that detects design intent at prompt submission. Memory is a typed graph stored in plain markdown files.

## Model Routing

The policy lives in `prompts/SYSTEM_PROMPT.md`. Three tiers:

**Sonnet** is the session default. The `cc` launcher sets it at session start and it covers most coding work.

**Haiku** is the default for spawned subagents on mechanical, formatting, or search tasks. It's 3x cheaper. Escalate to Sonnet when the subagent does real coding, and to Opus when it needs architecture.

**Opus** handles deep architectural planning only, and only when Sonnet wasn't enough. Keep Opus under 20% of total usage.

### Plan-mode routing

The system prompt directs: enter plan mode first for design work (new features, non-trivial refactors, architecture decisions). `settings.json` sets `"useAutoModeDuringPlan": true`, which puts auto mode in effect during plan mode. The system prompt states this combination routes plan mode to Opus and execution to Sonnet.

### auto-model-detect

The `auto-model-detect` hook (invoked as `playbook hook auto-model-detect`, `src/hooks/auto_model_detect.rs`) runs on every `UserPromptSubmit` event, wired in `settings.json` under the `UserPromptSubmit` hook list. It can't flip the session model mid-stream (Claude Code doesn't support that). Instead it detects design intent and injects a context message nudging Claude toward an Opus subagent.

The hook skips slash commands and prompts under 20 characters. For natural-prose prompts it matches a hand-expanded set of phrases (case-insensitive, word-boundary matched, ported from the original regex) covering:

- Design nouns: `design`, `architecture`, `ADR`, `schema`, `tradeoffs`, `migration`, `data model`, `interface design`, and related terms.
- Decision verbs: `evaluate`, `compare`, `brainstorm`, `propose`, `critique`, `review the approach`.
- Design-shaped questions: `should we`, `how would/should we/you/I`, `what's the best`, `which approach/design/pattern`, `pros and cons`.

On a match, the hook emits a prompt context message reminding Claude the session runs on Sonnet and recommending one of two delegation paths: the `Plan` agent (via `Agent` tool with `model: "opus"`) for implementation planning with codebase grounding, or `/playbook:plan` for design work before any code. For narrow prompts (a quick choice between two named options) staying inline on Sonnet is fine. No match: the hook exits silently.

### Effort policy

Effort is a second dial next to the model tier. Agent files, commands, and skills all accept an `effort` key (`low`, `medium`, `high`, `xhigh`, `max`). The `Agent` tool has no per-call effort override, only `model`, so an agent's effort is fixed by its file. Where one role needs two efforts, `playbook agents gen` writes tier variants (see [Authoring agents](../authoring/02-authoring-agents.md)) and the orchestrator picks by name.

The rule: lower effort where the work is mechanical or already decided, and never where a missed finding is costly. A model that is wrong costs a rerun; a reviewer that stops looking costs a bug in production.

| File | Effort | Reasoning |
| --- | --- | --- |
| `agents/git` | `low` (was medium) | Runs fixed git and `gh` steps from a command that already decided what to do. |
| `agents/patch-applier` | `low` (was medium) | Applies a diff someone else approved, verbatim, with no judgment. |
| `agents/collector` | `low` (was medium) | Gathers and compacts raw history; the analyst does the thinking later. |
| `agents/cheap-checker` | `low` (was medium) | One narrow concern from a named reference file. A full lens covers the rest. |
| `agents/review-triage` | `low` (was medium) | A three-way classifier; any bad or missing answer already falls back to `full-lens`, so a wrong call fails safe. |
| `commands/quick-review` | `medium` (was high) | One pass over a diff the user chose not to deep review; `/playbook:deep-review` is the thorough path. |
| `agents/reviewer`, `critic`, `fact-checker`, `test-reviewer` | `high` (kept) | A missed finding is the cost. This includes the security lens, so nothing here is lowered; the `-low` variants exist only for small diffs and are an orchestrator choice. |
| `agents/implementer`, `analyst`, `auditor` | `high` (kept) | They write code or distill facts that later steps trust. |
| `commands/deep-review`, `implement`, `plan`, `adr`, `learn-project`, `fix`, `address-pr-comments` | `high` (kept) | Judgment and orchestration; mistakes propagate to every spawned agent. |
| `commands/commit-and-push`, `create-pull-request`, `repo-audit` | none | They fork into `git` (`low`) and `auditor` (`high`), which set the effort. |
| `commands/doctor`, `session-start`, `setup` | `low` (kept) | Run a script and print the result. |
| `skills/*` | none | A skill is knowledge loaded into whoever uses it. An `effort` key would override the caller's choice, so none sets one. |

No model changed. Every row already matches the three tiers above.

## Memory Protocol

Memory is a typed graph. Both scopes share the same file format.

### Scopes

| Scope | Path | Coverage | Committed to git? |
|---|---|---|---|
| Global | `~/.config/playbook/memory/` | Cross-project | No, outside any repo |
| Org | `~/.config/playbook/memory/<owner>/` | Every repo under one owner | No, outside any repo |
| Project | `~/.config/playbook/memory/<owner>/<repo>/` | One repo only | No, outside any repo |

The `<owner>/<repo>` project index is injected at session start. The global index is read on demand.

### File format

One fact per file. Filenames are kebab-case (e.g. `commits-must-be-signed.md`). Every file opens with YAML frontmatter:

```yaml
---
name: fact-name
description: one-line trigger hint for when to use this fact
type: user | feedback | project | reference
links:
  supersedes: old-fact-name
  depends_on: prerequisite-name
  relates_to: neighbor-name
  contradicts: conflicting-name
anchors:           # optional; mainly used in the project store
  - src/auth/login.py#authenticate
  - src/auth/
---
```

Edge values are bare basenames with no path and no extension. For `feedback` and `project` type facts, the body follows a fixed structure: the rule first, then a **Why:** section and a **How to apply:** section.

### Index

`memory.graph.json` is the index. Each fact becomes one node:

```json
{
  "id": "acme/api/auth-flow",
  "file": "acme/api/auth-flow.md",
  "scope": "project",
  "type": "project",
  "name": "Auth Flow",
  "description": "How tokens are issued and validated in this service"
}
```

Nodes carry names and descriptions for navigation; edge declarations live only in each fact's frontmatter, not duplicated on the node.

### Edge types

| Edge | Direction | Meaning |
|---|---|---|
| `supersedes` | new → old | The authoring fact replaces the target. Act on the chain head; treat superseded facts as historical. |
| `depends_on` | authoring → prerequisite | Load the prerequisite before acting on this fact. |
| `relates_to` | symmetric | Pull the neighbor for related context. |
| `contradicts` | symmetric | Both facts are live but conflict. Surface the conflict; don't silently choose one. |

Each edge is stored once on the authoring node. Reverse links are inferred by scanning frontmatter at load time, not stored explicitly.

Traversal depth is 1 for all edge types except `supersedes`, which is followed fully (chain head wins). A project fact that contradicts a global fact wins for that repo. Dangling basenames (the target isn't in the store) are surfaced, not dropped.

### memory.graph.json

`~/.config/playbook/memory/memory.graph.json` is the single navigable export of the full memory graph: nodes are facts and referenced code locations, edges are `links:` between facts plus `anchors:` from facts to code. It covers all scopes: global, org, and project. The `rebuild-memory-graph` PostToolUse hook rebuilds it automatically after any fact file is saved. See [Decisions and Memory](../guides/03-decisions-and-memory.md) for how to query and use it day-to-day.

## See also

- [Decisions and Memory](../guides/03-decisions-and-memory.md): using memory day-to-day and the `/playbook:learn-project` command.
- [Internals: Launcher and Hooks](01-launcher-and-hooks.md): the `cc` launcher that sets the session model.
- [Docs index](../index.md)
