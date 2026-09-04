# Playbook docs

How to use and extend this Claude Code config. The [README](../README.md) covers install plus a short summary of every command and skill. These pages go deeper: why the config is shaped the way it is, real workflows end to end, how to add your own pieces, and how the machine works underneath.

Read them in order, or jump to what you need.

## Concepts

Why the foundations work the way they do.

- [The system prompt](concepts/01-system-prompt.md): what the custom prompt defines, and why a custom prompt behaves better than the default.
- [The memory system](concepts/02-memory-system.md): the global, org, and project fact store, typed edges, and how it feeds the commands.

## Guides

Task-oriented workflows.

- [Install](guides/00-install.md): requirements, the full local curl install, settings merge, and uninstall.
- [Plan and implement](guides/01-plan-and-implement.md): design with `/playbook:scope`, build with `/playbook:implement`.
- [Review and PR flow](guides/02-review-and-pr-flow.md): commit, review, and work through feedback.
- [Decisions and memory](guides/03-decisions-and-memory.md): record choices with `/playbook:adr`, build project knowledge with `/playbook:learn-project`.

## Authoring

Extend the config.

- [Commands, skills, and hooks](authoring/01-commands-skills-hooks.md): templates for each extension point.
- [Authoring agents](authoring/02-authoring-agents.md): the frontmatter schema, both binding mechanisms, and the parametrize or split rule.

## Internals

How the machine works.

- [Launcher and hooks](internals/01-launcher-and-hooks.md): the `cc` launcher, the worktree engine, and the hook lifecycle.
- [Model routing and memory](internals/02-model-routing-and-memory.md): how the session model is chosen, and the memory graph mechanics.
