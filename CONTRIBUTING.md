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

### Faster local builds

These settings are per shell or per command, so nothing changes for other contributors. Measured once each on an M-series Mac with Rust 1.99.1 and `CARGO_INCREMENTAL=0`, building every test binary with `cargo test --workspace --no-run`:

| Setting | Clean build | Rebuild after editing `playbook-common` | Target folder |
| --- | --- | --- | --- |
| debug info on (the default) | 43 s | 20 s | 1.7 GB |
| `CARGO_PROFILE_DEV_DEBUG=0 CARGO_PROFILE_TEST_DEBUG=0` | 35 s | 18 s | 1.0 GB |
| the above and `ld64.lld` | 36 s | 16 s | 1.0 GB |

- **Run only the tests you need.** `just test` passes its arguments to nextest with the repo's config file, for example `just test -E 'test(held_lock)'`, and then runs the doc tests. For one crate, use the `cargo nextest run -p` command above, which builds only that crate.
- **Drop debug info.** `export CARGO_PROFILE_DEV_DEBUG=0 CARGO_PROFILE_TEST_DEBUG=0` makes builds faster and the target folder smaller. A panic message keeps its file and line, but backtraces and debuggers show only function names. Unset both when you need a debugger.
- **Keep incremental builds on locally.** CI sets `CARGO_INCREMENTAL=0` because each run builds once. On your machine, incremental builds make the edit and test loop faster.
- **Linker.** The rustup toolchain for x86_64 Linux already links with `rust-lld`. In CI, mold was slower: 44 s against 37 s to rebuild the test binaries. On macOS, `ld64.lld` from `brew install lld` saved 2 s per rebuild in one run: `RUSTFLAGS="-C link-arg=-fuse-ld=$(brew --prefix lld)/bin/ld64.lld"`. Cargo rebuilds everything whenever `RUSTFLAGS` changes, so set it the same way in every shell and editor, or in your own `~/.cargo/config.toml`.
- **Disk.** One `CARGO_TARGET_DIR` per worktree keeps parallel builds from blocking each other, and each one grows to 1 GB or more. Run `cargo clean` in a worktree you are done with.

The repository has no shared `.cargo/config.toml` build settings for speed, because a debugger or a missing linker would break them for some contributors.

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
- A change to playbook's own config, its data files, the entries it writes into Claude Code settings, or the contract between the binary and the plugin keeps working after an upgrade or ships a migration that runs on first use ([ADR-0034](docs/adr/0034-self-healing-upgrades.md)).

### Adding a migration

A migration is named by a slug and the release that ships it ([ADR-0046](docs/adr/0046-migrations-by-release-version.md)).

- Append the entry to `registry()` in `crates/cli/src/init/migrate.rs` with a new slug, such as `move-old-cache`, and `since: "next",` on its own line in exactly that form. The release pull request writes the version in its place.
- Never rename or move an entry. The record and the doctor name it by its slug.
- A build skips a `next` migration, so it never runs from a dev build of `main`. Test it with the unit tests next to it in that file, or build from a scratch checkout after `scripts/release-prepare.sh <next version>`.
- [Internals: Migrations](docs/internals/07-migrations.md) explains the kinds and the scopes.

### Adding a command

`playbook commands` lists every visible command at run time, from the clap tree. It needs one line per command in `META` in `crates/cli/src/command_index.rs`:

- `ro("path")` for a command that only reads, `free("path")` for one that changes the repo or playbook's own state, and `user("path")` for one only the user may run: it writes their home or Claude Code settings, asks for consent, runs a paid benchmark or starts a session.
- Add `.purpose("...")` when the first sentence of the doc comment is too long or too short: 6 to 12 words for a top-level command, 3 to 14 for a subcommand.
- Keep `META` sorted by path. A group needs its own line too.

A test walks the whole tree and fails when a visible command has no line, a line names a command that is gone, or a purpose has the wrong length. Hidden commands, such as a deprecated name, need no line. Then regenerate the reference page with `UPDATE_REFERENCE=1 cargo test -p playbook --test reference_docs`.

Renaming a command is a deprecation. Keep the old name as a hidden alias that runs the same code, prints one line on stderr in a terminal that names the new command, and keeps its exit codes. Since the old name keeps working, the commit is not marked breaking. Remove the alias no earlier than the next major release, and mark that removal as breaking. The rename to `playbook setup` is the example ([ADR-0040](docs/adr/0040-rename-init-to-setup.md)).

## Changing the plugin

The skills, commands, agents and output style live in `plugins/playbook` and ship to users through the `pragmatic-engineer/marketplace` listing. See [ADR-0032](docs/adr/0032-plugin-in-the-playbook-repo.md) for how.

- Keep the folder to markdown and `plugin.json`. The `payload-guard` check fails on anything else.
- Do not bump the plugin version. The plugin ships with the next `vX.Y.Z` release, which sets one version for the binary and the plugin ([ADR-0038](docs/adr/0038-one-release-line.md)). Claude Code updates an installed plugin only when that version rises, so a plugin fix reaches users with the next release.
- When prose needs a task from the binary, name the task in plain words and list its intent in `plugins/intents.json`. Do not write a subcommand into the prose. The one exception is the `playbook:cli` skill, which points to `playbook commands` for the full list. The `intents-contract` check compares the list with `playbook tools map`, and fails when a command the tool map or the `playbook:cli` skill names is missing from `playbook commands --json`.
- Run `cargo test -p playbook-plugin-checks` for the prose and structure checks, and `scripts/check-plugin.sh payload` for the payload.

### Editing agents

The agent files live in `plugins/playbook/agents`. The binary renders effort variants such as `reviewer-low` from the base files for one session. Every base agent needs an entry in the `VARIANTS` table in `crates/cli/src/agents/variants.rs`, with every tier. Run `playbook agents check plugins/playbook/agents` to check the files and `playbook agents variants plugins/playbook/agents` to see what a session would get.

## Project-Specific Code Style

No project-specific code style rules beyond the baseline. See [.github/CONTRIBUTING-RULES.md](.github/CONTRIBUTING-RULES.md#code-style-baseline) for SPDX header, dependency policy, and commit signing requirements.

## Project-Specific Tests

See the [justfile](justfile) for test commands. `just check` runs everything CI runs.

A test that starts the built `playbook` gives it a scratch home: use `isolated::playbook()` from `crates/cli/tests/support/isolated.rs`, or set `HOME` yourself. `test_isolation.rs` fails for a spawn that does neither. As a second net, a debug build started by cargo or nextest does not run migrations, the upgrade heal or the real `launchctl` and `systemctl` against the real home of your account, and prints a note instead. Other writes are not covered by that net, so the scratch home stays the rule. Set `PLAYBOOK_ALLOW_REAL_HOME=1` to run a dev build against your real home on purpose.
