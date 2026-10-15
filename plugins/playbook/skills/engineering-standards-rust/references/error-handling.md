# Error handling

How to return and wrap errors, and when a panic is the right call. A lint marked (pedantic), (nursery) or (restriction) is off by default.

## Contents

- Typed errors and panics: rules 1 to 5
- Error types: rules 6 to 11
- Propagation and reporting: rules 12 to 17

## Typed errors and panics

1. **A library returns typed errors.** Use `thiserror` or a hand-written enum. An application may use `anyhow` or `eyre` in `main` and in glue code that only reports the error.
   Sources disagree here. Canonical's guide allows type-erased errors only in throwaway prototypes. Most other sources accept `anyhow` in applications. This skill takes the middle rule: any module whose callers branch on the failure returns a typed error, even inside an application, because a caller can only handle a failure it can name.

2. **No `unwrap` or `expect` on a path that can fail at run time** in library or production code. Propagate with `?`.
   Why: a panic takes the decision away from the caller and shows the user a crash.
   Check: `clippy::unwrap_used` (restriction) and `clippy::expect_used` (restriction). One way is to deny them for `--lib --bins` only, which leaves tests free to unwrap.

3. **Panic only for bugs.** A broken invariant or an unreachable state may panic. Input that the caller or the user controls must return an error. Test code and examples may panic.
   Check: `clippy::panic` (restriction), `clippy::indexing_slicing` (restriction) and `clippy::arithmetic_side_effects` (restriction) for code that handles untrusted input.

4. **Prove a panic next to where it can happen.** When you keep an `expect`, the reason it cannot fail sits next to it: a check right above it, or a unit test for a constant input. The message names the invariant. Where `clippy::expect_used` is denied, add a scoped `#[expect]` with a reason. Use `unreachable!` for states that cannot happen and `todo!` only for unfinished work.

   ```rust
   // wrong: the reader cannot tell why this is safe
   let re = Regex::new(PATTERN).unwrap();
   // right
   #[expect(clippy::expect_used, reason = "PATTERN is a literal, a unit test compiles it")]
   let re = Regex::new(PATTERN).expect("PATTERN is a valid regex literal");
   ```

   Why: a guarantee checked nearby survives a refactor. A promise made by a distant caller does not.
   Check: `clippy::todo` (restriction) keeps `todo!` out of merged code.

5. **Decide the panic strategy for the program.** Catch panics at thread or task boundaries where the program must keep running, and know whether the release profile sets `panic = "abort"` (no unwinding, no `Drop` cleanup).
   Why: panics happen in production, and recovery only works where you planned for it.

## Error types

6. **One error type per crate or major module.** It wraps the errors of the dependencies it calls. Keep those dependency types out of the public signature, for example with an opaque struct over a private enum and `#[error(transparent)]`.
   Why: a public `sqlx::Error` ties every caller to your dependency and its version.

7. **Mark public error enums `#[non_exhaustive]`.**
   Why: you can add a failure mode later without breaking every caller's `match`.

8. **Public error types implement `Debug`, `Display` and `std::error::Error`, and are `Send + Sync + 'static`.** Do not use `()` or a bare `String` as a public error type.
   Why: callers can then box, downcast, send the error across threads and wrap it in `anyhow`.
   Check: `clippy::result_unit_err`.

9. **Expose the cause through `source()`, and keep it out of the `Display` text.**

   ```rust
   #[derive(Debug, thiserror::Error)]
   pub enum ConfigError {
       // wrong: #[error("cannot read {path}: {source}")]
       #[error("cannot read {path}")]
       Read { path: PathBuf, #[source] source: std::io::Error },
   }
   ```

   Sources disagree here. Canonical's guide shows `{source}` in the message and also exposes `source`. The Rust API Guidelines and the error-handling articles say to do only one of the two. This skill picks `source()`, because chain reporters such as `anyhow`'s `{:#}` print every cause and the message would appear twice.

10. **Error messages are short and lowercase, with no trailing period.** For example `cannot open config`.
    Why: wrapped messages read as one line: `loading settings: cannot open config: permission denied`.

11. **Keep the error type small.** Box a large payload inside the variant.
    Why: every `Result<T, E>` is as big as its largest variant, on the success path too.
    Check: `clippy::result_large_err`, `clippy::large_enum_variant`.

## Propagation and reporting

12. **Add context at each fallible step.**

    ```rust
    // wrong: "No such file or directory" with no path
    let text = fs::read_to_string(&path)?;
    // right, in an application
    let text = fs::read_to_string(&path)
        .with_context(|| format!("reading {}", path.display()))?;
    // right, in a library
    let text = fs::read_to_string(&path)
        .map_err(|source| ConfigError::Read { path: path.clone(), source })?;
    ```

    Why: the bare I/O error does not say which file or which step failed.
    Check: `clippy::map_err_ignore` (restriction) flags `map_err(|_| ...)`, which drops the cause.

13. **Use `From` and `#[from]` only where one source type maps to one variant.**
    Sources disagree here. Andrew Gallant's guide uses `From` to make `?` convert errors. Simonas Kazlauskas argues against `From` on error types. This skill allows it only when the mapping has one meaning, because a blanket `From<io::Error>` merges two different I/O failures into one variant with no context.

14. **No `Box<dyn Error>` in a public library signature.** It is fine in binaries and tests.
    Why: callers get nothing to match on.

15. **`Option` for normal absence, `Result` when the caller needs the reason.** Convert with `ok_or_else` when building the error costs something.
    Check: `clippy::manual_ok_or`, and `clippy::or_fun_call` (nursery).

16. **Do not drop an error in silence.** `let _ = ...`, `.ok()` or `.unwrap_or_default()` on a `Result` needs a comment that says why the error does not matter.
    Check: `clippy::let_underscore_must_use` (restriction) for `let _ =`, `clippy::unused_result_ok` (restriction) for `.ok()`, and review for `.unwrap_or_default()`. rustc `unused_must_use` catches only a bare call whose result is ignored.

17. **`main` returns `Result`.** Let `?` reach `main`, or map the error to an `ExitCode`, and the user sees the message and its causes. A network service is different: it logs the full chain on the server and sends the client a short message, because the chain holds paths and internal details.
    The `# Errors` and `# Panics` doc sections are in the API design reference, rule 19.
