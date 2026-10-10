# Contributing to Playbook

This project follows the standard igorjs contribution rules. Start here:

- **[.github/CONTRIBUTING-RULES.md](.github/CONTRIBUTING-RULES.md)**: DCO, CLA, commit conventions, PR process, and the code style baseline (SPDX headers, dependency policy, commit signing). These rules are shared across all igorjs repos.
- **[README](README.md)**: project description and install.
- **[justfile](justfile)**: build and test commands (`just --list`).
- **[SECURITY.md](SECURITY.md)**: vulnerability disclosure process.

## Project-Specific Notes

No project-specific notes beyond the baseline rules. See the [justfile](justfile) for build and test commands (`just --list`).

## Editing agents

Agents live in `agents/*.md`, and only the 11 base agents are files. Effort variants such as `reviewer-low` and `reviewer-xhigh` are not: the `Agent` tool has no per-call effort, so `ccc` and `ccd` render the variants from the base files for one session and pass them to Claude Code with `--agents`. Committing them would put dozens of unused descriptions into every session.

- Edit the base file, for example `agents/reviewer.md`. Its `effort:` line is the agent's own effort.
- Every base agent needs an entry in the `VARIANTS` table in `src/agents/variants.rs`, with every tier. A tier equal to the agent's own effort renders nothing.
- Run `playbook agents variants` to see what a session would get (add `--ceiling medium` or `--json`).
- Run `playbook agents check` before you push. It fails on an agent with no `VARIANTS` entry, a tier that cannot render from its base, and any leftover generated variant file. CI runs it too.

See [Authoring agents](docs/authoring/02-authoring-agents.md) for the frontmatter rules.

## Project-Specific Code Style

No project-specific code style rules beyond the baseline. See [.github/CONTRIBUTING-RULES.md](.github/CONTRIBUTING-RULES.md#code-style-baseline) for SPDX header, dependency policy, and commit signing requirements.

## Project-Specific Tests

See the [justfile](justfile) for test commands. `just check` runs everything CI runs.
