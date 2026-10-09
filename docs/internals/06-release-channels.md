# Release channels

A tagged release reaches users through three channels: the GitHub release assets
(built by the `build` and `checksums` jobs), the Homebrew tap
`pragmatic-engineer/homebrew-tap`, and the plugin marketplace
`pragmatic-engineer/marketplace`. This repo does not push to the last two. Each
of those repos runs its own daily `bump` workflow that reads this repo's latest
release and updates itself:

1. The tap rewrites the URLs and checksums in `Formula/playbook.rb` from the
   release's `SHA256SUMS`, then audits, installs and tests the formula.
2. The marketplace points the `playbook` entry at the release's plugin archive,
   with the archive's `url` and `sha256`. It verifies the archive's build
   provenance attestation first.

Both create their commit through the GitHub API, so GitHub signs it. A release
can take up to a day to reach the tap and the marketplace. To update sooner, run
the `bump` workflow by hand in each repo.

## The plugin archive

A marketplace install used to clone the whole repository into the plugin cache
(423 files, 5.4 MB), of which about 0.66 MB is plugin content. The release now
attaches `playbook-plugin-<version>.zip` and the marketplace entry uses the
`archive` source:

```json
{"source": "archive",
 "url": "https://github.com/pragmatic-engineer/playbook/releases/download/v<version>/playbook-plugin-<version>.zip",
 "sha256": "<hex>"}
```

Claude Code refuses a download whose hash does not match.

The `plugin-archive` job runs after `verify-version`. It builds the zip with
`git archive` from the tagged commit, limited to the pathspecs in
`.claude-plugin/archive-files.txt`. Only tracked files can appear, and every
entry carries the commit time, so the same commit gives the same bytes. The job
uploads the zip (`--clobber`) and attests it. This job and `checksums` hold
the `id-token` and `attestations` permissions. The zip is named so it cannot match the `checksums` job's
`playbook-<version>-*` download, which must find exactly the five binaries.

What the zip holds: the plugin content (`.claude-plugin/plugin.json`,
`commands/`, `skills/`, `agents/`, `output-styles/`, `hooks/hooks.json`,
`LICENSE`, `README.md`) and the files `playbook init` and the commands read
through `CLAUDE_PLUGIN_ROOT` (`settings.shared.json`,
`prompts/SYSTEM_PROMPT.md`, `Brewfile`). The plugin also ships `bin/playbook` and `install.sh`, so a
plugin-only install can bootstrap the binary. It holds no `src/`, `tests/`,
`docs/`, `.github/`, `Cargo.*` or `*.test.sh`. The binary is not in it: it
still comes from `install.sh`, Homebrew or `playbook update`.

### Changing the allowlist

Edit `.claude-plugin/archive-files.txt`: one git pathspec per line, `#`
comments allowed, `:(exclude)` lines allowed. It is an allowlist, so a new
runtime file that is not listed is not shipped. `tests/plugin_archive.rs`
guards this. It builds the archive the way the workflow does and fails when a
tracked file under `commands/`, `skills/`, `agents/`, `output-styles/`,
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
`~/.claude/plugins/cache/<marketplace>/playbook/`, so older versions stay on
disk beside the current one. The trimmed archive makes each one roughly
an eighth of the size.

End to end (a real marketplace install of the zip) can only be proven by a
tagged release.
