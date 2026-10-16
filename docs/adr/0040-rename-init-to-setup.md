# ADR-0040: Rename `playbook init` to `playbook setup` and keep `init` as a deprecated name

- **Status:** Accepted
- **Date created:** 2026-10-16
- **Date modified:** 2026-10-16

## Context

The command that installs and repairs the local setup is `playbook init`. The plugin command that runs it is `/playbook:setup`. Users meet both names for one job, and `init` reads like a command that runs once, while this one is safe to run again and repairs what is missing.

Hints across the binary, the plugin, the install script and the docs name `playbook init`. Binaries from before this change have no `setup` command, and the plugin still supports them while `metadata.minBinaryVersion` in `plugins/playbook/.claude-plugin/plugin.json` allows them.

## Decision

1. **`playbook setup` is the command.** It takes every flag `init` takes today (`--only`, `--yes`, `--hooks`, `--no-hooks`, `--statusline`, `--telemetry`, `--security`, `--dry-run`, `--explain`, `--short-name` and the rest), asks the same questions in a terminal and keeps the same rules without one. `playbook --help` lists `setup`.
2. **`playbook init` stays as a deprecated name for the whole 0.x line.** It is hidden from `playbook --help` and runs the same code as `setup`, with the same output and exit codes. When stderr is a terminal, it first prints one line on stderr: `playbook init is deprecated, use: playbook setup`. It prints nothing on stdout, and nothing extra when stderr is not a terminal, so scripts and the plugin, which read the output, see what `setup` prints.
3. **This counts as a deprecation.** It breaks nothing: every command line that worked keeps working. The commit carries no breaking marker, and the release notes list it as a feature.
4. **Hints name the command that works.** Text the binary prints names `playbook setup`, since that binary has it. Text in the plugin, which can run next to an older binary, says `playbook setup (older versions: playbook init)`. So does text in the telemetry crate, since `playbook-telemetry` and `pbtop` can be installed on their own and run next to an older `playbook`. `playbook usage` prints the same text, note included. The `/playbook:setup` command runs `playbook setup` and falls back to `playbook init` when the binary does not know `setup`. The install script does the same check. The plugin has no tool map intent for this command, so the fallback is written out. The change that adds the command also moves the install script. The binary's messages, the docs and the plugin text move in the changes right after it, before the next release.
5. **Stored state stays as it is.** No config key, migration id, `state` table key, settings marker or rc file block holds the word `init` as a command name. The `shell-init` command, the `session-init` hook and the `# playbook binary` marker keep their names, so a downgrade and an older `playbook uninstall` keep working. This change adds no migration.
6. **Internal names stay.** The Rust module `crates/cli/src/init`, the `heal::Trigger::Init` variant and the `init_*` test files keep their names. Renaming them changes no behaviour and would make this change harder to review.

### Removal plan

The earliest release that can remove `playbook init` is 1.0.0. Removing it is a breaking change, so that release marks it as one. Before removal, all of these must hold:

- The plugin's `metadata.minBinaryVersion` is at or above the first release that ships `playbook setup`, so no supported binary needs the plugin fallback. The removal deletes the fallback in `/playbook:setup` and in the install script.
- `grep -rn 'playbook init' crates plugins install.sh .github README.md CONTRIBUTING.md docs/guides docs/reference docs/internals` finds only the deprecation line and the `(older versions: playbook init ...)` notes from decision 4. The removal deletes those too.
- A search of the open issues and discussions in `pragmatic-engineer/playbook` and of the Homebrew tap shows no report that still runs `playbook init`.

Playbook sends no usage data off the machine, so these checks stand in for a count of how often the old name runs.

## Earlier records that say `init`

These records stay as written. Where they say `playbook init` or `init`, read `playbook setup`:

- [ADR 0002](0002-plugin-based-install-safety-hooks.md)
- [ADR 0007](0007-rust-binary-for-hooks-and-launcher.md), with its blueprint, quality and test mapping files
- [ADR 0011](0011-memory-signals-usage-staleness-similarity-and-consolidation-blueprint.md) blueprint
- [ADR 0012](0012-unify-state-under-config-playbook.md) and its blueprint
- [ADR 0015](0015-multi-agent-support.md)
- [ADR 0016](0016-migration-framework.md)
- [ADR 0031](0031-hooks-in-settings-json.md)
- [ADR 0033](0033-telemetry-on-by-default.md)
- [ADR 0034](0034-self-healing-upgrades.md)
- [ADR 0035](0035-breaking-change-notices.md)
- [ADR 0036](0036-release-check-and-automatic-updates.md)
- [ADR 0037](0037-release-notes-from-conventional-commits.md)

## Alternatives considered

- **Rename with no alias.** Rejected: scripts, docs and older plugin versions that run `playbook init` would fail with no warning.
- **Keep `init` visible in help as an alias.** Rejected: two names in the command list keep the confusion this change removes.
- **Print the deprecation line on every run.** Rejected: the plugin and scripts read the output, and an extra line there changes what they parse and report.
- **Remove `init` in a 0.x minor release.** Rejected: users on the 0.x line expect a command that worked to keep working until 1.0.

## Consequences

- One name for one job in the binary, the plugin and the docs.
- A user who types `playbook init` in a terminal sees the new name once per run and loses nothing.
- The plugin and the install script carry a fallback until the removal checks pass.
