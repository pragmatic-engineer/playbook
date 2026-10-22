# Release channels

A release reaches users through three channels: the GitHub release assets
(built by the `build` and `publish` jobs of `release.yml`), the Homebrew tap
`pragmatic-engineer/homebrew-tap` and the plugin marketplace
`pragmatic-engineer/marketplace`. After a release is published and checked,
the `downstream` job of `release.yml` opens a pull request in the tap and in the
marketplace, as the section on the release workflow describes. The tap also
checks every 30 minutes for a newer release and bumps a formula itself when the
`downstream` job has not done it within an hour.

A release attaches ten assets (two per target), four Linux packages,
`release-notes.json` and `SHA256SUMS`, which lists the other fifteen. It
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

Before the tag, `macos` calls `rust-ci.yml` to build and test the release
commit on macOS. Pull requests and the merge queue test on Linux only, so this
is the one macOS check every release must pass. It tests the release commit,
or the tag when the run starts at `publish`. A dry run that starts at
`prepare` tests the dispatch commit. `tag` and `draft` stop the run unless it
passed on that same commit. The release pull request also builds and tests on
macOS, so a macOS break stops the bump before it reaches `main`. When the gate
still fails on the release commit, land the fix, tag `vX.Y.Z` on the fixed
commit by hand, and run the workflow again with `version=X.Y.Z`: it starts at
`publish` and tests the tag.

After the tag, `draft` creates a draft release with the notes and attaches
`release-notes.json`. `build` compiles the five
targets with `--locked` and attaches two assets per target: the `playbook`
binary and `pbtop-X.Y.Z-<target>.tar.gz` with `pbtop` alone. No release
carries the `playbook-telemetry` archive, which
[ADR 0041](../adr/0041-retire-playbook-telemetry.md) retires. Each
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
`.github/scripts/bump-formula.sh` for `playbook` and `pbtop`, which reads
the release's `SHA256SUMS`, in a
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

When `downstream` has moved the tap (its `moved` output, not set when the
app secrets are missing), `verify-homebrew-linux` runs `brew tap` and
`brew install` for `playbook` and `pbtop` on an Ubuntu runner, checks that
both report the release version, and runs `playbook doctor check` in an empty
`HOME`. It blocks no other job. A red result fails the run, and running again
with `version=X.Y.Z` starts it again. A dry run skips it, because nothing is
published to install.

After `verify`, the `nix` job moves the Nix flake (root `flake.nix`). It
checks `SHA256SUMS` and the four Nix assets against their attestation, then
`scripts/render-flake.sh` rewrites the `releases` block (the version and one
SRI hash per system), and the job opens `release/nix-vX.Y.Z` titled
`chore(release): nix flake X.Y.Z` with the app token, auto-merge on, and waits
up to 30 minutes for the merge. `scripts/release-notes.sh` leaves that
commit out of the next notes. The job skips when the release is not the
latest or `flake.nix` is already ahead, so a backport does not move the flake
back. Without the app secrets it warns and passes. If `main` moved between the
checkout and the commit, `release-gh.sh` fails with "Run again": run the
workflow with `version=X.Y.Z`. Then `verify-nix-linux` and `verify-nix-macos`
run `nix run github:pragmatic-engineer/playbook/<commit>#playbook --
--version` at the commit that carries the flake and compare it with the
version; the Linux job also builds a copy
with a wrong hash, which must fail. Both are leaf jobs, and they are skipped in a dry run and when the `nix`
job moved nothing. A dry run of `nix` renders from a stand-in `SHA256SUMS`, shows the diff
and opens nothing. The tag `vX.Y.Z` carries the flake of the release before
it, so users pin `main` or the merge commit of the flake pull request.
To roll the flake back, open a pull request that reverts the flake change:
the older hashes stay valid because release assets do not change.

After `verify`, the `aur` job publishes the Arch package `playbook-bin` to the
AUR. `packaging/aur/PKGBUILD.tmpl` is the template, and
`scripts/render-pkgbuild.sh` fills it with the version and the two Linux
`playbook-X.Y.Z-<target>` lines of `SHA256SUMS`. Before it renders, the job
checks `SHA256SUMS` and the two Linux binaries against their attestation, so
the AUR carries the attested hashes. It keeps the `pkgrel` and `epoch` of the
AUR when the `pkgver` is the same, so a manual fix is not overwritten, and a
new `pkgver` gives `pkgrel=1`. `scripts/aur-verify.sh` runs in the
`archlinux:base-devel` container (pinned by digest in `release.yml`): it makes
`.SRCINFO` with `makepkg --printsrcinfo` as a non-root user, builds with
`makepkg -f`, installs with `pacman -U`, compares `playbook --version` with the
release, and downloads the aarch64 asset to check its sum. A failure stops the job before
anything is pushed. Then the job commits `playbook-bin: X.Y.Z` and pushes
`PKGBUILD` and `.SRCINFO` to `ssh://aur@aur.archlinux.org/playbook-bin.git` on
`master`, the only branch the AUR takes, and builds the live repository from
`https://aur.archlinux.org/playbook-bin.git`, with up to five tries for the
AUR to catch up. No change means no commit, so a run with the same version
pushes nothing. The SSH host key is pinned in `packaging/aur/known_hosts`
with `StrictHostKeyChecking=yes`; the fingerprints are on
https://aur.archlinux.org/ and `scripts/aur-verify.test.sh` checks them. A
changed host key fails the job: refresh the file from that page. The key file
is written to `RUNNER_TEMP` with mode 600 and removed in an `always()` step.
The job is a leaf and a backport or an older release never rolls the AUR back.
A dry run builds the newest published release from its real assets, shows the
diff and pushes nothing. Without the secret `AUR_SSH_PRIVATE_KEY` in the `aur`
environment, the job builds and checks the package, skips the push with a
notice and passes.

One-time setup, by a maintainer. The workflow cannot do these:

1. Create a dedicated bot account on https://aur.archlinux.org/. An AUR SSH
   key reaches every package of its account, so do not use a personal account
   that owns other packages.
2. Check that `playbook-bin` is free (https://aur.archlinux.org/packages/playbook-bin).
   The first push creates it.
3. Make a key pair with `ssh-keygen -t ed25519 -f aur_key -C playbook-release`,
   add `aur_key.pub` to the bot account under My Account, SSH Public Key, and
   store the private key as the secret `AUR_SSH_PRIVATE_KEY` of a new GitHub
   environment `aur` that deploys from `main` only. Delete the local key files.
4. Add a personal AUR account as co-maintainer of `playbook-bin`, so a
   rollback does not depend on CI.
5. Rotate by adding a new public key to the bot account, replacing the secret,
   then removing the old public key.

To fix a bad push, clone `ssh://aur@aur.archlinux.org/playbook-bin.git`, commit
a correction and push to `master`. A fix at the same version raises `pkgrel`.
A real rollback to an older `pkgver` looks like a downgrade to pacman, so raise
`epoch` in the same commit; the template keeps an `epoch` that is there. A run
with `version=` of an older release does not apply, because the job only
publishes the latest release. Orphaning or deleting the package goes through
the AUR web page.

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
`release-notes.json`, which `SHA256SUMS` lists and the workflow attests.

No check fails a release for a breaking change: the notes list the commits
marked breaking first. Releases no longer carry `breaking-changes.json`
(ADR 0047). A 0.34 binary, the last that read it, treats the missing asset
as a release with no breaking changes.

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
   Formula/<formula>.rb` for `playbook` and `pbtop`, and open a pull
   request with the result. The tap's scheduled check never moves
   a formula back to a lower version, and it skips a pre-release. Before
   0.35.0 the tap had no `pbtop` formula: for such an A.B.C, move only
   `playbook` this way and remove `Formula/pbtop.rb` by hand. The retired
   `playbook-telemetry` formula stays on `telemetry-v0.2.0`; when
   `pbtop.rb` goes, also remove its `disable!` line, since it names `pbtop`
   as the replacement.
5. Ship the fix as the next patch release, which moves the tap and the
   marketplace forward again.

## Internal tool crates

`playbook-telemetry` and `playbook-code` are workspace members of this
repo under `crates/`. `playbook` depends on them by path, so there is one
`Cargo.lock`. `playbook-code` is a library of the `playbook` binary and has
no release of its own. `playbook-telemetry` uses the workspace version, and its
`pbtop` binary ships in the `vX.Y.Z` release (ADR 0038, ADR 0041). The
`telemetry-v` tags before 0.34.0 stay as history.

| Tool | Release assets |
| --- | --- |
| playbook | `playbook-<version>-<target>`, the raw binary |
| pbtop | `pbtop-<version>-<target>.tar.gz` holding `pbtop` alone |
| pbdoc | `pbdoc-<version>-<target>.tar.gz` holding `pbdoc` and `NOTICE-pbdoc`, for the four macOS and Linux targets |
| Debian, Ubuntu | `playbook_<version>_amd64.deb` and `playbook_<version>_arm64.deb` |
| Fedora, RHEL | `playbook-<version>.x86_64.rpm` and `playbook-<version>.aarch64.rpm` |

Each package holds `playbook`, the `pb` link and `pbtop` in `/usr/bin`, and the
license. It has no dependencies, no config file and no install or remove script,
so an upgrade or a removal never touches user config. `scripts/nfpm.yaml`
describes it and `scripts/package-linux.sh` builds it with a pinned nfpm
(version and SHA-256 are in the script; bump both together). The two Linux
build legs build the packages from their own static binaries, install them in
Debian, Ubuntu and Fedora containers with `scripts/package-smoke.sh`, and attest
them with the other assets, so a bad package stops the release before the
draft is published. The `publish` job puts them in `SHA256SUMS` and expects 18
assets. After `verify`, the `verify-packages` job downloads the x86_64 pair
from the published release, checks the sums and the attestation, and runs the
same container test. It blocks no other channel, a red result fails the run, and
a run with the same version starts it again. A dry run builds and tests the
packages and publishes nothing. The container images are pinned by digest in
`scripts/package-smoke.sh`.

Every release also carries `release-notes.json` and `SHA256SUMS`. The Homebrew tap maps each formula to its asset.

Only `install.sh` and Homebrew ship `pbdoc`, the document converter. The `.deb`
and `.rpm` packages, `cargo binstall`, mise and the nix flake install
`playbook` without it, and `playbook doc` then reports that no converter is
installed. This is a known limit.

Release checklist for the release that adds `pbdoc`: the tap pull request that
adds `resource "pbdoc"` to `Formula/playbook.rb` (homebrew-tap #53) merges
first, and the release workflow and the installer changes ship together. The
`downstream` job fails when the tap formula has no `resource "pbdoc"`, so a tag
cut before that pull request merges cannot ship silently.

`cargo binstall` and mise install the raw `playbook-<version>-<target>` binary
and depend on that name. `[package.metadata.binstall]` in `crates/cli/Cargo.toml`
holds the URL template, and mise needs nothing, since it matches the operating
system and architecture words in the asset name. `scripts/asset-names.test.sh`
expands the template for every target of the build matrix and fails when it
differs from the name `release.yml` gives the asset. After `verify`, the
`verify-installers` job runs `scripts/verify-installers.sh`: pinned cargo-binstall
and mise install the published release in an empty `HOME`, `cargo binstall` may
not compile, and the installed file must report the version, match its
`SHA256SUMS` line and verify against its attestation. It blocks no other
channel, a red result fails the run, and a run with the same version starts it
again. A dry run installs the last released version instead, which checks the
job and its egress list without publishing anything. The crates.io name
`playbook` belongs to another project, so the documented command uses `--git`
and the metadata disables the quick-install strategy.
