# Release channels

A tagged release reaches users through three channels: the GitHub release assets
(built by the `build` and `checksums` jobs of `release.yml`), the Homebrew tap
`pragmatic-engineer/homebrew-tap`, and the plugin marketplace, which lives in
`pragmatic-engineer/skills`. This repo does not push to the last two. Each of
those repos runs its own workflow, the tap every 30 minutes, that reads this repo's latest release and
updates itself:

1. The tap rewrites the URLs and checksums in `Formula/playbook.rb` from the
   release's `SHA256SUMS`, then audits, installs and tests the formula.
2. The marketplace points the `playbook-hooks` entry at the release's hooks
   plugin archive, with the archive's `url` and `sha256`. It verifies the
   archive's build provenance attestation first. The `playbook` entry (the
   skills, commands and agents) is not pinned to a release: it is a path inside
   the skills repo and has its own version line there.

Both create their commit through the GitHub API, so GitHub signs it. A release
can take up to a day to reach the tap and the marketplace. To update sooner, run
the workflow by hand in each repo (`release` in the tap, `pin-hooks` in the
skills repo).

## One workflow, one tag prefix per tool

`.github/workflows/release.yml` releases every tool. It runs on a pushed tag
matching `v*`, `telemetry-v*` or `code-v*`. Its first job, `resolve`, maps the
prefix to the package, the binaries, the archive shape, the version file and the
Homebrew tool name, and fails on any other tag. `code-v*` is reserved and fails
until `crates/code` has a binary.

The jobs after `resolve` are shared. `verify-version` checks the tag against the
tool's `Cargo.toml` (and, for `v*`, the hooks plugin manifest). `create-release`
creates the GitHub release when it is missing, with `--latest=false` for every
prefix except `v`, so the repository's latest release stays on the playbook
line. `build` compiles the five targets, ad hoc signs the macOS binaries and
uploads one asset per target. `checksums` writes `SHA256SUMS` and attests the
assets. `plugin-archive` runs for `v*` only. The workflow only publishes. The tap's own
`release` workflow checks every 30 minutes and picks the new release up, so a
formula follows within about half an hour.

To rehearse, run the `release` workflow by hand with a `tag` input. A manual run
is always a dry run: it resolves, verifies and builds, and skips release
creation, uploads and attestation. The tag's version must match
the files on the ref you run it from.

## The plugin archive

A marketplace install used to clone the whole repository into the plugin cache
(423 files, 5.4 MB), of which a small part is plugin content. The release
attaches `playbook-hooks-plugin-<version>.zip` and the `playbook-hooks`
marketplace entry uses the `archive` source:

```json
{"source": "archive",
 "url": "https://github.com/pragmatic-engineer/playbook/releases/download/v<version>/playbook-hooks-plugin-<version>.zip",
 "sha256": "<hex>"}
```

Claude Code refuses a download whose hash does not match.

The `plugin-archive` job runs after `verify-version`. It builds the zip with
`git archive` from the tagged commit, limited to the pathspecs in
`.claude-plugin/archive-hooks-files.txt`. Only tracked files can appear, and every
entry carries the commit time, so the same commit gives the same bytes. The job
uploads the zip (`--clobber`) and attests it. This job and `checksums` hold
the `id-token` and `attestations` permissions. The zip is named so it cannot match the `checksums` job's
`playbook-<version>-*` download, which must find exactly the five binaries.

What the zip holds: the hooks plugin (`.claude-plugin/plugin.json`, renamed
from `hooks.plugin.json`, `hooks/hooks.json`, `LICENSE`, `README.md`), the
`bin/playbook` shim and `install.sh`, so a plugin-only install can bootstrap the
binary, and the files `playbook init` reads through `CLAUDE_PLUGIN_ROOT`
(`settings.shared.json`, `prompts/SYSTEM_PROMPT.md`, `Brewfile`). It holds no
`src/`, `tests/`, `crates/`, `docs/`, `.github/`, `Cargo.*` or `*.test.sh`. The
binary is not in it: it still comes from `install.sh`, Homebrew or
`playbook update`. The skills, commands and agents are not in it: they ship from
`pragmatic-engineer/skills`.

### Changing the allowlist

Edit `.claude-plugin/archive-hooks-files.txt`: one git pathspec per line, `#`
comments allowed, `:(exclude)` lines allowed. It is an allowlist, so a new
runtime file that is not listed is not shipped. `crates/cli/tests/plugin_archive.rs`
guards this. It builds the archive the way the workflow does and fails when a
tracked file under
`prompts/` or `hooks/` is missing, when a file the code reads through the plugin
root is missing, when the zip holds source, tests, docs, CI or Cargo files, when
it exceeds 1 MB, or when `playbook init` cannot run from the unpacked copy. The
test cuts the archive from the git index, so `git add` a new file before
running it.

### Minimum Claude Code version

The `archive` source needs Claude Code 2.1.224 or later. An older client cannot
use the entry, so the plugin cannot be installed from the marketplace until
Claude Code is upgraded. There is deliberately no second entry for old clients.

Claude Code keeps one cache folder per plugin version under
`~/.claude/plugins/cache/<marketplace>/playbook-hooks/`, so older versions stay on
disk beside the current one. The trimmed archive makes each one roughly
an eighth of the size.

End to end (a real marketplace install of the zip) can only be proven by a
tagged release.

## Internal tool crates

`playbook-telemetry` (and later `playbook-code`) are workspace members of this
repo under `crates/`. `playbook` depends on them by path, so there is one
`Cargo.lock` and no bump pull requests. Each tool still releases on its own
tag prefix and keeps its own version in its own `Cargo.toml`:

| Tool | Tag | Release assets |
| --- | --- | --- |
| playbook | `vX.Y.Z` | hooks plugin zip, five binaries, `SHA256SUMS` |
| playbook-telemetry | `telemetry-vX.Y.Z` | `playbook-telemetry-<version>-<target>.tar.gz` holding `playbook-telemetry` and `pbtop`, `SHA256SUMS` |
| playbook-code | `code-vX.Y.Z` | `playbook-code-<version>-<target>.tar.gz` holding `playbook-code` and `pbcode`, `SHA256SUMS` |

The Homebrew tap maps each formula to the repo and tag prefix, and the
marketplace pin follows plain `vX.Y.Z` tags only.
