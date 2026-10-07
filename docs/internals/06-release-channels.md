# Release channels

A tagged release reaches users through three channels: the GitHub release assets
(built by the `build` and `checksums` jobs), the Homebrew tap
`pragmatic-engineer/homebrew-tap`, and the plugin marketplace
`pragmatic-engineer/marketplace`. The `publish-channels` job in
`.github/workflows/release.yml` updates the last two after `checksums` succeeds.

What it does, on a tag push only and only when the tag is the repo's latest
release (so a backport tag cannot roll users back):

1. Renders `Formula/playbook.rb` from `src/release/formula.rb.tmpl` and the
   release's `SHA256SUMS` (`playbook release render-formula`), and pushes it to
   the tap. The job runs the release's own `x86_64-unknown-linux-musl` binary,
   downloaded from the release and checked against `SHA256SUMS` first.
2. Points the `playbook` entry of the marketplace's `marketplace.json` at the
   release's plugin archive, with the archive's `url` and `sha256`
   (`playbook release pin-marketplace`), and pushes it. It first re-downloads the
   archive and refuses to pin a hash that differs from the one the
   `plugin-archive` job computed.
3. Reads both repos back and fails with a separate `::error::` per check if the
   tap does not show the version, or the marketplace shows the wrong archive
   URL or the wrong `sha256`.

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
uploads the zip (`--clobber`), attests it, and passes its sha256 to
`publish-channels`. Only this job holds `id-token` and `attestations`
permissions. The zip is named so it cannot match the `checksums` job's
`playbook-<version>-*` download, which must find exactly the five binaries.

What the zip holds: the plugin content (`.claude-plugin/plugin.json`,
`commands/`, `skills/`, `agents/`, `output-styles/`, `hooks/hooks.json`,
`LICENSE`, `README.md`) and the files `playbook init` and the commands read
through `CLAUDE_PLUGIN_ROOT` (`settings.shared.json`, `statusline.sh`,
`prompts/SYSTEM_PROMPT.md`, `hooks/migration-check.sh`, `hooks/lib/config-hash.sh`,
`shell/setup-local.sh`,
`Brewfile`). It holds no `src/`, `tests/`,
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

## The secret

The workflow token cannot write to other repos, so the job needs the secret
`RELEASE_PUSH_TOKEN`: a fine-grained personal access token with
`contents: write` on both `homebrew-tap` and `marketplace`. Writes go
through the contents API, so GitHub signs the commits.

Store it as an environment secret on the `release` environment (Settings,
Environments, `release`), not as a repository secret. The job runs in that
environment, and the environment only accepts deployments from `v*` tags, so a
workflow on any other branch or tag cannot read the token.

Without the secret the job prints a warning and passes. The release is still
valid, but the tap and marketplace stay on the previous version until updated
by hand.
