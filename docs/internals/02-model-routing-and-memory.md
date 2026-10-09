# Internals: Model Routing and Memory

The session model defaults to Sonnet. Two systems shape which model handles a given task: a static policy in the system prompt, and a hook that detects design intent at prompt submission. Memory is a typed graph stored in plain markdown files.

## Model Routing

The policy lives in `prompts/SYSTEM_PROMPT.md`. Three tiers:

**Sonnet** is the session default. The `ccc` launcher sets it at session start and it covers most coding work.

**Haiku** is the default for spawned subagents on mechanical, formatting, or search tasks. It's 3x cheaper. Escalate to Sonnet when the subagent does real coding, and to Opus when it needs architecture.

**Opus** handles design work (`/playbook:plan`, `/playbook:adr`), every reviewer spawn (`/playbook:quick-review`, `/playbook:deep-review`, `/playbook:implement` Step 9), the auditor behind `/playbook:repo-audit`, `/playbook:address-pr-comments` and `/playbook:learn-project`. Sonnet stays on `/playbook:implement`, `/playbook:fix`, the implementer, critic, fact-checker and test-reviewer.

### Design work

Design work uses `/playbook:plan` and `/playbook:adr`, not the built-in plan mode. Each runs on Opus at high effort by its own frontmatter.

### auto-model-detect

The `auto-model-detect` hook (invoked as `playbook hook auto-model-detect`, `src/hooks/auto_model_detect.rs`) runs on every `UserPromptSubmit` event, wired in `settings.json` under the `UserPromptSubmit` hook list. It can't flip the session model mid-stream (Claude Code doesn't support that). Instead it detects design intent and injects a context message nudging Claude toward an Opus subagent.

The hook skips slash commands and prompts under 20 characters. For natural-prose prompts it matches a hand-expanded set of phrases (case-insensitive, word-boundary matched, ported from the original regex) covering:

- Design nouns: `design`, `architecture`, `ADR`, `schema`, `tradeoffs`, `migration`, `data model`, `interface design`, and related terms.
- Decision verbs: `evaluate`, `compare`, `brainstorm`, `propose`, `critique`, `review the approach`.
- Design-shaped questions: `should we`, `how would/should we/you/I`, `what's the best`, `which approach/design/pattern`, `pros and cons`.

On a match, the hook emits a prompt context message reminding Claude the session runs on Sonnet and recommending one of two delegation paths: the `Plan` agent (via `Agent` tool with `model: "opus"`) for implementation planning with codebase grounding, or `/playbook:plan` for design work before any code. For narrow prompts (a quick choice between two named options) staying inline on Sonnet is fine. No match: the hook exits silently.

### Effort policy

The reasoning behind these choices, and how skills, agents and variants fit together, is in [Why the pieces are shaped this way](../concepts/03-why-the-pieces-are-shaped-this-way.md).

Effort is a second dial next to the model tier. Agent files, commands, and skills all accept an `effort` key (`low`, `medium`, `high`, `xhigh`, `max`). The `Agent` tool has no per-call effort override, only `model`, so an agent's effort is fixed by its file. Where one role needs two efforts, the launcher renders tier variants for the session (see [Authoring agents](../authoring/02-authoring-agents.md)) and the orchestrator picks by name.

The rule: lower effort where the work is mechanical or already decided, and never where a missed finding is costly. A model that is wrong costs a rerun; a reviewer that stops looking costs a bug in production.

| File | Model | Effort | Reasoning |
| --- | --- | --- | --- |
| `agents/git` | haiku | `low` (was medium) | Runs fixed git and `gh` steps from a command that already decided what to do. |
| `agents/patch-applier` | haiku | `low` (was medium) | Applies a diff someone else approved, verbatim, with no judgment. |
| `agents/collector` | haiku | `low` (was medium) | Gathers and compacts raw history; the analyst does the thinking later. |
| `agents/cheap-checker` | haiku | `low` (was medium) | One narrow concern from a named reference file. A full lens covers the rest. |
| `agents/review-triage` | haiku | `low` (was medium) | A three-way classifier; any bad or missing answer already falls back to `full-lens`, so a wrong call fails safe. |
| `commands/quick-review` | sonnet (orchestrator) | `medium` (was high) | One pass over a diff the user chose not to deep review; `/playbook:deep-review` is the thorough path. |
| `agents/reviewer`, `critic`, `fact-checker`, `test-reviewer` | reviewer opus; critic, fact-checker, test-reviewer sonnet | `high` (kept) | A missed finding is the cost. This includes the security lens, so nothing here is lowered; the `-low` variants exist only for small diffs and are an orchestrator choice. |
| `agents/implementer`, `analyst`, `auditor` | implementer, analyst sonnet; auditor opus | `high` (kept) | They write code or distill facts that later steps trust. |
| `commands/deep-review`, `implement`, `plan`, `adr`, `learn-project`, `fix`, `address-pr-comments` | opus, except implement and fix (sonnet) | `high` (kept) | Judgment and orchestration; mistakes propagate to every spawned agent. |
| `commands/commit-and-push`, `create-pull-request`, `repo-audit` | fork into git (haiku) and auditor (opus) | none | They fork into `git` (`low`) and `auditor` (`high`), which set the effort. |
| `commands/doctor`, `session-start`, `setup` | sonnet | `low` (kept) | Run a script and print the result. |
| per-session variants such as `reviewer-low` | same as base | any tier | Not files. Rendered from the base agent by `ccc` and `ccd` and passed with `--agents`. The orchestrator picks by ceiling, then diff size and risk; see `playbook:delegating-subagents`. |
| `skills/*` | none | none | A skill is knowledge loaded into whoever uses it. An `effort` key would override the caller's choice, so none sets one. |

Effort is fixed per agent definition: base agents run at their own effort, and an orchestrator picks a variant by ceiling, diff size and risk. Do not request xhigh or max on your own for anything else; if a deployment rejects it, use the base agent.

### Per-component ceilings

A ceiling for one component is the key `effort.<agents|commands|skills>.<name>` (see [ADR-0017](../adr/0017-per-component-effort-ceilings.md)). `playbook effort list` prints every component with its shipped, configured and effective effort. `playbook effort resolve agents <name> --json` returns, for an agent, the file to dispatch under the ceilings and every file allowed at or below them. The `Agent` tool has no per-call effort, so choosing the file is how an agent's ceiling is enforced.

### Model tiers and the 5.5 rule

Every model playbook picks is the 5.5 generation. Plugin files name a tier by alias (`haiku`, `sonnet`, `opus`) and never pin a model id. Claude Code resolves each alias to the newest model the provider offers (on the Anthropic API that is the 5.5 model).

| Tier | Alias | Preferred (5.5) | Falls back to |
|---|---|---|---|
| fast | `haiku` | `claude-haiku-5-5` | `claude-haiku-4-5` |
| balanced | `sonnet` | `claude-sonnet-5-5` | `claude-sonnet-5` |
| deep | `opus` | `claude-opus-5-5` | `claude-opus-5` |

The table lives in `src/models/mod.rs`. When the preferred model is overloaded or unavailable (including a Bedrock or Vertex account that cannot invoke it), Claude Code switches to a fallback for that turn. `ccc` and `ccd` pass the chain `claude-opus-5,claude-sonnet-5,claude-haiku-4-5` with `--fallback-model`. They add nothing when you pass `--fallback-model` yourself or set `fallbackModel` in `~/.claude/settings.json`. A session started without the launcher gets no playbook fallback.

Measured on Claude Code 2.1.293 (2026-10-09): all six ids answer, and the aliases resolve to the 5.5 ids. With an unknown primary id and `--fallback-model`, Claude Code runs the fallback. Without it the call fails with `api_error`. Claude Code accepts a chain of more than three models without a flag error, so the three-model cap is only a documented limit.

**Effort and the fallback.** The previous Sonnet and Opus reject `xhigh` and `max` with HTTP 400 `Invalid effort level`. They accept `low`, `medium` and `high`. Haiku 4.5 accepts all four. A 400 also triggers the fallback, so a session at `xhigh` would walk the chain and end at Haiku anyway. When the effort in force is `xhigh` or `max` (from `--effort`, or `effortLevel` in your user settings, lowered to the effective ceiling), `ccc` therefore passes only `claude-haiku-4-5`. When no effort is stated, the model default applies and the full chain is passed. Playbook never changes your effort to suit a fallback.

**Overrides.** `playbook config set --global models.sonnet claude-sonnet-5` (also `models.haiku` and `models.opus`) points an alias at another model of the same tier. The value is empty (no override) or `claude-<tier>-<major>[-<minor>]` for its own tier, and anything else is refused. `ccc` and `ccd` export `ANTHROPIC_DEFAULT_<ALIAS>_MODEL` for each override, so subagents and skills that name the alias follow it. A variable you exported yourself wins. Shipped plugin files still name only aliases, so the pin check in `tests/model_pins.rs` is unchanged.

`playbook doctor models` (add `--json`) prints the table, any override, the chain `ccc` passes, whether your own fallback replaces it, and whether the effort in force trimmed the chain.

`tests/model_pins.rs` fails when an agent, command, skill, prompt, output style, workflow or `settings.shared.json` names a model id outside this table, or when an agent uses anything but a tier alias.

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
- [Internals: Launcher and Hooks](01-launcher-and-hooks.md): the `ccc` launcher that sets the session model.
- [Docs index](../index.md)
