# ADR-0048: Tidy the Repo Root, the Settings Seed and the Leftovers

- **Status:** Accepted
- **Date created:** 2026-10-19
- **Date modified:** 2026-10-19
- **Builds on:** [ADR 0006](0006-separate-product-tree-from-runtime-dir.md), which left open whether the seed is generated or written by hand.
- **Amends:** the [ADR 0007 blueprint](0007-rust-binary-for-hooks-and-launcher-blueprint.md) work units 20 and 21, which gave `playbook settings gen` its PERMS argument and `playbook settings check` its permissions sync check; [ADR 0037](0037-release-notes-from-conventional-commits.md) decision 7, which set 0.36.0 for the removal of `breaking-changes.json`; and [ADR 0041](0041-retire-playbook-telemetry.md) decision 5, which set 0.36.0 for the removal of `install.sh --pbtop`.

## Context

The repo root holds 26 entries. Nine of them are not community or build files: `Brewfile`, `breaking-changes.json`, `install-pbtop.sh`, `install.sh`, `intents.json`, `justfile`, `permissions.shared.json`, `prompts/` and `settings.shared.json`. A new contributor cannot tell which of them ship to users, which are for maintainers and which are dead.

The settings seed is split in two. `permissions.shared.json` holds a copy of the `permissions` block of `settings.shared.json`. `playbook settings gen` writes that block into the seed, and `playbook settings check`, `playbook ci` and the rust-ci job fail when the two copies differ. Every change to the shipped permissions needs two edits and a regenerate, and the copy protects nothing: the binary only ever reads `settings.shared.json`.

`breaking-changes.json`, `src/breaking.rs` and `playbook breaking` were due to go in 0.36.0. 0.37.0 shipped with them still in the tree.

The repo also carries smaller mess: comments that describe a retired Python and Makefile pipeline, an ADR table that missed two ADRs, four workflows that run Node steps in a Rust repo and a fifth that runs the same unconfigured agent, and a Dependabot file with no updates.

## Inventory of the root

| Entry | Read by | Used | Goes to |
|---|---|---|---|
| `.cargo/`, `.github/`, `.gitignore`, `Cargo.toml`, `Cargo.lock`, `rust-toolchain.toml`, `crates/` | cargo, GitHub, git | yes | stays |
| `README.md`, `LICENSE`, `NOTICE`, `CHANGELOG.md`, `CODE_OF_CONDUCT.md`, `CONTRIBUTING.md`, `SECURITY.md` | people, GitHub, the release notes script | yes | stays |
| `docs/`, `plugins/`, `scripts/` | people, the marketplace, CI | yes | stays |
| `install.sh` | `curl .../main/install.sh`, named in the docs, in released binaries and in `install-pbtop.sh` | yes | stays, public URL |
| `install-pbtop.sh` | `curl .../main/install-pbtop.sh` in `README.md`, `docs/guides/00-install.md` and `crates/telemetry/README.md` | yes | stays, public URL |
| `justfile` | `just`, `CONTRIBUTING.md` | yes | stays, see decision 4 |
| `settings.shared.json` | `include_str!` in `init/self_root.rs`, `playbook ci`, `playbook settings check`, rust-ci and its cache key, `scripts/release-notes.sh`, tests, the manifest allowlist | yes | `seed/` |
| `prompts/SYSTEM_PROMPT.md` | `include_str!` in `init/self_root.rs`, rust-ci cache key, `scripts/release-notes.sh`, tests, the manifest allowlist | yes | `seed/prompts/` |
| `Brewfile` | `include_str!` in `init/self_root.rs`, `playbook deps ensure --builtin`, `brew bundle`, rust-ci cache key, `scripts/release-notes.sh`, tests, the manifest allowlist | yes | `seed/` |
| `intents.json` | `scripts/check-intents.sh`, the `intents-contract` workflow, `skills_lint.rs`, the manifest allowlist | yes | `plugins/` |
| `permissions.shared.json` | `settings gen`, `settings check`, `playbook ci`, rust-ci, `justfile` | a copy only | deleted |
| `breaking-changes.json` | `src/breaking.rs`, `playbook breaking`, release assets | only 0.34.x reads it, through the asset and the new binary's `playbook breaking` | deleted |

## Decision

1. **One `seed/` folder for what the binary ships.** `seed/` holds exactly the three files the binary embeds with `include_str!` and writes to `~/.config/playbook/embedded/<version>/`: `settings.shared.json`, `prompts/SYSTEM_PROMPT.md` and `Brewfile`. The folder has the same layout as the embedded folder. `seed_in_checkout` maps a checkout root to its `seed/` folder, the way `plugin_root::in_checkout` maps it to `plugins/playbook`, so a dev checkout in `CLAUDE_PLUGIN_ROOT` and `playbook ci` keep finding the seed, and tests can keep pointing at the repo root. The name is `seed`, since the code already calls these files the seed (`has_seed_files`, `seed_root`, `tests/install_seed.rs`). `config` would clash with `crates/cli/src/config`, `playbook config` and `~/.config/playbook`.

2. **`settings.shared.json` is the only source of the shipped permissions.** `permissions.shared.json` is deleted. A maintainer edits the `permissions` block of `seed/settings.shared.json` by hand. `playbook settings gen <SRC> <SEED>` refreshes the hooks and status line of the seed from a live `settings.json`, keeps the seed's own `permissions` block and prints the result to stdout, as today. The PERMS argument is gone. Gen exits 2 and prints nothing when SEED cannot be read or has no `permissions` object with a non-empty `allow` array. It never falls back to the permissions of SRC, which are the maintainer's own. `playbook settings check <SEED> <ROOT>` drops the sync check and keeps the shippable key allowlist, no `model`, no `skipAutoPermissionPrompt: true` and hook commands that exist. It gains the rule gen used to apply to PERMS: `permissions` is an object with a non-empty `allow` array. `playbook ci` runs the two argument check, so the settings row passes and is never skipped for a missing file. `setup --security` already reads the block from the embedded seed and does not change.

   ADR 0006 rejected a hand written seed because it drifts from the config the maintainer runs. That concern is about the hooks and the status line, which gen still copies from the live file. The shipped permissions are a product choice with no live copy to drift from.

3. **`breaking-changes.json` and its machinery go now.** This removes the file, `src/breaking.rs`, `playbook breaking`, the `PLAYBOOK_BREAKING_SHOWN` constant, the release asset lines in `release.yml`, the `breaking-changes.json` part of the rust-ci cache key, the manifest entry and the docs that name them. The tap's breaking block and the bump script cases that ADR 0037 also lists are already gone. No compatible empty file is uploaded. The only binaries that read the asset are 0.34.x. Their auto update treats a 404 for the asset as "no breaking changes" (`is_not_found` in `v0.34.0:crates/cli/src/update/auto.rs`), and their `playbook update` treats a failed `playbook breaking` on the new binary as an empty list (`breaking_notice` in `v0.34.0:crates/cli/src/update/mod.rs`). Binaries from 0.35.0 on read `release-notes.json` only. `playbook update` keeps working on every released binary. The `--pbtop` flag of `install.sh` goes in a later change, since another branch is rewriting `install.sh` now.

4. **`justfile` stays at the root.** `just` finds its file by walking up from the current folder. In `dev/justfile` every contributor would type `just -f dev/justfile` and every recipe would need `set working-directory`. CI does not call `just`. With the `Brewfile` in `seed/` and the `justfile` at the root, no `dev/` folder is needed.

5. **`intents.json` moves to `plugins/`.** It is the contract between the plugin and the binary, so it belongs with the plugin. `skills_lint.rs` already looks for it one level above the plugin root. The `intents-contract` path filter changes to `plugins/intents.json`, and plugin-guards now also runs on an intents edit.

6. **The rest of the tidy follows in small pull requests**, each under 1000 changed lines, with moves done by `git mv` and never mixed with logic changes:
   1. this ADR;
   2. settings single source (logic);
   3. the `seed/` and `plugins/intents.json` moves, with every path that names them: the `include_str!` paths, the tests that read them, the rust-ci cache key globs, and the `scripts/release-notes.sh` pathspec, which keeps the old root paths next to `seed` so the notes of the release with the move still see older commits;
   4. the `breaking-changes.json` removal;
   5. scripts, `justfile` and `Brewfile` tidy;
   6. docs: one index in `docs/README.md`, an ADR index in `docs/adr/README.md` with a test that fails when an ADR has no row, the overlap between `docs/concepts`, `docs/guides`, `docs/internals` and `docs/reference` merged, stale names removed;
   7. `.github`: dead workflows removed, Dependabot updates for cargo and GitHub Actions;
   8. code: files over about 800 lines split along their seams, dead code removed, one pull request per crate area, after the open migration, disk guard, rm guard, attribution and install branches merge.

   Every pull request keeps `playbook ci`, the manifest check, the egress test, shell-ci and rust-ci green. The seed pull requests prove that `playbook setup --yes --security` in a scratch HOME writes the same `settings.json` before and after, once with the embedded copies and once with `CLAUDE_PLUGIN_ROOT=<repo>/seed`. `memory_isolation.rs` reads the seed with `unwrap_or_default()` and skips a missing folder, so the move makes it fail on a missing path.

## Slop found

Each item is fixed in the pull request named in decision 6.

| Item | Where | Fix |
|---|---|---|
| Comments about the retired Python generator, shell scripts and Makefile | `settings/gen.rs`, `settings/check.rs`, `manifest/check.rs`, `justfile` header | rewrite to describe the current code (6.2, 6.5) |
| `docs/index.md` ADR table missed 0045 and 0047 | `docs/index.md` | rows added with this ADR; the table moves to `docs/adr/README.md` (6.6) |
| A guide named after the old `init` command | `docs/guides/06-what-init-installs.md` | rename to `05-what-setup-installs.md` and renumber the guides (6.6) |
| Guides listed out of order, a config key reference filed as a guide | `docs/index.md`, `docs/guides/04-config-keys.md` | move to `docs/reference/` (6.6) |
| `docs/policy-table.md` alone at the docs root | `docs/` | move to `docs/reference/` (6.6) |
| `docs/concepts/` and `docs/internals/` both explain how it works | `docs/` | merge into `docs/internals/` (6.6) |
| ADR 0024 and 0025 still Proposed | `docs/adr/` | set the status the code shows (6.6) |
| `.github/release-notes-upgrade.md` tells users about the `playbook-hooks` plugin of 0.34 | `.github/` | delete; `CHANGELOG.md` keeps the version note (6.4) |
| Four `agent-*.yml` workflows run `npm ci` and `npm run build` in this Rust repo, and `agent-triage.yml` calls the same unpinned outside workflow (issue #547) | `.github/workflows/` | delete, close #547 (6.7) |
| `dependabot.yml` has no `updates` | `.github/dependabot.yml` | weekly grouped cargo and GitHub Actions updates (6.7) |
| `CONTRIBUTING.md` says "until 0.36.0" for `breaking-changes.json` | `CONTRIBUTING.md` | remove (6.4) |
| `crates/cli/src/init/` still named after the old command | `crates/cli/src/` | rename to `setup/` (6.8) |
| Source files over 800 lines: `main.rs` 2769, `lib.rs` 2678, `init/migrate.rs` 2256, `rm_workspace_guard/parse.rs` 2055, `cc/worktree.rs` 2046, `tui/view.rs` 1714, `doctor/check.rs` 1468, `tui/chart.rs` 1485, `tui/app.rs` 1484, `code/src/index.rs` 1411, `rebuild_memory_graph.rs` 1344, `common/shell.rs` 1295, `common/attribution.rs` 1266, `tui/panels.rs` 1225, `policy_guard.rs` 1214, `telemetry_db.rs` 1173, `service_manager.rs` 1122, `update/check.rs` 1116, `config/store.rs` 1113, `limits_source.rs` 1092, `heal.rs` 1074, `eval/bench.rs` 1073, `memory_anchors.rs` 1070, `query.rs` 1066, `data/src/migrate.rs` 1063, `tui/snapshots.rs` 1056, `effort/auto.rs` 1036, `commit_message_sanitizer/git.rs` 1035, `collector.rs` 1025, `plan.rs` 1010, `update/mod.rs` 1010, and 13 more between 800 and 1000 | `crates/` | split along their seams (6.8) |

No TODO or FIXME comment in the tree names an issue. No script under `scripts/` is dead: each one is called by a workflow, another script or a test. There is no `shell/` folder left. `brew "node"` in the `Brewfile` is used: the worktree engine runs `npm install`, `playbook commit` runs `npx biome` or `npx prettier` in JavaScript projects, and the anydoc install hint needs `npm`.

## Alternatives considered

- **Keep `permissions.shared.json` and drop the sync check.** Rejected: two files that can disagree with nothing to say which one wins.
- **A forwarding stub at the root for `install.sh` and `install-pbtop.sh`.** Rejected: released binaries and users' notes fetch the root URL, and a stub adds a second hop that can break.
- **Upload an empty `breaking-changes.json` for one more release line.** Rejected: 0.34.x already handles a missing asset, and an empty file keeps the release step alive for no reader.
- **Name the folder `config/`.** Rejected for the clash in decision 1.

## Consequences

- The root holds only community files, build files, the main folders and the two public install scripts.
- A permissions change is one edit in one file.
- `playbook settings gen` and `playbook settings check` take one argument fewer. They are maintainer commands, so no user setup changes.
- A 0.34.x binary that updates to a release without `breaking-changes.json` prints no breaking change block. It still prints the release link, as ADR 0037 expects.
- The `agent-*.yml` workflows say the agent-factory app provisions them. It has to stop provisioning this repo, or they come back.
- `.github/dependabot.yml` says it is managed by repo-config. The same change has to land there, or the next `terraform apply` reverts it.
