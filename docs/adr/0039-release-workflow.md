# ADR-0039: A Release Workflow That Runs Every Step

- **Status:** Accepted
- **Date created:** 2026-10-16
- **Date modified:** 2026-10-16
- **Amends:** [ADR 0029](0029-cargo-workspace-monorepo.md) decision 6, which leaves the tap to poll for releases, [ADR 0037](0037-release-notes-from-conventional-commits.md) decision 2, which starts a release from a pushed tag, and the rollback consequence of [ADR 0038](0038-one-release-line.md), which moves the marketplace `ref` back.

## Context

A release took several manual steps: a pull request with the version bump, a signed tag pushed by hand, a workflow that built and published on that tag, and a Homebrew tap that noticed the release within half an hour. A step missed or done out of order left a release half done, and there was no single command to finish it. ADR 0038 joined the binary and the plugin on one version line, so every plugin fix now waits for a release, and releases must be cheap to cut.

The repository rules shape what a workflow can do. `main` takes changes only through a pull request and the merge queue, with signed commits. The "Protect release tags" ruleset restricts who creates a `v*` tag and requires signatures. A pull request or a push made with `GITHUB_TOKEN` starts no other workflow, so a release pull request made that way would never get its required checks. The igorjs GitHub App is installed on the organization, its credentials are the organization secrets `BOT_APP_CLIENT_ID` and `BOT_APP_PRIVATE_KEY`, visible to this repository, the marketplace and the tap, and the app is on the bypass list of the tag rulesets of this repository and the marketplace.

## Decision

1. **One entry point.** `release.yml` runs only from `workflow_dispatch`, with `bump` (patch, minor or major) or `version` (X.Y.Z), and `dry_run`. A pushed tag no longer starts it.
2. **The plan decides where to start.** `scripts/release-plan.sh` reads the versions on `main` and the `v` tags. Files below the version start at the release pull request, files at the version without a tag start at the tag, and an existing tag starts at the build. When the files carry a version above the last tag, that pending release is the only one the plan accepts. Only the latest release can be resumed, and no version is reused or goes backward.
3. **The release pull request.** `scripts/release-prepare.sh` runs `scripts/bump-version.sh` and adds the changelog section from `scripts/release-notes.sh`. `scripts/release-gh.sh pr` opens `chore(release): playbook X.Y.Z` with one commit made through `createCommitOnBranch`, which GitHub signs, and turns on auto-merge. The pull request is made with the app token, so CI runs on it, and it merges through the merge queue like any other. The run waits for the merge.
4. **The tag.** The run creates the lightweight tag `vX.Y.Z` on the merged commit with the app token. The squash commit of the merge queue is signed by GitHub. When GitHub refuses the tag, the run stops and prints the signed `git tag` command for a maintainer, and a new run with the version goes on from the tag.
5. **Draft first, public last.** The assets go to a draft release. Each build job runs its binaries with `--version` and attests the assets it built before it uploads them. `publish` checks every draft asset against that attestation, writes `SHA256SUMS` over every asset, attests it and the notes, and only then makes the release public and the latest. A `verify` job downloads the published release, checks the sums and the attestations of a run from `main`, and runs `--version` on the x86_64 Linux binaries.
6. **Telemetry ships in the same release.** Each of the five targets gives the `playbook` binary, `playbook-telemetry-X.Y.Z-<target>.tar.gz` with `playbook-telemetry` and `pbtop`, and `pbtop-X.Y.Z-<target>.tar.gz` with `pbtop` alone. `playbook-telemetry` uses the workspace version, as ADR 0038 decision 9 says.
7. **Every step can run again.** An open or merged release pull request is reused, a tag that points at the release commit is kept and one that points elsewhere stops the run, a draft is reused, and a published release is never rebuilt or uploaded again. One release runs at a time, and a running release is never cancelled.
8. **Determinism.** Actions are pinned by commit, builds use `--locked`, the toolchain comes from `rust-toolchain.toml`, and every job runs behind an egress allowlist on Linux. The notes are built from the history at the tag, so a rerun writes the same notes.
9. **Downstream repositories.** After `verify`, the release opens a pull request with auto-merge in `pragmatic-engineer/homebrew-tap` for the formulas and in `pragmatic-engineer/marketplace` for the plugin `ref`, waits for both to merge, and reports the links. Each of those repositories then tags and releases its own `vX.Y.Z`. This part comes with the change that follows this record.
10. **Rollback.** A published release is not deleted and its tag is not moved. A bad release is marked as a pre-release, the previous one becomes the latest again, its notes get a warning, and the fix ships as the next patch. The plugin only moves forward: Claude Code never installs a lower plugin version, so a plugin fix also ships as the next patch. The release doc lists the commands.

## Alternatives considered

- **Keep the pushed tag as the trigger.** Rejected. The tag had to be signed by a maintainer on the right commit after a manual pull request, which is the manual chain this record removes.
- **Use `GITHUB_TOKEN` for the release pull request.** Rejected. CI does not run on it, so the merge queue would never get its required checks.
- **A personal access token.** Rejected. The app already exists, its token expires within the hour, and its permissions can be narrowed for each step.
- **Publish the release first and upload the assets after.** Rejected. Users and the tap could read a release with missing assets or no `SHA256SUMS`.

## Consequences

- A release is one command, and a failed one is finished with the same command and the version.
- The release depends on the app secrets. Without them the run fails at the app token step of `prepare` or `tag`, and a maintainer finishes those steps by hand with the commands in the release doc, then runs the workflow again with the version.
- The app can create `v*` tags. Its key is an organization secret, so any workflow in the three repositories can read it. The guards are the rulesets, the narrow permissions each step asks for, and the checks in each workflow: the release runs only from `workflow_dispatch` on `main`, never from a pull request event, and, with the downstream change that follows this record, the release workflows of the tap and the marketplace act only on a merged pull request from the app on a `release/v*` branch. An organization admin rotates the key when the app's key changes.
- `telemetry-v` tags are no longer made. The tap maps `playbook-telemetry` to the `v` tags.
