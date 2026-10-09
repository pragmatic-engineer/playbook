# Playbook docs

How to use and extend this Claude Code config. The [README](../README.md) covers install plus a short summary of every command and skill. These pages go deeper: why the config is shaped the way it is, real workflows end to end, how to add your own pieces, and how the machine works underneath.

Read them in order, or jump to what you need.

## Concepts

Why the foundations work the way they do.

- [The system prompt](concepts/01-system-prompt.md): what the custom prompt defines, and why a custom prompt behaves better than the default.
- [The memory system](concepts/02-memory-system.md): the global, org, and project fact store, typed edges, and how it feeds the commands.
- [Why the pieces are shaped this way](concepts/03-why-the-pieces-are-shaped-this-way.md): why commands, skills, agents and hooks are separate, why each model and effort level was chosen, and where to change them.

## Guides

Task-oriented workflows.

- [Install](guides/00-install.md): requirements, the full local curl install, settings merge, and uninstall.
- [Plan and implement](guides/01-plan-and-implement.md): design with `/playbook:plan`, build with `/playbook:implement`, fix a small bug with `/playbook:fix`.
- [Review and PR flow](guides/02-review-and-pr-flow.md): commit, review, and work through feedback.
- [Decisions and memory](guides/03-decisions-and-memory.md): record choices with `/playbook:adr`, build project knowledge with `/playbook:learn-project`.
- [Config keys](guides/04-config-keys.md): every `playbook config` key with its default, allowed values and what it does.

## Authoring

Extend the config.

- [Commands, skills, and hooks](authoring/01-commands-skills-hooks.md): templates for each extension point.
- [Authoring agents](authoring/02-authoring-agents.md): the frontmatter schema, both binding mechanisms, and the parametrize or split rule.

## Internals

How the machine works.

- [Launcher and hooks](internals/01-launcher-and-hooks.md): the `ccc` launcher, the worktree engine, and the hook lifecycle.
- [Model routing and memory](internals/02-model-routing-and-memory.md): how the session model is chosen, and the memory graph mechanics.
- [Worktree engine](internals/03-worktree.md): the `ccc worktree` background subshell, node_modules cloning, and rebase conflict resolution.
- [Usage dashboard](internals/04-usage-dashboard.md): `playbook usage`, where the numbers come from, the store, and how the local dashboard server runs safely.
- [Headless mode](internals/05-headless-mode.md): what runs without a person at the keyboard, real `claude -p` exit codes, how the hooks behave headless, and a first CI use case.
- [Release channels](internals/06-release-channels.md): how a tagged release reaches GitHub assets, the Homebrew tap and the marketplace, the trimmed plugin archive, and the secret it needs.
- [Migrations](internals/07-migrations.md): how `playbook init` carries an install across breaking changes, the three migration kinds, and how edits to placed files are detected.
- [State store](internals/08-state-store.md): where playbook keeps its internal state, how to inspect it with `playbook state list`, and the one-time import of the old files.

## Decisions

Architecture decision records in [`docs/adr/`](adr/). A decision can have a blueprint, quality report or test mapping file next to it; only the decision itself is listed here.

| ADR | Title | Status | Date |
|---|---|---|---|
| [0001](adr/0001-package-toolkit-as-plugin.md) | Package the toolkit as an opt-in Claude Code plugin | Accepted, amended by 0002, 0006, 0007 | 2026-07-23 |
| [0002](adr/0002-plugin-based-install-safety-hooks.md) | Plugin based install with always-on safety hooks | Accepted, amended by 0006, 0007 | 2026-07-30 |
| [0003](adr/0003-purpose-built-subagents.md) | Purpose-built subagents over generic fallbacks | Accepted | 2026-08-07 |
| [0004](adr/0004-graph-first-memory.md) | Graph-first memory retrieval and triggered capture | Accepted | 2026-08-10 |
| [0005](adr/0005-python-hooks-and-config-scripts.md) | Migrate the hooks and the config scripts from shell to Python | Superseded by 0007 | 2026-08-11 |
| [0006](adr/0006-separate-product-tree-from-runtime-dir.md) | Separate the product tree from the Claude Code runtime directory | Accepted | 2026-08-12 |
| [0007](adr/0007-rust-binary-for-hooks-and-launcher.md) | Move the hooks and the launcher into a single Rust binary | Accepted | 2026-08-13 |
| [0008](adr/0008-bounded-memory-injection-with-prompt-recall-and-handoff-continuity.md) | Bounded memory injection, prompt-time recall, and handoff continuity across /clear | Accepted | 2026-08-24 |
| [0009](adr/0009-evidence-verified-memory-capture-and-bounded-handoff-nudging.md) | Evidence-verified memory capture and bounded handoff nudging | Accepted | 2026-08-25 |
| [0010](adr/0010-agent-agnostic-repo-local-storage.md) | Agent-agnostic repo-local storage | Superseded by 0012 | 2026-08-30 |
| [0011](adr/0011-memory-signals-usage-staleness-similarity-and-consolidation.md) | Memory signals: usage promotion, staleness, similarity edges, and consolidation | Accepted | 2026-08-27 |
| [0012](adr/0012-unify-state-under-config-playbook.md) | Unify home and repo-local state under `$HOME/.config/playbook/` | Accepted | 2026-09-03 |
| [0013](adr/0013-org-level-memory-scoping.md) | Org-level memory scoping | Accepted | 2026-09-04 |
| [0014](adr/0014-retire-memory-md-index.md) | Retire MEMORY.md as a parallel memory index | Accepted | 2026-09-09 |
| [0015](adr/0015-multi-agent-support.md) | Multi-agent support via a thin agent-neutral core and per-agent adapters | Accepted | 2026-10-07 |
| [0016](adr/0016-migration-framework.md) | A registry of ordered migrations run from `playbook init` | Accepted | 2026-10-07 |
