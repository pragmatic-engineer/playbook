# ADR-0038: One Release Line for the Binary and the Plugin

- **Status:** Accepted
- **Date created:** 2026-10-16
- **Date modified:** 2026-10-16
- **Supersedes:** decisions 3 and 5 of [ADR 0032](0032-plugin-in-the-playbook-repo.md), the plugin's own version line and its own release workflow.
- **Amends:** [ADR 0034](0034-self-healing-upgrades.md) decisions 9, 10 and 11, the supported plugin range and `minBinaryVersion`. [ADR 0029](0029-cargo-workspace-monorepo.md) decisions 4 and 5, the `telemetry-v` tags and the separate telemetry version. [ADR 0037](0037-release-notes-from-conventional-commits.md) decisions 2 and 5.6, the plugin release page and notes.

## Context

ADR 0032 gave the plugin its own version line and its own release. The binary stayed at 0.26 while the plugin moved from 0.27.2 to 0.32.0. A user who saw "playbook 0.26" and "plugin 0.31" could not tell which pairs work together. ADR 0034 then needed a constant in the binary (the oldest plugin it supports), a field in the plugin (the oldest binary it supports), a doctor row and a migration to explain the gap. Every plugin change also needed a version bump and a changelog section, checked by a guard, even when nothing else was ready to ship.

The binary and the plugin live in one repository and are reviewed together. They should be released together.

## Decision

1. **One version.** A `vX.Y.Z` release ships the binary and the plugin. The `version` in `plugins/playbook/.claude-plugin/plugin.json` equals the `[workspace.package] version` in `Cargo.toml`, which the `playbook` and `playbook-code` crates use. `metadata.minBinaryVersion` in `plugin.json` equals the same version, so a plugin asks for the binary of its own release.
2. **One check.** `scripts/check-plugin.sh version` fails when the three values differ. The `version-sync` job in `plugin-guards.yml` runs it on every pull request that changes the plugin or `Cargo.toml`, and on every push to `main`. The `verify-version` job of `release.yml` runs it before a `v` release.
3. **One bump.** `scripts/bump-version.sh X.Y.Z` writes the version to `Cargo.toml`, to the `playbook` and `playbook-code` entries of `Cargo.lock`, to `version` and `metadata.minBinaryVersion` in `plugin.json`, and to the `PLAYBOOK_REF` pins in `README.md` and `docs/guides/00-install.md`. The release pull request runs it. A change under `plugins/playbook` between releases needs no bump and no changelog section.
4. **No plugin release.** `plugin-release.yml` is removed, and no new `playbook-plugin-v` tags are made. The existing tags and releases stay as history. The marketplace entry does not change: it reads `plugin.json` from `main`, as ADR 0032 decision 2 says.
5. **Plugin fixes reach users with the next release.** Claude Code updates an installed plugin only when its version rises, and the version rises only in a release. A plugin fix that merges between releases waits for the next one. Patch releases should therefore be cheap. The release workflow is meant to run from `workflow_dispatch`, with a dry run that builds and checks everything without publishing.
6. **Versions never go backward.** Claude Code does not install a plugin whose version is lower than the one it has. The plugin was at 0.32.0 when this record was accepted and the binary at 0.26, so the first joint release is 0.32.0. The binary skips 0.27 to 0.31, and the release notes of 0.32.0 say so. `scripts/bump-version.sh` refuses a version that is not above the binary version or that is below the plugin version.
7. **Compatible means the same minor line.** A plugin works with a binary that has the same MAJOR.MINOR version. The doctor row of layer 6 and migration `0013-plugin-version-range` compare the plugin with the running binary this way, in place of the minimum constant of ADR 0034 decision 9. The session start line from ADR 0034 decision 10 stays: when the binary is older than `minBinaryVersion`, it names the upgrade command.
8. **One changelog.** `CHANGELOG.md` covers the binary and the plugin. It keeps the plugin sections written so far. From 0.32.0, each section is named `## playbook X.Y.Z` and holds the notes that `scripts/release-notes.sh` builds for that release, added by the release pull request.
9. **Telemetry joins the line.** `playbook-telemetry` and its alias `pbtop` ship as assets of the `vX.Y.Z` release, with the same version, the same `SHA256SUMS` and the same attestation. `pbtop` also gets an archive of its own, so it can be installed alone. The Homebrew formula keeps the name `playbook-telemetry`, and the tap maps it to the `v` tags. No new `telemetry-v` tags are made after the release workflow change that does this. Until that change merges, `telemetry-v` tags work as before. An install at `playbook-telemetry` 0.2.0 sees 0.32.0 as an upgrade. The collector runs under the command names `playbook usage collect` and `playbook-telemetry collect`, which do not depend on a version, so starting it does not change.

## Alternatives considered

- **Keep two lines and check a range.** Rejected. It is what ADR 0034 built, and it still leaves users to work out which versions go together.
- **Start the joint line at the binary version.** Rejected. The plugin would go back from 0.32.0, and Claude Code would not install it.
- **Pin the marketplace entry to a tag.** Not chosen here. It would hold plugin changes on `main` back until the release is published, but every release would need a commit to the marketplace repository. The release workflow decides this later.
- **Keep `telemetry-v` tags.** Rejected. Two release pages and two versions for one repository bring back the problem this record solves.

## Consequences

- Users see one version for everything playbook ships, and `playbook --version` matches the plugin they run.
- A plugin change merges without a version bump. It reaches installed plugins with the next release. A fresh install between releases copies `main`, so two installs that both say X.Y.Z can differ in plugin text until the next release.
- `plugin.json` on `main` names the new version as soon as the release pull request merges, a short while before the binary assets are published. In that window, the session start line may ask for a binary that is not yet published.
- The release pull request is the one place that changes versions. The `version-bump` guard and its changelog rule are gone. The `payload-guard` and `intents-contract` checks stay.
- The constant `MIN_PLUGIN_VERSION` and the major version check in the binary go away in favor of the minor line rule.
