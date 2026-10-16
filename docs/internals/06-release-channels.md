# Release channels

A release reaches users through three channels: the GitHub release assets
(built by the `build` and `publish` jobs of `release.yml`), the Homebrew tap
`pragmatic-engineer/homebrew-tap` and the plugin marketplace
`pragmatic-engineer/marketplace`. After a release is published and checked,
the `downstream` job of `release.yml` opens a pull request in the tap and in the
marketplace, as the section on the release workflow describes. The tap also
checks every 30 minutes for a newer release and bumps a formula itself when the
`downstream` job has not done it within an hour.

A release attaches fifteen assets (three per target), `release-notes.json`,
`breaking-changes.json` and `SHA256SUMS`, which lists the other seventeen. It
builds no plugin archive. The plugin in `plugins/playbook` ships with the same `vX.Y.Z`, as
described next.

## The plugin release

The binary and the plugin share one version line
([ADR 0038](../adr/0038-one-release-line.md)). The `version` in `plugin.json`
equals the workspace version in `Cargo.toml`, and its
`metadata.minBinaryVersion` is the `X.Y.0` of that minor line. The `version-sync` check in `plugin-guards.yml` fails a pull
request or a push to `main` when they differ, and the `plan` job of
`release.yml` checks them again before a `v` release. The release pull request
runs `scripts/bump-version.sh X.Y.Z`, which sets all of them and the
`PLAYBOOK_REF` pins together. A plugin change between releases needs no bump.

The marketplace repository `pragmatic-engineer/marketplace` lists the plugin as
a `git-subdir` source that points at `plugins/playbook` in this repository.
The `downstream` job pins its `ref` to the release tag `vX.Y.Z` and sets
`metadata.version` once the release is published. Claude Code copies only that
folder at the tag and updates an installed plugin when the `version` at the
new pin is higher. Until the first
release that runs `downstream`, the entry has no `ref` and follows the default
branch. There is no separate plugin release. The `playbook-plugin-v` tags made before 0.34.0 stay as history. The
`payload-guard` check keeps the folder to markdown and the manifest.

## The release workflow

`.github/workflows/release.yml` releases playbook ([ADR 0039](../adr/0039-release-workflow.md)).
A maintainer starts it from the Actions tab or with `gh`:

```bash
gh workflow run release.yml -f bump=patch          # or minor, or major
gh workflow run release.yml -f version=0.35.0
gh workflow run release.yml -f bump=minor -f dry_run=true
```

`plan` runs `scripts/release-plan.sh`, which reads the versions on `main` and
the `v` tags and picks where the run starts:

- `prepare`: the files are below the version. `scripts/release-prepare.sh`
  runs `scripts/bump-version.sh` and adds the `## playbook X.Y.Z` section to
  `CHANGELOG.md`. `scripts/release-gh.sh pr` opens the pull request
  `chore(release): playbook X.Y.Z` on `release/vX.Y.Z` with one commit that
  GitHub signs, turns on auto-merge, and the job waits for the merge queue to
  land it.
- `tag`: `main` carries the version and the tag is missing. The job creates
  the lightweight tag `vX.Y.Z` on the release commit with the igorjs app
  token, which the "Protect release tags" ruleset lets through.
- `publish`: the tag exists. The run builds and publishes, unless the release
  is already published, in which case it only checks it.

After the tag, `draft` creates a draft release with the notes and attaches
`release-notes.json` and `breaking-changes.json`. `build` compiles the five
targets with `--locked` and attaches three assets per target: the `playbook`
binary, `playbook-telemetry-X.Y.Z-<target>.tar.gz` with `playbook-telemetry`
and `pbtop`, and `pbtop-X.Y.Z-<target>.tar.gz` with `pbtop` alone. Each
build job first runs its binaries with `--version` (all but the cross built
x86_64 macOS one) and attests the assets it built. `publish` checks every
draft asset against that attestation, writes `SHA256SUMS` over every asset,
attests it and the notes, and makes the release public and the latest.
`verify` downloads the published release, checks the sums and the
attestations of a run from `main`, and runs `--version` on the x86_64 Linux
binaries.

Every step can run again. A pull request, a tag or a release that exists is
reused, a tag is never moved, and a published release is never rebuilt or
uploaded again. When a run fails, run the workflow again with
`version=X.Y.Z`. When GitHub refuses to create the tag, the run summary
prints the signed `git tag` command to run by hand, and the next run with
`version=X.Y.Z` goes on from the tag.

After `verify`, `downstream` moves the tap and the marketplace to the release
([ADR 0039](../adr/0039-release-workflow.md) decision 9). It runs the tap's
`.github/scripts/bump-formula.sh` for `playbook`, `pbtop` and
`playbook-telemetry`, which reads the release's `SHA256SUMS`, in a
`formulas` job with no secrets, and sets the marketplace `ref` to `vX.Y.Z`
and `metadata.version` to `X.Y.Z`. Each change
goes to a `release/vX.Y.Z` pull request titled `chore(release): playbook X.Y.Z`,
made with `scripts/release-gh.sh pr` and the app token, with auto-merge on.
When a pull request merges, a workflow in that repository tags the merge
commit `vX.Y.Z` and publishes a release. The job opens both pull requests
first, then waits up to 30 minutes for both merges, and the run summary links
them. When a repository has not published its release five minutes after its
merge, the job tags and releases it through the API, and only when the merge
commit changes nothing but the formulas or the marketplace file. When something here fails, the playbook release
stays published, the job fails, and the summary prints the commands to finish
it. Running the workflow again with `version=X.Y.Z` goes on from where it
stopped. When `BOT_APP_CLIENT_ID` or `BOT_APP_PRIVATE_KEY` is not set, the job
skips the downstream step with a notice, and the summary says to move the tap
and the marketplace by hand or to run again once the secrets are set.

A dry run plans, shows the diff of the release pull request in the run
summary, and builds the five targets from `main`. It also shows the tap and
marketplace diffs, with placeholder checksums for the formulas, and checks the
app tokens with read-only calls. It opens, tags, uploads and publishes
nothing.

`scripts/release-notes.sh` builds the body of the release from the
Conventional Commits since the previous `v` tag that touch the playbook
paths, the plugin included (ADR 0037). The sections are Breaking changes,
Features, Fixes, Performance and Other changes, in that order, and an empty one
is left out. The release commit itself is left out. The same notes go into
`release-notes.json`, which `SHA256SUMS` lists and the workflow attests. The
body starts with the upgrade note in `.github/release-notes-upgrade.md`. When
most users have upgraded from the `playbook-hooks` plugin, delete the file and
the line in the `body` mode of `scripts/release-notes.sh` that prints it.

`breaking-changes.json` is still attached, since the Homebrew tap and the
automatic update read it until the binary reads `release-notes.json`
(ADR 0037). No check fails a release for a breaking change: the notes list
the commits marked breaking first. Until the binary reads
`release-notes.json`, a breaking change to playbook also gets an entry in
`breaking-changes.json`, so automatic updates hold the release.

## Credentials

The workflow uses `GITHUB_TOKEN` to read, to build the draft and to publish,
and the igorjs GitHub App token for the release pull request, the tag and the
`downstream` pull requests. `BOT_APP_CLIENT_ID` and `BOT_APP_PRIVATE_KEY` are
organization secrets that the playbook, tap and marketplace repositories can
read. Each step asks the app for only the permissions it needs, and
`downstream` mints one token for each repository. The workflow runs only from
`workflow_dispatch`, anything but a dry run only from `main`, and `downstream`
only from `main`. The key can be read by any workflow in those three
repositories, so the guards are the rulesets and the checks in each workflow:
the downstream release workflows act only on a merged pull request from the
app on a `release/v*` branch. An organization admin rotates the key and
updates both secrets. Pull
requests made with the app token run CI, which pull requests made with
`GITHUB_TOKEN` do not. The app is on the bypass list of the "Protect release
tags" ruleset, so it can create `v*` tags. The tag is a lightweight ref on the
squash commit of the merge queue, which GitHub signs.

## Rolling back a release

A published release is not deleted, and its tag is not moved, because
installs and the tap may already point at them. To pull a bad release:

1. Mark it as a pre-release, so it stops being the latest:
   `gh release edit vX.Y.Z --prerelease --latest=false`, then mark the
   previous release as the latest with `gh release edit vA.B.C --latest`.
   `install.sh` and `playbook update` read the latest release.
2. Add a warning at the top of its notes, saying what is wrong and which
   version to use. `--notes-file` replaces the whole body, so start from the
   current notes: `gh release view vX.Y.Z --json body --jq .body > notes.md`,
   add the warning at the top of `notes.md`, then
   `gh release edit vX.Y.Z --notes-file notes.md`.
3. Move the plugin back: open a pull request in `pragmatic-engineer/marketplace`
   that sets the `ref` of the `playbook` entry to `vA.B.C` and
   `metadata.version` to `A.B.C`. New installs then get the previous plugin.
   Claude Code never installs a lower plugin version over an installed one,
   so users who already updated keep the bad plugin until the next patch.
4. Move the formulas back: in `pragmatic-engineer/homebrew-tap`, run
   `SOURCE_REPO=playbook TAG_PREFIX=v .github/scripts/bump-formula.sh A.B.C
   Formula/<formula>.rb` for `playbook`, `pbtop` and `playbook-telemetry`, and
   open a pull request with the result. The tap's scheduled check never moves
   a formula back to a lower version, and it skips a pre-release. Before
   0.34.0, `pbtop` and `playbook-telemetry` had no assets in the `v`
   releases: for such an A.B.C, move only `playbook` this way, take
   `playbook-telemetry` back to its last `telemetry-v` tag with
   `TAG_PREFIX=telemetry-v`, and remove `Formula/pbtop.rb` by hand.
5. Ship the fix as the next patch release, which moves the tap and the
   marketplace forward again.

## Internal tool crates

`playbook-telemetry` (and later `playbook-code`) are workspace members of this
repo under `crates/`. `playbook` depends on them by path, so there is one
`Cargo.lock`. `playbook-telemetry` uses the workspace version and ships in the
`vX.Y.Z` release (ADR 0038). The `telemetry-v` tags before 0.34.0 stay as
history.

| Tool | Release assets |
| --- | --- |
| playbook | `playbook-<version>-<target>`, the raw binary |
| playbook-telemetry | `playbook-telemetry-<version>-<target>.tar.gz` holding `playbook-telemetry` and `pbtop` |
| pbtop | `pbtop-<version>-<target>.tar.gz` holding `pbtop` alone |

Every release also carries `release-notes.json`, `breaking-changes.json` and
`SHA256SUMS`. The Homebrew tap maps each formula to its asset.
