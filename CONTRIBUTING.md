# Contributing to Playbook

This project follows the standard igorjs contribution rules. Start here:

- **[.github/CONTRIBUTING-RULES.md](.github/CONTRIBUTING-RULES.md)**: DCO, CLA, commit conventions, PR process, and the code style baseline (SPDX headers, dependency policy, commit signing). These rules are shared across all igorjs repos.
- **[README](README.md)**: project description and install.
- **[justfile](justfile)**: build and test commands (`just --list`).
- **[SECURITY.md](SECURITY.md)**: vulnerability disclosure process.

## Project-Specific Notes

See the [justfile](justfile) for build and test commands (`just --list`).

### Repository layout

This is a Cargo workspace. The hooks plugin files (`hooks/`, `prompts/`, `bin/`, `install.sh`) stay at the repository root, which is the plugin root. The skills, commands and agents live in [pragmatic-engineer/skills](https://github.com/pragmatic-engineer/skills). The Rust code lives under `crates/`:

| Folder | Package | What it holds |
| --- | --- | --- |
| `crates/cli` | `playbook` | the `playbook` binary and library, with its tests in `crates/cli/tests` |
| `crates/common` | `playbook-common` | shared helpers: paths, git, sqlite, atomic files |
| `crates/code` | `playbook-code` | the code index, search, symbols and references |
| `crates/telemetry` | `playbook-telemetry` | the usage dashboard, with the `playbook-telemetry` and `pbtop` binaries |
| `crates/data` | `playbook-data` | shared storage (data folder, `config.db`, `memory.db`) |

Run `just test` for everything, or `cargo nextest run -p playbook-code --config-file .github/nextest.toml` for one crate (nextest, because some tests share ports and plain `cargo test` runs them in one process). Doc tests run with `cargo test --workspace --doc`. Tests that read plugin files use `env!("PLAYBOOK_REPO_ROOT")`, set in `.cargo/config.toml`, instead of `CARGO_MANIFEST_DIR`. Regenerate the reference docs with `UPDATE_REFERENCE=1 cargo test -p playbook --test reference_docs`. See [ADR-0029](docs/adr/0029-cargo-workspace-monorepo.md) for the layout and the release tags.

### Releases

`vX.Y.Z` releases `playbook`. `telemetry-vX.Y.Z` releases `playbook-telemetry`. Pushing the signed tag is enough: `release.yml` creates the release if it is missing (with `--latest=false` for every prefix except `v`, so GitHub's latest release stays on the `v` line), builds and uploads the assets, and stops. The Homebrew tap checks for new releases every 30 minutes and updates the matching formula. To rehearse, run the `release` workflow by hand with a `tag` such as `v0.25.0` or `telemetry-v0.2.0`: a manual run is always a dry run that builds without publishing.

## Editing agents

The agent files live in [pragmatic-engineer/skills](https://github.com/pragmatic-engineer/skills) under `plugins/playbook/agents`. The binary renders effort variants such as `reviewer-low` from the base files for one session. Every base agent needs an entry in the `VARIANTS` table in `crates/cli/src/agents/variants.rs`, with every tier. Run `playbook agents variants` and `playbook agents check` against a checkout of the skills repo to see what a session would get.

## Project-Specific Code Style

No project-specific code style rules beyond the baseline. See [.github/CONTRIBUTING-RULES.md](.github/CONTRIBUTING-RULES.md#code-style-baseline) for SPDX header, dependency policy, and commit signing requirements.

## Project-Specific Tests

See the [justfile](justfile) for test commands. `just check` runs everything CI runs.
