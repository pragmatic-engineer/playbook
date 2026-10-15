# ADR-0037: Release Notes Built from Conventional Commits

- **Status:** Accepted
- **Date created:** 2026-10-16
- **Date modified:** 2026-10-16
- **Supersedes:** [ADR 0035](0035-breaking-change-notices.md).
- **Amends:** [ADR 0034](0034-self-healing-upgrades.md) decisions 13 and 14, which show and require the ADR 0035 record, and [ADR 0036](0036-release-check-and-automatic-updates.md) decisions 6 and 7.7, which read it during an automatic update.

## Context

ADR 0035 asked for a hand written entry in `breaking-changes.json` for every breaking change. Three guards failed a pull request, a plugin version bump and a release when a marked commit had no entry. The plugin entries had a copy in `plugin.json` with a guard of its own, and an environment variable passed the state of the notice between the installers and the binary.

Playbook is before 1.0, so a breaking change can ship in any release. The user wants a heads up when one does. A hard gate on each marked commit is more than that needs. The commits already hold the facts. The project uses Conventional Commits, and a squash merge turns the pull request title and body into the commit message, so a `!` in the title or a `BREAKING CHANGE:` footer in the body reaches the history of `main`.

## Decision

1. **A generator.** `scripts/release-notes.sh` builds the notes of one release from the commit subjects. It uses `git` and `jq`, which every runner has.
   1. *Which commits.* The commits after the previous tag of the same component, up to `HEAD`, that touch the paths the component is built from. Merge commits are left out. The previous tag is the highest tag of the component below the version.

      | Component | Tag | Paths |
      |---|---|---|
      | `playbook` | `vX.Y.Z` | `crates` except `crates/telemetry` and `crates/plugin-checks`, the root `Cargo.toml`, `install.sh`, `settings.shared.json`, `prompts`, `Brewfile` |
      | `telemetry` | `telemetry-vX.Y.Z` | `crates/telemetry`, `crates/data`, `crates/common` |
      | `plugin` | `playbook-plugin-vX.Y.Z` | `plugins/playbook` |

      `Cargo.lock` counts for none, since every dependency bump rewrites it.
   2. *Sections.* The notes have these sections, in this order. An empty section is left out.

      | Section | Commits |
      |---|---|
      | Breaking changes | `!` after the type or scope, or a `BREAKING CHANGE:` or `BREAKING-CHANGE:` footer |
      | Features | `feat` |
      | Fixes | `fix` |
      | Performance | `perf` |
      | Other changes | every other type, such as `docs`, `refactor`, `build`, `ci`, `test`, `chore`, `style` and `revert`, and any subject that does not follow Conventional Commits |

      A breaking commit shows only in the first section. Its text is the footer text when the commit has a footer, else the subject text. A trailing `(#123)` in the subject is the pull request number.
   3. *Two outputs.* The script prints `release-notes.json`, and renders markdown from that file, so both say the same thing. The JSON has these fields:

      | Field | Meaning |
      |---|---|
      | `format` | `1`. A reader skips a file with a format it does not know. |
      | `component`, `version`, `tag` | The release. |
      | `previous`, `previous_tag` | The previous release of the component, or `null` for the first one. |
      | `date` | The day the notes were built, as `YYYY-MM-DD` in UTC. |
      | `sections` | A list of `{name, title, items}`, with `name` one of `breaking`, `features`, `fixes`, `performance` and `other`. Each item is `{type, scope, text, pr, breaking}`. |

      The JSON keeps the text as the commit has it, with control characters removed. The markdown puts a backslash before each markup character, so a subject cannot add a link, an image, HTML or emphasis to the release page. Each line is the text and a link to the pull request. Lines under Other changes start with the type. The markdown ends with a link that compares the two tags.
2. **The release page.** On a `v*` or `telemetry-v*` tag, `release.yml` builds the body from the generator: the upgrade note in `.github/release-notes-upgrade.md` for playbook, or the assets note for telemetry, then the notes. It replaces the notes GitHub generates. It attaches `release-notes.json` as an asset, and the `checksums` job lists it in `SHA256SUMS` and attests it with the binaries. A release that exists before the run keeps its body and gets the asset. A dry run prints the body and the JSON in the job summary. `plugin-release.yml` builds the plugin release body with the same script, from the commits that touch `plugins/playbook`.
3. **No gates.** `breaking-changes.json`, `scripts/check-breaking.sh`, the check in `verify-version`, the breaking part of the `version-bump` plugin guard, the `### Breaking` changelog heading rule, `metadata.breakingChanges` in `plugin.json` and its mirror check are removed. A plugin change still needs a version bump and a changelog section. The `breaking-changes` pull request check stays as a warning that never fails: when the title has `!` and the body has no `BREAKING CHANGE:` footer, it asks for a footer that says what breaks and what to do.
4. **The policy before 1.0.** `CONTRIBUTING.md` states it. A breaking change can ship in any release. Its commit carries `!` or a `BREAKING CHANGE:` footer with a one line action for the user. The release notes show it first. The migrations of ADR 0034 still run on first use, so a change to playbook's own files heals itself.
5. **Delivery.**
   1. *`playbook update`.* After it resolves the release, the update downloads that release's `release-notes.json` and checks it against the `SHA256SUMS` it already downloads for the binary. It prints the Breaking changes section, if there is one, and the line `Full notes: <release url>`, before the swap and again after the swap and the heal. When the running version is below the `previous` of the notes, the update crosses more than one release, and a line names the releases page. When the asset is missing, does not download, does not match its checksum or does not parse, the update prints only the release URL. The notes never fail an update.
   2. *Automatic updates.* The detached process reads the same asset in the same way. A release whose notes list a breaking change falls back to the notice of ADR 0036 decision 5 and waits for the user. A release without the asset has no known breaking change. An asset that does not match or does not parse holds the release for a day. The report of a finished update is the line `playbook updated from A to B. Release notes: <url>` and its warnings.
   3. *First run after any other upgrade.* After `brew upgrade`, the install script or a manual download, the first run step of ADR 0034 decision 13 prints one line on stderr: `playbook updated from A to B. Release notes: <url>`. The `session-init` hook adds the same line to the session message. The line needs no network. It reads `notices/binary` in the `state` table of `config.db`, else `versions/binary`, and moves `notices/binary` to the running version in one transaction, so it shows once. `playbook migrate`, `playbook doctor --fix` and `playbook init` move the key without printing, since `playbook update` and the install script run them right after they print their own notes. A fresh install and a downgrade show nothing. `notices/plugin` is no longer written.
   4. *The install script.* After it replaces an older binary, the script downloads `release-notes.json` and checks it against the `SHA256SUMS` it already reads. It prints `playbook updated from A to B. Release notes: <url>`, then the Breaking changes section, which the new binary renders from the file with the hidden flag `playbook update --print-notes <file>`. When the asset is missing or does not match, or the binary has no such flag, it prints the line alone.
   5. *Homebrew.* The caveats of each formula name the release notes of the installed version in one line, built from `#{version}`. The bump script writes no block, and the first run line covers the upgrade itself.
   6. *The plugin.* The plugin release page carries its notes. The `session-init` line that names an older binary than the plugin needs stays (ADR 0034 decision 10).
6. **Removed code.** The `playbook breaking` command, the record reader in `crates/cli/src/breaking.rs`, the `PLAYBOOK_BREAKING_SHOWN` variable and its handling in the install script, the update and the heal, the tap's breaking block and its cases in the bump script, and the tests of all of these. ADR 0035 stays as it was, marked superseded.

## Alternatives considered

- **Keep the record of ADR 0035.** Rejected: before 1.0 a gate on each marked commit costs more than the notice is worth, and the plugin copy needs a version bump for a wording fix.
- **The notes GitHub generates.** Rejected: they list pull request titles with no sections and no data file, so the binary cannot find the breaking changes.
- **A release tool such as git-cliff or release-please.** Rejected for now: each adds a dependency and a configuration format to learn. The script is a short `git log` and one `jq` program.
- **Notes built into the binary.** Rejected: a running binary cannot know about later releases.
- **Fetch the notes on the first run.** Rejected: the first run step and the session hook must stay offline and fast. They print a link.

## Consequences

- A breaking change needs no extra file. Its commit says what breaks, and the release notes show it first.
- The notes are as good as the commit subjects and footers. A breaking commit without a marker shows as a normal change. The pull request warning is the only reminder.
- The notes of a release cover that release alone. An update that crosses several releases shows the breaking changes of the newest one and links the releases page for the rest.
- The first run line after `brew upgrade` or the install script carries a link, with no list of changes.
- In `update.mode = auto`, any release with a breaking change waits for the user. They see its notes when they run `playbook update`.
- No released binary reads `breaking-changes.json` or has `playbook breaking`, since ADR 0035 shipped after 0.26.0. Removing them breaks no installed binary.
