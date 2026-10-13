# Contributing to Playbook

This project follows the standard igorjs contribution rules. Start here:

- **[.github/CONTRIBUTING-RULES.md](.github/CONTRIBUTING-RULES.md)**: DCO, CLA, commit conventions, PR process, and the code style baseline (SPDX headers, dependency policy, commit signing). These rules are shared across all igorjs repos.
- **[README](README.md)**: project description and install.
- **[justfile](justfile)**: build and test commands (`just --list`).
- **[SECURITY.md](SECURITY.md)**: vulnerability disclosure process.

## Project-Specific Notes

See the [justfile](justfile) for build and test commands (`just --list`).

### Repository layout

This is a Cargo workspace. The Claude Code plugin files (`commands/`, `skills/`, `agents/`, `hooks/`, `output-styles/`, `prompts/`, `bin/`, `install.sh`, `.claude-plugin/`) stay at the repository root, which is the plugin root. The Rust code lives under `crates/`:

| Folder | Package | What it holds |
| --- | --- | --- |
| `crates/cli` | `playbook` | the `playbook` binary and library, with its tests in `crates/cli/tests` |
| `crates/common` | `playbook-common` | shared helpers: paths, git, sqlite, atomic files |
| `crates/code` | `playbook-code` | the code index, search, symbols and references |
| `crates/telemetry` | `playbook-telemetry` | the usage dashboard, with the `playbook-telemetry` and `pbtop` binaries |
| `crates/data` | `playbook-data` | shared storage (data folder, `config.db`, `memory.db`) |

Run `just test` for everything, or `cargo nextest run -p playbook-code --config-file .github/nextest.toml` for one crate (nextest, because some tests share ports and plain `cargo test` runs them in one process). Doc tests run with `cargo test --workspace --doc`. Tests that read plugin files use `env!("PLAYBOOK_REPO_ROOT")`, set in `.cargo/config.toml`, instead of `CARGO_MANIFEST_DIR`. Regenerate the reference docs with `UPDATE_REFERENCE=1 cargo test -p playbook --test reference_docs`. See [ADR-0029](docs/adr/0029-cargo-workspace-monorepo.md) for the layout and the release tags.

### Releases

`vX.Y.Z` releases `playbook`. `telemetry-vX.Y.Z` releases `playbook-telemetry`. Pushing the signed tag is enough: the telemetry workflow creates the release with `--latest=false`, so GitHub's latest release stays on the `v` line. The workflow then tells the Homebrew tap, and the tap also checks daily.

## Editing agents

Agents live in `agents/*.md`, and only the 11 base agents are files. Effort variants such as `reviewer-low` and `reviewer-xhigh` are not: the `Agent` tool has no per-call effort, so `ccc` and `ccd` render the variants from the base files for one session and pass them to Claude Code with `--agents`. Committing them would put dozens of unused descriptions into every session.

- Edit the base file, for example `agents/reviewer.md`. Its `effort:` line is the agent's own effort.
- Every base agent needs an entry in the `VARIANTS` table in `crates/cli/src/agents/variants.rs`, with every tier. A tier equal to the agent's own effort renders nothing.
- Run `playbook agents variants` to see what a session would get (add `--ceiling medium` or `--json`).
- Run `playbook agents check` before you push. It fails on an agent with no `VARIANTS` entry, a tier that cannot render from its base, and any leftover generated variant file. CI runs it too.

See [Authoring agents](docs/authoring/02-authoring-agents.md) for the frontmatter rules.

## Project-Specific Code Style

No project-specific code style rules beyond the baseline. See [.github/CONTRIBUTING-RULES.md](.github/CONTRIBUTING-RULES.md#code-style-baseline) for SPDX header, dependency policy, and commit signing requirements.

## Project-Specific Tests

See the [justfile](justfile) for test commands. `just check` runs everything CI runs.
