# ADR-0029: Keep the Rust Crates in One Cargo Workspace

- **Status:** Accepted, amended by ADR 0030 (the plugin root no longer holds the skills, commands and agents) and by [ADR 0038](0038-one-release-line.md) (decisions 4 and 5: `playbook-telemetry` joins the `v` release line), and by [ADR 0039](0039-release-workflow.md) (decision 6: the release opens the tap pull request)
- **Date created:** 2026-10-13
- **Date modified:** 2026-10-16

## Context

The binary grew a code index and a usage dashboard that other tools should reuse. The usage dashboard lived in its own repository, `playbook-telemetry`, and reached `playbook` through a git tag pin in `Cargo.toml`. Every change to it took a release, a bump pull request and a second review loop. A code index (`playbook-code`) and a shared storage crate (`playbook-data`) were planned as two more repositories with the same cost. Shared helpers such as the data folder paths had no home that every crate could depend on.

## Decision

1. **One repository, one Cargo workspace.** The crates live under `crates/`: `cli` (package `playbook`, the binary and library), `common` (package `playbook-common`, shared helpers including the paths), `code` (package `playbook-code`), `telemetry` (package `playbook-telemetry`, with the `playbook-telemetry` and `pbtop` binaries) and `data` (package `playbook-data`, shared storage). Directory names are short. Package, library and binary names keep the `playbook-` prefix.
2. **The plugin root stays the repository root.** `commands/`, `skills/`, `agents/`, `hooks/`, `output-styles/`, `prompts/`, `bin/`, `install.sh` and `.claude-plugin/` do not move. The plugin zip file list is the same as before the move.
3. **Crates depend on each other by path.** There is one `Cargo.lock`. The internal-crate bump workflow and its scripts are removed.
4. **Tags carry a prefix per tool.** `vX.Y.Z` releases `playbook` (plugin zip, five binaries, `SHA256SUMS`, attestations). `telemetry-vX.Y.Z` releases `playbook-telemetry` as `playbook-telemetry-<version>-<target>.tar.gz` holding both binaries. `code-vX.Y.Z` is reserved for `playbook-code`. Telemetry and code releases are created with `--latest=false` so GitHub's latest release stays on the `v` line.
5. **Versions.** `playbook` and `playbook-code` use the workspace version. `playbook-telemetry` keeps its own version. `playbook-common` and `playbook-data` are not published.
6. **Consumers map tags to tools.** The Homebrew tap maps each formula to a repository and tag prefix and falls back to the legacy repository until a prefixed release exists. The tap checks for new releases every 30 minutes, so the release workflow sends it nothing. The marketplace pin follows plain `vX.Y.Z` tags only.
7. **CI selects packages by changed path.** The required check names do not change. A change under one crate runs that crate and the crates that depend on it. Anything else runs the whole workspace.
8. **History.** `playbook-telemetry` was imported as a single squash commit of tag v0.1.7, because the repository allows squash merges only. The old repository is archived with a README pointer and keeps its history and releases.

## Limits that remain

- A change to `playbook-common` runs the whole workspace.
- Tests read repository files through `PLAYBOOK_REPO_ROOT`, set in `.cargo/config.toml`. Code that reads the repo root outside Cargo needs its own path.
- Existing installs keep working: the binary, the formulas and the zip are named as before.

## Alternatives considered

- **Keep separate repositories with tag pins.** Rejected: each change needs a release and a bump pull request.
- **Git submodules.** Rejected: they hide the code from the same checkout and the same lock file.
- **One crate with modules.** Rejected for telemetry, which ships its own binaries on its own release cadence. The code index may follow with a `pbcode` binary and `code-v` releases.
- **Publish the crates on crates.io.** Rejected for now: the API is still settling, and the internal crates set `publish = false`.

## Consequences

- Cross-crate changes land in one pull request.
- One release workflow, `release.yml`, serves every tool in the repository. Its first job maps the tag prefix to the package, binaries and archive shape, and fails on an unknown prefix.
- Tools that read the repository layout (the tap, the marketplace, scripts) must know the prefixes.
