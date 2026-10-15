# Testing

Rust specifics on top of the base testing rules in engineering-standards. A lint marked (pedantic), (nursery) or (restriction) is off by default.

## Contents

- Where tests live: rules 1 to 4
- Isolation and timing: rules 5 to 8
- Kinds of tests: rules 9 to 13

## Where tests live

1. **Unit tests go in a `#[cfg(test)] mod tests` at the bottom of the file they test**, with `use super::*;`.
   Why: they compile only for tests and can reach private items.

2. **Integration tests go in `tests/` and use only the public API.** Shared helpers go in `tests/common/mod.rs`.
   Why: each file in `tests/` is its own crate. A file such as `tests/common.rs` would become a test binary of its own.

3. **Keep logic in `lib.rs`, and keep `main.rs` thin.** `main` parses arguments, calls the library and maps errors to an exit code.
   Why: integration tests and doc tests need a library target.

4. **Public items have doc tests in an `# Examples` section.** Run them on their own with `cargo test --doc`, because `cargo nextest` does not run doc tests.
   Why: the example is documentation and a test in one place.

## Isolation and timing

5. **Tests share nothing.** No fixed file paths, working directory changes, environment variables, global statics or fixed ports. Use `tempfile` for files and bind to port 0 for a free port.
   Why: `cargo test` runs tests in parallel threads of one process.

6. **Fix shared state instead of serializing the suite.** Do not make `--test-threads=1` the default.
   Why: serial runs hide the coupling and slow every run.

7. **No `sleep` to wait for work.** Poll the condition with a deadline. For timer logic, pause Tokio's clock with `#[tokio::test(start_paused = true)]` (needs the `test-util` feature).
   Why: a sleep is slow on a fast machine and flaky on a slow one. Paused time is exact.

8. **Treat a flaky test as a bug.** Find the cause. Do not add retries to hide it.

## Kinds of tests

9. **Tests may `unwrap` and `expect`.** Prefer `expect` with a message, or return `Result<(), E>` and use `?`.
   Sources differ here. Canonical's guide prefers `unwrap` in tests because the panic shows the exact line. The Rust Book shows tests that return `Result`. Either is fine. When `clippy::unwrap_used` is on, set `allow-unwrap-in-tests = true` in `clippy.toml`, or deny it only for `--lib --bins`.

10. **Give `#[should_panic]` an `expected` message.**
    Why: a bare `#[should_panic]` passes on any panic, including the wrong one.
    Check: `clippy::should_panic_without_expect` (pedantic).

11. **Use property tests for parsers and invariants.** `proptest` checks round trips and invariants on many inputs and shrinks a failure to a small case. Commit the `proptest-regressions/` folder.

12. **Use snapshot tests for large or structured output.** `insta` stores the expected output, and `cargo insta review` shows the diff. Redact timestamps and IDs. CI must not write new snapshots.

13. **Test a CLI from the outside.** Use `assert_cmd` to check the exit code, stdout and stderr. Write output through `impl Write` so unit tests can capture it.
