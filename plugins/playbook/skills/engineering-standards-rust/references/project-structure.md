# Project structure

Workspace layout, crate boundaries, modules and visibility. A lint marked (pedantic), (nursery), (restriction) or (cargo) is off by default.

## Rules

1. **A repository with several crates uses one workspace.** Put members under `crates/`, and set `resolver` in a virtual root manifest.
   Why: one `Cargo.lock` and one target folder for all crates.

2. **Share versions and settings at the root.** Declare dependencies once in `[workspace.dependencies]` and write `serde.workspace = true` in members. Share `edition`, `rust-version` and `license` through `[workspace.package]`, and lints through `[workspace.lints]`.
   Why: one place to bump a version, and no drift between crates.

3. **Profiles live in the root manifest.** Cargo ignores `[profile]` tables in member crates.

4. **Keep `main.rs` thin.** It parses arguments, sets up logging, calls the library and maps the error to an exit code. The logic lives in `lib.rs` or in a library crate.
   Why: the logic can then be tested and reused.

5. **Split a library crate from a binary crate when the library is reused or the binary pulls heavy dependencies.**
   Why: library users do not compile the binary's dependencies.

6. **Group modules by domain.** `orders/` and `billing/` say more than `utils/` and `helpers/`.
   Why: privacy boundaries then match domain boundaries.

7. **Private by default.** Use `pub(crate)` for crate-wide items and `pub` only for the public contract.
   Why: each `pub` item in a library is a semver promise.
   Check: rustc `unreachable_pub` (allow by default).

8. **Re-export the public API at the crate root** with `pub use`. Users then write `mycrate::Client`. Re-export a dependency whose types appear in your API.
   Why: you can move internal modules without breaking users.

9. **Pick one module file style per repository.** The Rust Book prefers `foo.rs` next to a `foo/` folder. Canonical's guide prefers `foo/mod.rs`.
   Sources disagree here, and neither style is safer. Pick one and enforce it, because a mix makes files hard to find.
   Check: `clippy::self_named_module_files` (restriction) or `clippy::mod_module_files` (restriction).

10. **Put the crate's `Error` type and `Result` alias in one visible place,** the crate root or `error.rs`.

11. **Watch for duplicate dependency versions** with `cargo tree --duplicates` or the bans check in `cargo deny`.
    Why: two versions of one crate cost build time and can make types incompatible.
    Check: `clippy::multiple_crate_versions` (cargo).
