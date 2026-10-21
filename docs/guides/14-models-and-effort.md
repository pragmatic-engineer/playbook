# Models and effort

Each command and agent names its model and effort in its own frontmatter. Spend on judgment, save on routine work.

| Tier | Model | Falls back to | Used for |
|---|---|---|---|
| Opus | `claude-opus-5-5` | `claude-opus-5` | Design, every reviewer, the repo auditor |
| Sonnet | `claude-sonnet-5-5` | `claude-sonnet-5` | Session default, implementer, critic |
| Haiku | `claude-haiku-5-5` | `claude-haiku-4-5` | Mechanical agents: git, collector, fact-checker, triage |

Plugin files name only the alias, and `playbook cc` passes the fallback chain. Run `playbook doctor models` to see the table, your overrides and the chain in force.

- **Effort ceiling.** `playbook effort <auto|low|medium|high|xhigh|max>` sets `maxEffortLevel`. The lower of this and Claude Code's own `maxEffortLevel` wins, and playbook never changes Claude Code's setting. The ceiling does not cap a command, skill or base agent that ships its own effort, or a level you state for the session, since only Claude Code's setting can. See the [config keys page](../reference/config-keys.md#what-the-effort-ceiling-does-and-does-not-cap). Override one component with `effort.agents.<name>`, `effort.commands.<name>` or `effort.skills.<name>`.
- **Variants.** The `Agent` tool has no per-call effort, so `playbook cc` renders effort variants of the agents for each session (`reviewer-low`, `reviewer-xhigh`). Nothing is committed. `agents.variants` is `auto`, `all` or `off`, and `playbook agents variants` shows what a session gets.
- **Routing.** `playbook route <kind>` says which model, effort and agent a task goes to, and when you must approve first.
- **Measure.** `playbook eval bench` compares roles across models and efforts on real cases, with a cost cap.

The per-component table is the [policy table](../reference/policy-table.md). Reasoning is in [Why the pieces are shaped this way](../internals/14-why-the-pieces-are-shaped-this-way.md) and [Model routing](../internals/02-model-routing-and-memory.md).
