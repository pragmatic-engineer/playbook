# Contributing to Playbook

This project follows the standard igorjs contribution rules. Start here:

- **[.github/CONTRIBUTING-RULES.md](.github/CONTRIBUTING-RULES.md)**: DCO, CLA, commit conventions, PR process, and the code style baseline (SPDX headers, dependency policy, commit signing). These rules are shared across all igorjs repos.
- **[README](README.md)**: project description and install.
- **[justfile](justfile)**: build and test commands (`just --list`).
- **[SECURITY.md](SECURITY.md)**: vulnerability disclosure process.

## Project-Specific Notes

See the [justfile](justfile) for build and test commands (`just --list`).

### Repository layout

This is a Cargo workspace. The skills, commands and agents live in `plugins/playbook`. The Rust code lives under `crates/`:

| Folder | Package | What it holds |
| --- | --- | --- |
| `crates/cli` | `playbook` | the `playbook` binary and library, with its tests in `crates/cli/tests` |
| `crates/plugin-checks` | `playbook-plugin-checks` | tests that read `plugins/playbook`: prose, structure and policy table |
| `crates/common` | `playbook-common` | shared helpers: paths, git, sqlite, atomic files |
| `crates/code` | `playbook-code` | the code index, search, symbols and references |
| `crates/telemetry` | `playbook-telemetry` | the usage dashboard, with the `playbook-telemetry` and `pbtop` binaries |
| `crates/data` | `playbook-data` | shared storage (data folder, `config.db`, `memory.db`) |

Run `just test` for everything, or `cargo nextest run -p playbook-code --config-file .github/nextest.toml` for one crate (nextest, because some tests share ports and plain `cargo test` runs them in one process). Doc tests run with `cargo test --workspace --doc`. Tests read the repository through `env!("PLAYBOOK_REPO_ROOT")` and the plugin through `env!("PLAYBOOK_PLUGIN_ROOT")`, both set in `.cargo/config.toml`, instead of `CARGO_MANIFEST_DIR`. Regenerate the reference docs with `UPDATE_REFERENCE=1 cargo test -p playbook --test reference_docs`. See [ADR-0029](docs/adr/0029-cargo-workspace-monorepo.md) for the layout and the release tags.

### Releases

`vX.Y.Z` releases `playbook`. `telemetry-vX.Y.Z` releases `playbook-telemetry`. Pushing the signed tag is enough: `release.yml` creates the release if it is missing (with `--latest=false` for every prefix except `v`, so GitHub's latest release stays on the `v` line), builds and uploads the assets, and stops. The Homebrew tap checks for new releases every 30 minutes and updates the matching formula. To rehearse, run the `release` workflow by hand with a `tag` such as `v0.26.0` or `telemetry-v0.2.0`: a manual run is always a dry run that builds without publishing.

### Compatibility policy

Every release is backward compatible, or ships a migration that runs on first use ([ADR-0034](docs/adr/0034-self-healing-upgrades.md)). A release that still changes what a user sees or does is a breaking change. Mark its commit with `!` after the type or a `BREAKING CHANGE:` footer, and add an entry to `breaking-changes.json` at the repository root with the version, the component (`playbook`, `plugin` or `telemetry`), a one line summary, the action the user takes (or `none, migrated automatically`) and the migration id or `null`. A unit test checks the format of the file. The `breaking-changes` pull request check, the `version-bump` plugin guard and the `verify-version` release job fail when a marked change has no entry, and `scripts/check-breaking.sh` runs the same checks locally. A `plugin` entry also needs a `### Breaking` heading in the matching `## playbook <version>` section of `CHANGELOG.md`. [ADR-0035](docs/adr/0035-breaking-change-notices.md) says how each update channel shows the entries.

## Changing the plugin

The skills, commands, agents and output style live in `plugins/playbook` and ship to users through the `pragmatic-engineer/marketplace` listing. See [ADR-0032](docs/adr/0032-plugin-in-the-playbook-repo.md) for how.

- Keep the folder to markdown and `plugin.json`. The `payload-guard` check fails on anything else.
- Bump `version` in `plugins/playbook/.claude-plugin/plugin.json` and add a `## playbook <version>` section to the top of `CHANGELOG.md` in the same pull request. Claude Code updates the plugin only when the version changes, and the `version-bump` check enforces both. A patch is a wording fix or a repair, a minor version changes how a command, skill or agent behaves, and a major version removes or renames a command or changes the contract with the binary.
- When prose needs a task from the binary, name the task in plain words and list its intent in `intents.json`. Do not write a subcommand into the prose. The `intents-contract` check compares the list with `playbook tools map`.
- Run `cargo test -p playbook-plugin-checks` for the prose and structure checks, and `scripts/check-plugin.sh payload` for the payload.
- `plugin-release.yml` tags and releases the plugin after the pull request merges. Do not tag it by hand.

### Editing agents

The agent files live in `plugins/playbook/agents`. The binary renders effort variants such as `reviewer-low` from the base files for one session. Every base agent needs an entry in the `VARIANTS` table in `crates/cli/src/agents/variants.rs`, with every tier. Run `playbook agents check plugins/playbook/agents` to check the files and `playbook agents variants plugins/playbook/agents` to see what a session would get.

## Project-Specific Code Style

No project-specific code style rules beyond the baseline. See [.github/CONTRIBUTING-RULES.md](.github/CONTRIBUTING-RULES.md#code-style-baseline) for SPDX header, dependency policy, and commit signing requirements.

## Project-Specific Tests

See the [justfile](justfile) for test commands. `just check` runs everything CI runs.
