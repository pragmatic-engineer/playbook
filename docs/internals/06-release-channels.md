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

A release attaches the five binaries, `release-notes.json`,
`breaking-changes.json` and `SHA256SUMS`, which lists all seven. It builds no plugin
archive. The plugin in `plugins/playbook` ships with the same `vX.Y.Z`, as
described next.

## The plugin release

The binary and the plugin share one version line
([ADR 0038](../adr/0038-one-release-line.md)). The `version` in `plugin.json`
equals the workspace version in `Cargo.toml`, and its
`metadata.minBinaryVersion` is the `X.Y.0` of that minor line. The `version-sync` check in `plugin-guards.yml` fails a pull
request or a push to `main` when they differ, and the `verify-version` job of
`release.yml` checks them again before a `v` release. The release pull request
runs `scripts/bump-version.sh X.Y.Z`, which sets all of them and the
`PLAYBOOK_REF` pins together. A plugin change between releases needs no bump.

The marketplace repository `pragmatic-engineer/marketplace` lists the plugin as
a `git-subdir` source that points at `plugins/playbook` in this repository, with
no `ref` and no `version`. Claude Code follows the default branch, copies only
that folder and updates the plugin when the `version` in `plugin.json` rises,
which happens when the release pull request merges. ADR 0038 moves the entry
to a `ref` on the release tag, which the release workflow sets after the
release is published, starting with the first joint release. There is no separate plugin
release. The `playbook-plugin-v` tags made before 0.34.0 stay as history. The
`payload-guard` check keeps the folder to markdown and the manifest.

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

`scripts/release-notes.sh` builds the body of every release from the
Conventional Commits since the previous tag of the tool that touch its paths
(ADR 0037). The sections are Breaking changes, Features, Fixes, Performance and
Other changes, in that order, and an empty one is left out. A commit is breaking
when it has `!` after its type or scope or a `BREAKING CHANGE:` footer. The same
notes go into `release-notes.json`, an asset that `SHA256SUMS` lists and the
`checksums` job attests. A release drafted by hand before the tag keeps its
body and gets the asset. Run `scripts/release-notes.sh body <component>
<version> <json-out>` to see the notes before you push a tag.

The body of a `v*` release starts with the upgrade note in
`.github/release-notes-upgrade.md`, above the generated notes. A
`telemetry-v*` release gets a short assets note there. When most users have
upgraded from the `playbook-hooks` plugin, delete the file and the line in the
`body` mode of `scripts/release-notes.sh` that prints it. A dry run prints the
body and `release-notes.json` in the run summary.

`breaking-changes.json` is still attached for the Homebrew tap, and
`verify-version` still fails when a commit since the previous tag of the tool
that touches its paths is marked breaking and that file has no entry for the
version (ADR 0035). `scripts/check-breaking.sh` holds that check. Run
`scripts/check-breaking.sh release <component> <version>`, or a dry run of the
workflow from `main`, before you push a tag. When the check fails on a pushed
tag, merge the entry, delete the tag, and push a new signed tag on the commit
that has the entry. A rerun checks out the old commit and fails again.

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
| playbook | `vX.Y.Z` | five binaries, `release-notes.json`, `breaking-changes.json`, `SHA256SUMS` |
| playbook-telemetry | `telemetry-vX.Y.Z` | `playbook-telemetry-<version>-<target>.tar.gz` holding `playbook-telemetry` and `pbtop`, `release-notes.json`, `breaking-changes.json`, `SHA256SUMS` |
| playbook-code | `code-vX.Y.Z` | `playbook-code-<version>-<target>.tar.gz` holding `playbook-code` and `pbcode`, `release-notes.json`, `breaking-changes.json`, `SHA256SUMS` |

The Homebrew tap maps each formula to the repo and tag prefix.
