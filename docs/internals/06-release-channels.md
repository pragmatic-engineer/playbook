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

A release attaches the five binaries and `SHA256SUMS`. It builds no plugin
archive. The skills, commands and agents ship from `pragmatic-engineer/skills`
and have their own version line there.

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
| playbook | `vX.Y.Z` | five binaries, `SHA256SUMS` |
| playbook-telemetry | `telemetry-vX.Y.Z` | `playbook-telemetry-<version>-<target>.tar.gz` holding `playbook-telemetry` and `pbtop`, `SHA256SUMS` |
| playbook-code | `code-vX.Y.Z` | `playbook-code-<version>-<target>.tar.gz` holding `playbook-code` and `pbcode`, `SHA256SUMS` |

The Homebrew tap maps each formula to the repo and tag prefix.
