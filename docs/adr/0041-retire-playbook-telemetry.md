# ADR-0041: Retire the `playbook-telemetry` binary and formula, and keep `pbtop`

- **Status:** Accepted. The context is amended by [ADR 0043](0043-collector-service-in-setup.md): setup can now install a launchd agent or systemd user unit that runs `playbook usage collect`.
- **Date created:** 2026-10-16
- **Date modified:** 2026-10-17
- **Amends:** [ADR 0038](0038-one-release-line.md) decision 9 and [ADR 0039](0039-release-workflow.md) decisions 6 and 9, which ship and bump the `playbook-telemetry` archive and formula.

## Context

The usage dashboard ships as two commands that are one program: `playbook-telemetry` and its short alias `pbtop`. Both start `playbook_telemetry::cli::main()`, and the name the program was started under only changes its help text. [ADR 0038](0038-one-release-line.md) and [ADR 0039](0039-release-workflow.md) plan two archives for it per target in each `vX.Y.Z` release: `playbook-telemetry-X.Y.Z-<target>.tar.gz` with both names, and `pbtop-X.Y.Z-<target>.tar.gz` with `pbtop` alone. No release has shipped them yet: v0.34.0 was made by hand with the `playbook` binaries only. The Homebrew tap has a formula for each, and the two formulas conflict, because both install `pbtop`.

The `playbook` binary embeds the same crate. `playbook usage` runs the dashboard and the commands around it: `warm`, `collect`, `status`, `doctor` and `prune`. The only gap is the `--utc` and `--local` flags, which the standalone program has and `playbook usage` does not. The collector that receives Claude Code telemetry is only ever started as `playbook usage collect`, by `playbook cc` and by the `session-init` hook. No launchd or systemd unit starts it, and no file records the path of a telemetry binary: `collector.json` holds a process id, a port, a token, a start time and an idle limit, and `telemetry-env.json` holds the `env` keys setup wrote into Claude Code settings.

The long name adds a second archive, a second formula and a conflict between formulas, and it gives users nothing that `pbtop` and `playbook usage` do not.

## Decision

1. **`pbtop` is the one standalone command.** The `playbook-telemetry` binary target is removed from `crates/telemetry`, and the program names itself `pbtop`. The Cargo package keeps the name `playbook-telemetry`, since `playbook` depends on its library, and the config scope `playbook-telemetry` (`APP_TELEMETRY`) stays, since it is a key in stored data and not a command.
2. **Each role has one home.** The dashboard, the summary and the JSON report are `pbtop` on a machine without playbook, and `playbook usage` with it. The collector, `status`, `doctor`, `prune` and `warm` are `playbook usage` subcommands, and `pbtop` keeps them too, since it is the same program. `playbook usage` gains `--utc` and `--local`, so nothing that worked with the long name needs `pbtop` on a machine with playbook.
3. **Releases.** From 0.35.0 a `vX.Y.Z` release has two assets per target, the raw `playbook` binary and `pbtop-X.Y.Z-<target>.tar.gz`, so ten in all, plus `SHA256SUMS`, the notes and their attestations. There is no `playbook-telemetry` archive and no `telemetry-v` tag. The `telemetry-v0.2.0` release and its tag stay, so old formulas and links keep working.
4. **Homebrew.** The tap's `playbook-telemetry` formula gets `disable!` with the date 2026-11-16, the reason "is replaced by pbtop. Run `brew uninstall playbook-telemetry && brew install pbtop`" and `replacement_formula: "pbtop"`. Homebrew accepts a date and a replacement together. Until the date it treats the formula as deprecated and prints the same message. The `disable!` line lands in the tap only after the 0.35.0 release has added the `pbtop` formula, so the message names a formula that exists. If 0.35.0 ships after 2026-11-01, that tap change moves the date to a month after the release. Until then the tap stops bumping `playbook-telemetry` and it keeps working. After the date the formula cannot be installed or upgraded. An installed copy keeps working. This change to `release.yml` merges before the tap change, since the tap's `bump-formula.sh` then refuses `playbook-telemetry`, and a release run that still asks for it would fail. The tap stops bumping the formula and drops `conflicts_with` from both formulas, and its URLs stay on `telemetry-v0.2.0`.
5. **Install script.** `install-pbtop.sh` installs `pbtop` alone (`curl -fsSL https://raw.githubusercontent.com/pragmatic-engineer/playbook/main/install-pbtop.sh | sh`). `install.sh --pbtop` stays as a hidden alias for the 0.35 line and goes in 0.36.0. Both arrive with the change that adds `install-pbtop.sh` (pull request #1078). The Homebrew formula covers macOS and Linux users with Homebrew, and the script covers the rest, so both stay.

## Migration

No on-disk migration is needed, in line with [ADR 0034](0034-self-healing-upgrades.md). The data folder `~/.config/playbook/data`, the `usage.*` keys and `collector.json` are shared and do not name a binary. Nothing in this change writes Claude Code settings. The `env` keys setup wrote point at the collector's port, which `playbook usage collect` serves as before.

What a user does:

- **Homebrew:** `brew uninstall playbook-telemetry && brew install pbtop`. The disabled formula prints this line.
- **Archive install:** delete `playbook-telemetry` from the folder it was copied to, and keep or install `pbtop` (`install-pbtop.sh`).
- **Scripts that call `playbook-telemetry`:** call `pbtop` with the same arguments, or `playbook usage` on a machine with playbook.

There is no entry in the migration registry of ADR 0034 decision 14 either: a `playbook-telemetry` left on `PATH` keeps working and reads the same data, so there is nothing to repair or report on first run. The release that removes the binary carries a `BREAKING CHANGE:` footer with these steps, so its release notes show them first ([ADR 0037](0037-release-notes-from-conventional-commits.md)).

## Deleting the old repository

The archived repository `pragmatic-engineer/playbook-telemetry` is not deleted now. The earliest date to delete it is 2027-04-16, and only when all of these hold:

- The tap no longer reads it: `latest-release.sh` has no fallback to it, and no formula URL points to it.
- No file in `pragmatic-engineer/playbook`, the tap or the marketplace links to it, the crate README included (`git grep playbook-telemetry` shows no repository link).
- The `playbook-telemetry` formula has been disabled for at least five months, and no open issue in the playbook repository or the tap asks for it.
- Its release archives are not needed for a rollback: every supported version of the formula and of `install.sh` installs from `pragmatic-engineer/playbook` releases.
- A maintainer records the deletion in this ADR, with the date.

Deleting it is a manual step for a maintainer. No workflow does it. The `telemetry-v0.2.0` release and tag in `pragmatic-engineer/playbook` are not part of this and stay.

## Alternatives considered

- **Keep `playbook-telemetry` as an alias of `pbtop`.** It keeps scripts working, and it keeps the second archive, the second formula and the conflict between them, which are the costs this record removes.
- **Keep only the long name.** `pbtop` is the name the docs and the help already teach, and it is the name people type.
- **Deprecate the formula now and disable it a release later.** Homebrew takes a disable date and a replacement together, and a date in the future gives the same grace period with one change.

## Consequences

- One command name to document and support. The help text and the docs say `pbtop`.
- Scripts that call `playbook-telemetry` break at 0.35.0 and need the one word change above.
- A user who does not run `brew` again keeps the old formula's binary until they uninstall it. It still works, since it reads the same data.
- [ADR 0038](0038-one-release-line.md) decision 9 and [ADR 0039](0039-release-workflow.md) decisions 6 and 9 name the `playbook-telemetry` archive and formula. This record replaces that part of both.
