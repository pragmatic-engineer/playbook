# ADR-0020: A Terminal UI for `playbook usage`, With ratatui and crossterm

- **Status:** Accepted
- **Date created:** 2026-10-10
- **Date modified:** 2026-10-10

## Context

Issue #577 asks for a btop style terminal view of usage. The maintainer decision on the issue is binding:

- `playbook usage` with no subcommand opens the terminal UI.
- The other `usage` subcommands are deprecated for one release with a warning that names the replacement, then removed.
- The web dashboard stays for one release behind an explicit flag (`--web`) with a deprecation notice, then `src/usage/dashboard.rs`, its `tiny_http` server, its tests and its docs are deleted (#525, #550).
- The store and the query layer stay, because the terminal UI reads them.
- A non-interactive `--json` or `--summary` output stays for scripts and for a session with no TTY.

The target is v0.21.0. Removal of the dashboard happens the release after, and is not part of this work.

## Decision

1. **Library: `ratatui` 0.30.2 with `default-features = false` and the `crossterm` feature.** It gives widgets (table, tabs, sparkline, bar chart, gauge), a layout engine, and a `TestBackend` that renders to a buffer, so a screen can be asserted in a unit test without a terminal. `ratatui::crossterm` re-exports the event API, so there is no direct `crossterm` dependency to pin.
2. **One query layer for both views.** `src/usage/query.rs` holds the date `Range`, the `Totals` over a set of events, and the typed `LiveView`. The web JSON in `src/usage/api.rs` and the terminal view both call it, so the same range gives the same number in both. The existing aggregation (`aggregate.rs`) and the store (`db.rs`) are unchanged.
3. **Refresh from the same store.** The terminal view ingests new transcript lines and re-reads the store on a timer, with the same functions the web live stream calls (`ingest_new`, `load_live_inputs`). It does not start the dashboard server.
4. **Scripting.** `playbook usage --json` prints the totals, the groups and the live view as JSON, and `playbook usage --summary` prints the existing text report. Both run when stdout is not a TTY, so a pipe or CI never gets a screen of escape codes.
5. **Deprecation.** `usage ingest` and `usage dashboard` keep working for one release and print a warning on stderr that names the replacement (`playbook usage --json` for `ingest`, `playbook usage --web` for the dashboard, which is itself deprecated).

## Dependency vetting

Measured on macOS arm64 on 2026-10-10 with the release profile (`opt-level = "s"`, fat LTO, strip), using a probe that links a tab bar, table, sparkline, bar chart, gauge, list and the event loop.

| Item | Result |
| --- | --- |
| `ratatui` | 0.30.2, MIT, released 2026-06-19, about 58 million downloads, first published 2023 |
| `crossterm` (through `ratatui-crossterm`) | 0.29.0, MIT, about 206 million downloads, first published 2018, last release 2025-04 |
| New packages in `Cargo.lock` | 70 (91 to 161), which counts Windows-only crates and build-time proc-macro crates |
| Licenses of the new packages | all permissive: MIT, Apache-2.0, MIT OR Apache-2.0, Zlib, BSL-1.0 option. Compatible with Apache-2.0 |
| Known vulnerabilities | `cargo audit` against 1296 advisories (2026-10-10) reports none for the 161 locked crates |
| Release binary | 6,025,152 bytes before, 6,240,800 after the probe (+215,648 bytes, +3.6%). The real view uses more widgets, so the final delta is reported in the PR that adds it |
| Clean release build | 68 s before, 72 s with the probe, with the other crates already cached. Compiling the new crates is a one time cost per cache |
| Minimum Rust | 1.88, the repo pins 1.97.0 |

`default-features = false` drops `macros`, `layout-cache`, `all-widgets` (the calendar widget) and `underline-color`.

## Alternatives considered

- **`crossterm` alone with hand drawn output.** Smaller (no widgets), but every panel, the layout and the resize logic would be ours, and there would be no `TestBackend` to snapshot. Rejected: the saving is a fraction of the 215 KB and the cost is a large amount of terminal drawing code that ratatui already tests.
- **`cursive` or `tui-realm`.** Heavier models (callbacks or a component framework) for a read-only dashboard. Rejected.
- **`termion`.** Unix only, and the repo keeps `cargo check` on Windows green.
- **Keep the web dashboard as the default.** Rejected by the maintainer decision.

## Consequences

- The release binary grows by roughly 0.2 to 0.3 MB. Removing `tiny_http` the release after (about 60 KB, per the issue) gives a little back.
- Dependabot now tracks 70 more crates. They are pinned with `=` like the others.
- Both views stay consistent by construction until the web view is removed. A test over a fixture store asserts it (added with the terminal UI).
- The terminal view needs a TTY. Without one, `playbook usage` prints the text summary, as before.
