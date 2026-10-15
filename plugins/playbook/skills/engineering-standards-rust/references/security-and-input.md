# Security and input

Rust specifics for untrusted input, secrets and external processes. The base rule from engineering-standards still applies: validate input at the boundary where it enters. A lint marked (pedantic), (nursery) or (restriction) is off by default.

## Rules

1. **Parse input into typed values at the boundary.** Deserialize with `serde` into a struct, then convert into a validated newtype with `TryFrom`. Code past the boundary takes the newtype.
   Why: code inside the program then cannot see an unchecked value.

2. **Do not panic on input you do not control.** Use `get()` in place of indexing, and `checked_`, `saturating_` or `wrapping_` arithmetic on sizes and offsets from input.
   Why: a panic on a crafted request is a denial of service, and integer overflow wraps in silence in release builds.
   Check: `clippy::indexing_slicing` (restriction), `clippy::arithmetic_side_effects` (restriction), `clippy::string_slice` (restriction).

3. **Limit the size of what you read.** Set a body size limit on HTTP handlers, use `Read::take` on streams, and cap collections built from input.
   Why: an unbounded `read_to_end` lets one client use all memory.

4. **Stream large files and bodies.** Do not load them whole.
   Why: memory stays bounded.

5. **Pass process arguments one by one.** Use `Command::new(program).arg(value)`. Do not build a string for `sh -c` from input.
   Why: a shell reads the string as code.

6. **Bind SQL parameters.** Use the query builder or bind API (`sqlx::query!` and `.bind()`, or Diesel). Do not build SQL with `format!`.
   Why: string-built SQL is open to injection.

7. **Keep secrets out of `Debug`, logs and errors.** Wrap secrets in a type such as `secrecy::SecretString`, or write `Debug` by hand.
   Why: a derived `Debug` prints the secret into every log line and panic message that shows the struct.

8. **A network service logs the full error and sends the client a short message.** Map errors to responses in one place, for example one `IntoResponse` impl in axum or one `ResponseError` impl in actix-web.
   Why: the error chain holds file paths, SQL text and dependency details that a client must not see.

9. **Put a timeout on each outbound call.** Use the client's timeout setting or `tokio::time::timeout`.
   Why: a slow peer can otherwise hold a task and its resources forever.
