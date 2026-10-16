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
| `crates/telemetry` | `playbook-telemetry` | the usage dashboard, with the `pbtop` binary |
| `crates/data` | `playbook-data` | shared storage (data folder, `config.db`, `memory.db`) |

Run `just test` for everything, or `cargo nextest run -p playbook-code --config-file .github/nextest.toml` for one crate (nextest, because some tests share ports and plain `cargo test` runs them in one process). Doc tests run with `cargo test --workspace --doc`. Tests read the repository through `env!("PLAYBOOK_REPO_ROOT")` and the plugin through `env!("PLAYBOOK_PLUGIN_ROOT")`, both set in `.cargo/config.toml`, instead of `CARGO_MANIFEST_DIR`. Regenerate the reference docs with `UPDATE_REFERENCE=1 cargo test -p playbook --test reference_docs`. See [ADR-0029](docs/adr/0029-cargo-workspace-monorepo.md) for the layout and the release tags.

### Releases

`vX.Y.Z` releases the `playbook` binary, the plugin and `pbtop` together, with one version ([ADR-0038](docs/adr/0038-one-release-line.md)). The `release` workflow does every step ([ADR-0039](docs/adr/0039-release-workflow.md)):

```bash
gh workflow run release.yml -f bump=patch -f dry_run=true   # rehearse
gh workflow run release.yml -f bump=patch                   # or minor, major, or -f version=X.Y.Z
```

It opens the release pull request with `scripts/bump-version.sh` and the changelog section, waits for the merge queue, tags the merged commit, builds and attests the assets, publishes the release as the latest and checks it, then opens the release pull requests in the Homebrew tap and the marketplace and waits for their own `vX.Y.Z` releases. When a run fails, run it again with `-f version=X.Y.Z`: it goes on from where it stopped. [Release channels](docs/internals/06-release-channels.md) has the details and the rollback steps.

### Breaking changes before 1.0

Playbook is before 1.0, so a breaking change can ship in any release. A change is breaking when a user who upgrades has to do something, or when something they use stops working or works in a new way.

- Mark it with `!` after the type or scope in the pull request title, as in `feat(cli)!: drop the old flag`, or with a `BREAKING CHANGE:` footer at the end of the body. The title and body become the squash commit.
- Give the footer one line that says what breaks and what to do, as in `BREAKING CHANGE: the config moved; run playbook setup`. The `breaking-changes` check warns when the title is marked and the body has no footer.
- The release notes list breaking changes first ([ADR-0037](docs/adr/0037-release-notes-from-conventional-commits.md)). `scripts/release-notes.sh` builds them from the commits.
- Until the binary reads `release-notes.json`, a breaking change to playbook also gets an entry in `breaking-changes.json`, so automatic updates hold the release.
- A change to playbook's own config, its data files, the entries it writes into Claude Code settings, or the contract between the binary and the plugin keeps working after an upgrade or ships a migration that runs on first use ([ADR-0034](docs/adr/0034-self-healing-upgrades.md)).

Renaming a command is a deprecation. Keep the old name as a hidden alias that runs the same code, prints one line on stderr in a terminal that names the new command, and keeps its exit codes. Since the old name keeps working, the commit is not marked breaking. Remove the alias no earlier than the next major release, and mark that removal as breaking. The rename to `playbook setup` is the example ([ADR-0040](docs/adr/0040-rename-init-to-setup.md)).

## Changing the plugin

The skills, commands, agents and output style live in `plugins/playbook` and ship to users through the `pragmatic-engineer/marketplace` listing. See [ADR-0032](docs/adr/0032-plugin-in-the-playbook-repo.md) for how.

- Keep the folder to markdown and `plugin.json`. The `payload-guard` check fails on anything else.
- Do not bump the plugin version. The plugin ships with the next `vX.Y.Z` release, which sets one version for the binary and the plugin ([ADR-0038](docs/adr/0038-one-release-line.md)). Claude Code updates an installed plugin only when that version rises, so a plugin fix reaches users with the next release.
- When prose needs a task from the binary, name the task in plain words and list its intent in `intents.json`. Do not write a subcommand into the prose. The `intents-contract` check compares the list with `playbook tools map`.
- Run `cargo test -p playbook-plugin-checks` for the prose and structure checks, and `scripts/check-plugin.sh payload` for the payload.

### Editing agents

The agent files live in `plugins/playbook/agents`. The binary renders effort variants such as `reviewer-low` from the base files for one session. Every base agent needs an entry in the `VARIANTS` table in `crates/cli/src/agents/variants.rs`, with every tier. Run `playbook agents check plugins/playbook/agents` to check the files and `playbook agents variants plugins/playbook/agents` to see what a session would get.

## Project-Specific Code Style

No project-specific code style rules beyond the baseline. See [.github/CONTRIBUTING-RULES.md](.github/CONTRIBUTING-RULES.md#code-style-baseline) for SPDX header, dependency policy, and commit signing requirements.

## Project-Specific Tests

See the [justfile](justfile) for test commands. `just check` runs everything CI runs.
