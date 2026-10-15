# ADR-0035: Breaking Change Notices on Every Update Channel

- **Status:** Accepted
- **Date created:** 2026-10-15
- **Date modified:** 2026-10-15
- **Extends:** [ADR 0034](0034-self-healing-upgrades.md), whose migrations fix what playbook owns after an upgrade. This record tells the user about the changes they need to know of.

## Context

Playbook reaches users through five update channels:

- `brew upgrade`, with the formulas `playbook` and `playbook-telemetry` in the tap `pragmatic-engineer/homebrew-tap`;
- `playbook update`;
- the install script, with `PLAYBOOK_REF` for a given release;
- a manual download from the GitHub release page;
- the plugin's own update through the marketplace, on the plugin version line.

No channel tells the user when a release needs a step from them. Release 0.26.0 is an example. Plugin 0.28 and later need a 0.26 binary, and a 0.25 binary cannot find the plugin files. Users learned this from an error. The release page has a hand written upgrade note, Homebrew shows nothing, and the plugin updates without a word.

## Decision

1. **One record.** `breaking-changes.json` at the repository root lists every breaking change. The file is an object with one key, `entries`, a list of objects with these fields:

   | Field | Meaning |
   |---|---|
   | `version` | The release that brings the change, as MAJOR.MINOR.PATCH. |
   | `component` | `playbook`, `plugin` or `telemetry`. Each has its own version line. |
   | `summary` | One plain line that says what changed. |
   | `action` | The exact command or step the user runs, or `none, migrated automatically`. |
   | `migration` | The id of the ADR 0034 migration that handles the change, or `null`. |

   The binary embeds the file with `include_str!`. A unit test parses the embedded copy, checks each field, and checks each migration id against the registry. `crates/cli/src/breaking.rs` holds the reader. It refuses a summary, action or migration id that holds a control character, `\`, `"`, a backtick, `$`, `<`, `>` or `#{`, so the text is safe in a terminal, a Ruby string and markdown. An action of `none, migrated automatically` must name its migration.
2. **What counts as breaking.** A change is breaking when a user who upgrades has to do something, or when something they use stops working or works in a new way. A migration that runs on first use still gets an entry, with the action `none, migrated automatically`. A commit marks a breaking change with `!` after its type or a `BREAKING CHANGE:` footer, as Conventional Commits 1.0.0 defines.
3. **Guards.**
   1. The `verify-version` job of `release.yml` lists the commits since the previous tag of the component that touch its paths. When one of them has a breaking marker, the file must hold an entry for that component and version, or the release fails before any build.
   2. The `version-bump` job of `plugin-guards.yml` does the same for the plugin when the version in `plugin.json` rises. The pull request title and body count as a commit, since they become the squash commit. The new version's `CHANGELOG.md` section must also have a `### Breaking` heading. The workflow also runs when `breaking-changes.json` changes.
   3. A light pull request check fails when the title has `!` or the body has a `BREAKING CHANGE:` footer, unless the file holds an entry for a component the pull request touches with a version above that component's latest tag.
4. **Delivery on every channel.**
   1. *Release page.* `create-release` puts a `Breaking changes` section built from the file at the top of the release body, above the upgrade note. Every `v*` and `telemetry-v*` release attaches `breaking-changes.json` as an asset.
   2. *`playbook update`.* The update asks the downloaded binary for the entries between the running version and its own with `playbook breaking --since <version>`. It prints them before the swap, and again after the swap and the heal, also when the heal fails. A target without the command is older than the running binary, so it has no entries to show.
   3. *First run after any upgrade.* `breaking.rs` exposes `entries_between(from, to)` and `render(entries)`. Every step that runs the ADR 0034 migrations shows the block for the versions the user crossed. The `session-init` hook adds it to the session message, which is how that hook reaches the user. `playbook migrate`, `playbook doctor --fix` and `playbook init` print it to stderr. This covers Homebrew, the install script and a manual download alike.

      The notice keeps its own record in the `state` table: `notices/binary` and `notices/plugin`, the last versions whose block was shown. A step writes them right after it shows the block, in their own transaction, whether the migrations succeed or not. So the block shows once, even when a heal fails or another trigger moves `versions/binary` first. The starting version is `notices/binary`, else `versions/binary`. When both are missing and the `state` table holds applied migrations, an older binary ran on this machine, since 0.26 and earlier write no version record. The block then starts from `0.0.0` and lists every entry up to the running version, once. On a fresh install, with no applied migrations, the step writes the record and shows nothing. `entries_between` returns nothing for a missing or corrupt version, so a bad record never shows the whole history.
   4. *Homebrew.* The tap's bump workflow downloads the asset. When the release has entries for the formula's component, it writes a `caveats` block that lists them and names `playbook breaking --since <version>` for older ones. Each line goes into a single quoted Ruby string, so nothing in it is interpolated. The next release without entries removes the block. `brew upgrade` and `brew info` show caveats.
   5. *Install script.* After it installs a binary over an older one, the script runs the new binary's `playbook breaking --since <old version>`, then `playbook migrate` when a terminal or `--yes` allows it.
   6. *Plugin.* The marketplace copies only `plugins/playbook`, and the payload guard allows only markdown and `plugin.json` there. So `plugin.json` mirrors the plugin entries under `metadata.breakingChanges`, and `scripts/check-plugin.sh` fails when the mirror differs from the plugin entries in the file. The `CHANGELOG.md` section of a breaking plugin release has a `### Breaking` heading. The first run block also reads `metadata.breakingChanges` from the resolved plugin's `plugin.json` with `breaking::lenient`, which skips fields and components an older binary does not know. It lists the plugin entries between `notices/plugin` and the resolved plugin version. The hook checks this on every session, since the plugin updates on its own while the binary stays the same.
5. **Compatibility policy.** Every release is backward compatible, or ships a migration that runs on first use. A release that changes what a user sees or does lists the change in the file, also when a migration handles it. `CONTRIBUTING.md` states this rule.

## Alternatives considered

- **Breaking notes only in markdown changelogs.** Rejected: the binary and the tap would have to parse prose, and the binary has no changelog of its own.
- **A `breaking-changes.json` in the plugin folder.** Rejected for now: the payload guard allows only markdown and the manifest, and a new file type would ship to every user. `plugin.json` already carries `metadata`, which Claude Code accepts.
- **Fetch the record from GitHub at run time.** Rejected: the session hook must work offline and stay fast, and the binary already embeds the record for its own version.
- **List every entry since the formula's previous version in the caveat.** Rejected: the tap does not know which version a user had. The caveat lists the release's own entries and names the command that shows older ones.

## Consequences

- One file drives the release page, the tap, the binary and the plugin guard. A missing entry fails a release, so a breaking change cannot ship without one.
- A user sees the notice whatever channel they use, at the latest on the first session after the upgrade.
- A binary released before this change has no `breaking` command, so `playbook update` from it shows nothing extra. Its users see the 0.26.0 entry on the first run of the new binary, through the missing record rule in decision 4.3.
- The plugin entries live in two places, kept equal by a guard. A plugin release with a breaking change edits both in the same pull request.
- `playbook-telemetry` has no first run step. Its entries reach users through the release page and the Homebrew caveat.
