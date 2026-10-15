# Release channels

A tagged release reaches users through two channels: the GitHub release assets
(built by the `build` and `checksums` jobs of `release.yml`) and the Homebrew
tap `pragmatic-engineer/homebrew-tap`. This repo does not push to the tap. The
tap runs its own workflow that reads this repo's releases and updates itself. It
checks every 30 minutes for the newest release under each formula's tag prefix,
rewrites the URLs and checksums in `Formula/playbook.rb` from the release's
`SHA256SUMS`, then audits, installs and tests the formula. The commit goes
through the GitHub API, so GitHub signs it. The tap follows a release within
about half an hour. To update sooner, run the `release` workflow by hand in the
tap repo.

A release attaches the five binaries, `breaking-changes.json` and `SHA256SUMS`,
which lists all six. It builds no plugin
archive. The plugin in `plugins/playbook` has its own version line and its own
release, described next.

## The plugin release

The marketplace repository `pragmatic-engineer/marketplace` lists the plugin as
a `git-subdir` source that points at `plugins/playbook` in this repository, with
no `ref` and no `version`. Claude Code follows the default branch, copies only
that folder and updates the plugin when the `version` in `plugin.json` changes.

A change under `plugins/playbook` needs a higher version in `plugin.json` and a
`## playbook <version>` section at the top of `CHANGELOG.md`. The `version-bump`
check enforces both and the `payload-guard` check keeps the folder to markdown
and the manifest. When `plugin.json` or `CHANGELOG.md` changes on `main`,
`plugin-release.yml` creates the tag `playbook-plugin-v<version>` and a release
with that changelog section. It stops when the tag exists and marks the release
as not latest. The tag does not match `v*`, so `release.yml` never runs for it.
See [ADR 0032](../adr/0032-plugin-in-the-playbook-repo.md) for the versioning
rules.

## One workflow, one tag prefix per tool

`.github/workflows/release.yml` releases every tool. It runs on a pushed tag
matching `v*`, `telemetry-v*` or `code-v*`. Its first job, `resolve`, maps the
prefix to the package, the binaries, the archive shape, the version file and the
tool name, and fails on any other tag. `code-v*` is reserved and fails
until `crates/code` has a binary.

The jobs after `resolve` are shared. `verify-version` checks the tag against the
tool's `Cargo.toml`. `create-release`
creates the GitHub release when it is missing, with `--latest=false` for every
prefix except `v`, so the repository's latest release stays on the playbook
line. `build` compiles the five targets, ad hoc signs the macOS binaries and
uploads one asset per target. `checksums` writes `SHA256SUMS` and attests the
assets. The workflow only publishes.

The body of every release starts with a `Breaking changes` section when
`breaking-changes.json` has entries for the tool and version. On a release
drafted by hand before the tag, `create-release` puts the section on top of
the existing body. Every release attaches that file as an asset for the
Homebrew tap (ADR 0035). `verify-version` fails when a commit since the
previous tag of the tool that touches its paths has `!` after its type or a
`BREAKING CHANGE:` footer, and the file has no entry for the version.
`scripts/check-breaking.sh` holds these checks, and its `body` mode builds the
start of the body.

Run `scripts/check-breaking.sh release <component> <version>`, or a dry run
of the workflow from `main`, before you push a tag. When the check fails on a
pushed tag, merge the entry, delete the tag, and push a new signed tag on the
commit that has the entry. A rerun checks out the old commit and fails again.

The body of a `v*` release then goes on with the upgrade note in
`.github/release-notes-upgrade.md`, above the generated notes. A
`telemetry-v*` release gets a short assets note instead. When most users have
upgraded from the `playbook-hooks` plugin, delete the file and the line in the
`body` mode of `scripts/check-breaking.sh` that prints it. A dry run prints
the start of the body, breaking changes included, in the run summary.

To rehearse, run the `release` workflow by hand with a `tag` input. A manual run
is always a dry run: it resolves, verifies and builds, and skips release
creation, uploads and attestation. The tag's version must match
the files on the ref you run it from.

## Internal tool crates

`playbook-telemetry` (and later `playbook-code`) are workspace members of this
repo under `crates/`. `playbook` depends on them by path, so there is one
`Cargo.lock` and no bump pull requests. Each tool still releases on its own
tag prefix and keeps its own version in its own `Cargo.toml`:

| Tool | Tag | Release assets |
| --- | --- | --- |
| playbook | `vX.Y.Z` | five binaries, `breaking-changes.json`, `SHA256SUMS` |
| playbook-telemetry | `telemetry-vX.Y.Z` | `playbook-telemetry-<version>-<target>.tar.gz` holding `playbook-telemetry` and `pbtop`, `breaking-changes.json`, `SHA256SUMS` |
| playbook-code | `code-vX.Y.Z` | `playbook-code-<version>-<target>.tar.gz` holding `playbook-code` and `pbcode`, `breaking-changes.json`, `SHA256SUMS` |

The Homebrew tap maps each formula to the repo and tag prefix.
