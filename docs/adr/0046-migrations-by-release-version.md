# ADR-0046: Attach Each Migration to the Release That Introduced It

- **Status:** Accepted. Amended 2026-10-19 in implementation: `release-prepare.sh` also refuses a version that already has its tag, so a manual run cannot mark a `next` migration as shipped in a released build; a `since` that is not a plain X.Y.Z stays unreleased, because `update::resolve::compare` reads a word as 0; and the hook writes slug pending ids together with schema 20, so an older binary of the same version backs off before it drops ids it cannot match.
- **Date created:** 2026-10-18
- **Date modified:** 2026-10-19
- **Amends:** [ADR 0034](0034-self-healing-upgrades.md) decision 5, which records `versions/schema` as the highest migration number, decision 7, which compares the running version and schema with the record, and decision 12, whose `--status` shows the record as a version and a migration number. [ADR 0016](0016-migration-framework.md) decision 1, whose entries are `Migration { id, kind, run }` with numbered ids. [ADR 0039](0039-release-workflow.md) decision 3, whose `release-prepare.sh` only bumps the version and adds the changelog section.

## Context

Each migration in `crates/cli/src/init/migrate.rs` has a numbered id, from `0001-memory-store-move` to `0019-system-prompt-default-on`, and `0020-collector-service-label` is in review. `migrate::schema()` is the highest number. The heal writes it to `versions/schema` next to `versions/binary`, and `Record::compare_with` orders a record by the binary version first and the schema second. A record that is ahead on either one counts as a downgrade, and nothing runs.

The number causes three problems.

- A build from `main` reports the last release version, because the version only moves in the release pull request (ADR 0039 decision 3). A migration merged after 0.37.0 raises the schema to 20 while the version stays 0.37.0. A dev build then sees schema 20 above the recorded 19 and heals, so the migration runs before its release ships. That build records 0.37.0 with schema 20, and the released 0.37.0 binary then reads the record as newer and runs nothing.
- Two branches that each add a migration both take the next number. The second one to merge has to renumber, and the numbers in comments and docs drift. Today `collector_service.rs` and ADR 0043 call `0016-collector-service` "migration 0017".
- The number says nothing about the release a user needs. The question a user asks is "what changed since my version", and the record already stores a version.

Every migration so far shipped in a tagged release, which `git tag --contains` on the commit that added it shows:

| Old id | Slug | Since |
|---|---|---|
| 0001 | `memory-store-move` | 0.18.0 |
| 0002 | `gate-repo-local-move` | 0.18.0 |
| 0003 | `system-prompt-edited` | 0.18.0 |
| 0004 | `skills-edited` | 0.18.0 |
| 0005 | `statusline-edited` | 0.18.0 |
| 0006 | `statusline-rust-command` | 0.18.0 |
| 0007 | `shell-init-rc-line` | 0.18.0 |
| 0008 | `remove-config-hash-script` | 0.20.0 |
| 0009 | `adopt-late-config-json` | 0.20.0 |
| 0010 | `state-files-to-sqlite` | 0.20.0 |
| 0011 | `dead-hook-scripts` | 0.34.0 |
| 0012 | `other-playbook-plugins` | 0.34.0 |
| 0013 | `plugin-version-range` | 0.34.0 |
| 0014 | `orphaned-plugin-caches` | 0.34.0 |
| 0015 | `attribution-off` | 0.36.0 |
| 0016 | `collector-service` | 0.36.0 |
| 0017 | `auto-budget-defaults` | 0.36.0 |
| 0018 | `retired-launcher-rc-line` | 0.37.0 |
| 0019 | `system-prompt-default-on` | 0.37.0 |
| 0020 | `collector-service-label` | next |

`git show v0.37.0:crates/cli/src/init/migrate.rs` holds 0018 and 0019, so both are in 0.37.0. Only 0020 is unreleased.

## Decision

1. **Each migration names its release.** A registry entry has a stable `id` slug, a `since` version, a `kind` (`Auto`, `Idempotent`, `Manual`) and a `scope` (`Owned`, `Consent`, `Report`). The list order is still the apply order. `since` never goes down along the list, and `next` entries come last.
2. **Ids are slugs.** The existing ids drop their number, as the table above shows. A slug is never renamed or reused, because the record and the doctor name it.
3. **Unreleased migrations say `since: "next"`.** A build treats `next` as newer than itself, so it never runs that migration, never checks it and never lists it as open. The code matches `next` before it compares versions, because `update::resolve::compare` reads a word as 0. `scripts/release-prepare.sh` replaces each `since: "next"` in `migrate.rs` with the release version and then fails when one is left. Both steps run on every run of the script, before its early exit for a checkout that already carries the version, so a resumed release still fills them in. The change lands in the release pull request with the bump. A unit test checks that each `since` is `next` or a version no higher than the crate version, that `since` never goes down along the list, that every `next` comes last, and that each `next` is written in the one form the script replaces.
4. **What runs.** `R` is the recorded version and `V` the running one, compared as semver with `update::resolve::compare`. A migration is released when its `since` is a version and not above `V`.
   - A heal is due when there is no record or `R` is below `V`. When `R` equals `V`, the hook only rechecks the open items and the first-run step does nothing. When `R` is above `V`, nothing runs, as today. The schema no longer breaks a tie.
   - A released `Idempotent` `Owned` migration runs on every heal.
   - A released `Auto` `Owned` migration runs when no applied row names it. The applied row is the only rule. An `Auto` migration that returns without finishing writes no row, so it runs again on a later heal even after the record moved to `V`.
   - `Consent` and `Report` checks and the `Manual` notes cover released migrations only. An item that is still open stays open whatever its `since`.
   - `playbook setup` follows the same rule for `next`.
5. **The record keeps its keys.** `versions/binary` stays the last version that ran the heal clean. `versions/pending` holds slugs. Applied `Auto` migrations are `migrations/applied/<slug>`. A migration that had a number in a release, 0001 to 0019, also gets its numbered row when it is applied, so an older binary sees it as applied too. The code keeps a fixed table of those nineteen old ids for this and for the conversion. `versions/plugin` and `versions/failed-at` do not change. `versions/schema` is written as 20, the last number ever used, and is never raised. Older binaries still parse it, and one with the same version reads the record as newer.
6. **Answers stay where they are.** A stored no lives in its own key: `attribution/declined`, `collector-service/declined`, `collector-service/reminded` and the `systemPrompt.enabled` config key. None holds a migration number, so each survives this change as it is. The registry does not copy them.
7. **A one-time conversion.** Reading the record maps an applied row or a `versions/pending` id from the old-id table to its slug, so `0017-auto-budget-defaults` reads as `auto-budget-defaults`. A numbered id outside the table, such as `0020-collector-service-label` from a dev build, maps to nothing and is left alone. Every path that runs migrations (`setup`, `migrate`, `doctor --fix`, the hook and the first-run step) takes the migration lock and writes a slug row for each numbered applied row before anything else runs. It keeps the numbered rows, and every later write of the `migrations/` rows writes the raw keys it read, so no numbered row is lost. The next record write stores `versions/pending` as slugs. The conversion keys on the applied rows, whether a `versions/` record exists or not, and runs after the import of an old `migrations.state` file. Running it twice changes nothing.
   - **0.18 to 0.26.** No `versions/` record. Applied rows, if any, sit in `config.db` or come in from `migrations.state` when the store opens, and the conversion picks them up. The heal is due and runs the released migrations, as it does today.
   - **0.34.0 to 0.37.0.** The first heal or `setup` of the first release with this change converts the rows, then writes the new version.
   - **A record at 0.37.0 with schema 19.** The same. A dev build that still reports 0.37.0 reads the record as equal and does not heal on its own. `playbook migrate` there converts the rows and writes schema 20.
   - **An older binary after the new one ran.** From 0.34.0 the hook, the first-run step, `migrate` and `doctor --fix` read a version above their own, or the same version with schema 20 above their own number, and stop at the downgrade guard: "a newer playbook ran here; nothing runs after a downgrade". No key they read is removed. An older `playbook setup` runs its own registry before it reaches that guard, as it does today. Its `Idempotent` migrations are safe to repeat, its `Manual` ones only warn, and its one `Auto` migration, `0017-auto-budget-defaults`, finds its numbered row and is skipped. It rewrites the `migrations/` rows from the rows it read, so the slug rows stay. A 0.26 binary has no version record and no guard, and its registry has no `Auto` migration.
8. **Output names versions and slugs.** `playbook migrate --status` prints `running: playbook V` and `recorded: playbook R`, then one line per migration with its `since`, slug, kind, scope and state. `--json` keeps every field name and adds `since` to each migration, a version or `next`. `id` holds the slug. `running.schema` and `recorded.schema` stay, with the fixed 20 and the stored number. The doctor row says `upgrade record: playbook R`. Notes and open items start with the slug. The `migration` field of `breaking-changes.json` names a slug.
9. **Adding a migration.** Append the entry with `since: "next"` and a new slug, pick the kind and the scope, and add a test. Never renumber, rename or reorder. The release fills in the version.

## Alternatives considered

- **A pre-release version on `main`, such as 0.38.0-dev, right after each release.** Rejected. `release-plan.sh` reads a version above the last tag as a pending release to finish (ADR 0039 decision 2), `bump-version.sh` takes MAJOR.MINOR.PATCH only, and Claude Code reads the plugin version from the same files. Every release would need a second version pull request, and a dispatch between the two would try to release the pre-release.
- **Keep the numbers and add `since`.** Rejected. Two orders to keep in step, and the number still collides across branches.
- **Work out `since` from git tags in `build.rs`.** Rejected. A build from a source archive has no git history, and a dev build would still need its own rule.
- **Move the stored answers to `migrations/declined/<slug>`.** Rejected. An older binary reads the old keys and would ask again after a downgrade. Its `setup` also rewrites the `migrations/` rows from the applied and shipped rows it read, so it would drop a declined row there.
- **Drop the numbered applied rows during the conversion.** Rejected. An older binary's `setup` would then run `auto-budget-defaults` again. The rows are a few bytes.

## Consequences

- A dev build never runs a migration before its release.
- A `next` migration that merges while a release pull request is open ships in that release's binary without running, and gets the version of the release after. `since` names the first release that runs it. To try a new migration end to end, run `scripts/release-prepare.sh` in a scratch checkout and build that, or use the unit tests, which pass the version in.
- Branches that add migrations no longer collide, and no doc or comment needs a number.
- The release pull request touches `migrate.rs` whenever a `next` migration is waiting. A `next` the script missed stops the release before the pull request opens.
- `config.db` keeps the numbered applied rows next to the slug rows, and the code keeps the table of nineteen old ids.
- The `id` values in `playbook migrate --json` change from `0015-attribution-off` to `attribution-off`.
- Old binaries need no change. Their heal paths stop at the downgrade guard, and their `setup` finds the numbered rows it expects.
