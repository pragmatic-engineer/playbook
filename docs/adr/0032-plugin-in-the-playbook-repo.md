# ADR-0032: Keep the Plugin in the Playbook Repository

- **Status:** Accepted. Decisions 3 and 5 are superseded by [ADR 0038](0038-one-release-line.md): the plugin shares the binary's version and ships in its `vX.Y.Z` release. Decision 5 was first amended by [ADR 0037](0037-release-notes-from-conventional-commits.md).
- **Date created:** 2026-10-14
- **Date modified:** 2026-10-16
- **Supersedes:** decisions 1, 5, 6 and 7 of [ADR 0030](0030-skills-and-commands-in-their-own-repo.md): the second repository, the daily comparison, the checks crate there and the version line

## Context

ADR 0030 moved the skills, commands and agents into a second repository so that prose edits would not need a binary release. It also kept a hooks plugin in this repository, pinned from the marketplace by a release archive and a hash.

Two things changed. ADR 0031 moved the hooks into `settings.json`, so the marketplace no longer pins an archive. And the second repository cost more than it saved: two sets of reviews, a daily workflow to compare the plugin with the binary, links across repositories, and a contract that two people had to keep in step.

Claude Code can install a plugin from a folder inside a git repository (a `git-subdir` source). With no `ref` it follows the default branch, it copies only that folder into its cache, and it updates the plugin when the `version` in `plugin.json` changes.

## Decision

1. **The plugin lives in this repository** under `plugins/playbook`. The folder holds `.claude-plugin/plugin.json` and markdown: `agents/`, `commands/`, `skills/` (with their `references/`) and `output-styles/`. Nothing else ships.
2. **The marketplace is a small repository.** `pragmatic-engineer/marketplace` (the former `pragmatic-engineer/skills`, renamed) holds `marketplace.json`, a README and CI. Its `playbook` entry is a `git-subdir` source with the url `https://github.com/pragmatic-engineer/playbook.git` and the path `plugins/playbook`. It sets no `ref` and no `version`, so the entry follows the default branch and the plugin's own `version` decides when users get an update.
3. **The plugin has its own version line**, in `plugin.json`, starting at 0.27.2. It is separate from the binary version.
   - A change to any file under `plugins/playbook` needs a higher version. A patch is a wording fix or a repair. A minor version is new or changed behavior of a command, skill or agent. A major version is a removed or renamed command, or a change to the contract with the binary.
   - `CHANGELOG.md` starts with a `## playbook <version>` section that matches `plugin.json`.
   - The `version-bump` check enforces both.
4. **The payload is markdown only.** The `payload-guard` check fails on any other file type, a symlink, or markdown nested deeper than the layout allows.
5. **The plugin is released by its own workflow.** When `plugin.json` or `CHANGELOG.md` changes on `main`, `plugin-release.yml` creates the tag `playbook-plugin-v<version>` and a GitHub release with the changelog section, unless the tag exists. The release is not marked latest, and the tag does not match the `v*` pattern of the binary release workflow.
6. **The contract with the binary stays a list.** `intents.json` names the tasks the plugin expects the binary to offer. The `intents-contract` check builds the binary from the same commit and compares the list with `playbook tools map`. No intent is required, so a missing one is a warning.
7. **The prose checks run here.** They live in the `playbook-plugin-checks` crate and read `plugins/playbook`.
8. **Users keep the same ids.** They run `claude plugin marketplace add pragmatic-engineer/marketplace` and `claude plugin install playbook@pragmatic-engineer`. The old repository name keeps working through the GitHub redirect.

## Alternatives considered

- **Keep two repositories.** Rejected for the costs in the context.
- **A `ref` or a `version` in the marketplace entry.** Rejected: either one needs a marketplace change for every plugin release.
- **A release archive for the plugin.** Rejected: the `git-subdir` source already copies only the folder, so an archive adds a build, a hash and a pin for nothing.

## Consequences

- One review and one history for the binary and the plugin.
- A change to the plugin reaches users after the version bump merges, not before. A change without a bump reaches nobody, and the `version-bump` check catches it.
- The first run of `plugin-release.yml` happens at the next change to `plugin.json` or `CHANGELOG.md`.
- Commit hashes in the old skills repository stay valid there. The plugin history before the move is read in that repository, now named `marketplace`.
