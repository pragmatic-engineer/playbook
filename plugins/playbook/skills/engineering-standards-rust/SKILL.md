---
name: engineering-standards-rust
description: Rust rules for code and reviews, covering ownership, error handling, types, API design and Clippy lints. Use when writing or reviewing Rust, or a diff that touches .rs files.
---

# Engineering Standards: Rust

Language-specific engineering standards for Rust, derived from the team's base standards. RFC 2119 keywords (MUST, SHOULD) carry their standard meanings.

**REQUIRED BACKGROUND:** engineering-standards owns the general rules (PR readiness, size, test types and requirements, the mocking philosophy, deployment). Read it first. This skill adds only the Rust specifics and does not restate the base.

## How to use this skill

- The index below holds one line per rule. Each topic names a reference file in this skill's `references/` folder and says when to load it. Load only the files the work touches.
- When you review a diff that touches `.rs` files, load the reference for each topic the diff touches. Cite the rule and the lint or command that checks it.
- The project's own setup wins. When `Cargo.toml`, `clippy.toml`, CI or the contributing guide sets a rule, follow it over this skill.
- A Clippy lint in the `pedantic`, `nursery` or `restriction` group is off by default. The references mark those lints. Turn one on in the `[lints.clippy]` table of `Cargo.toml` (or `[workspace.lints.clippy]`) before you rely on it.

## Rules index

### Ownership and borrowing

- Borrow to read, own to keep. Take `&str`, `&[T]` and `&Path` in parameters.
- A clone MUST have a reason a reviewer can see. Shorten the borrow or use `mem::take` before you clone.
- Prefer one owner. Use an arena with indices for trees and graphs, and `Rc<RefCell<T>>` only when sharing and mutation are both real.

Load `references/ownership-and-borrowing.md` when you fix a borrow checker error, add a `.clone()`, `Rc`, `RefCell` or a lifetime parameter, or choose a parameter type.

### Error handling

- A library MUST return typed errors (`thiserror` or a hand-written enum). An application MAY use `anyhow` where callers only report the error.
- Library and production code MUST NOT `unwrap` or `expect` on a path that can fail at run time. Panic only for a broken invariant, and say which one in the `expect` message.
- Add context at each fallible step. Expose the cause through `source()` and keep it out of the `Display` text.

For example, this repository's CI denies `clippy::unwrap_used` and `clippy::expect_used` for `--lib --bins` and lets tests unwrap.

Load `references/error-handling.md` when you define an error type, write `?`, `unwrap`, `expect` or `panic!`, or add a dependency whose errors cross your API.

### Types and traits

- Wrap meaningful primitives in newtypes and validate once in the constructor.
- Use an enum in place of a `bool` parameter or a string that holds one of a fixed set of values.
- Implement `From` and `TryFrom`, avoid lossy `as` casts, and derive the common traits on public types.
- Write a concrete type first. Make it generic when a second type needs it.

Load `references/types-and-traits.md` when you add a type or a trait, or a conversion between types.

### API design

- Public items follow the Rust API Guidelines: `as_`, `to_` and `into_` prefixes, getters with no `get_`, private fields, `#[non_exhaustive]` on types that will grow.
- Public items have doc comments with examples that use `?`, and `# Errors`, `# Panics` and `# Safety` sections where they apply.

Load `references/api-design.md` when you add or change a `pub` item, a crate feature or a doc comment, or prepare a library release.

### Style and naming

- Copy the project's naming scheme. No glob imports from other crates. Use `let else` and `if let` over a check then `unwrap`.

Load `references/style-and-naming.md` when you name things, write imports or `match` blocks, or review a diff for readability.

### Antipatterns

Load `references/antipatterns.md` when you review a Rust diff. It is a checklist of the most common smells, each with a fix and a check.

## Sources

The source list started from [mre/idiomatic-rust](https://github.com/mre/idiomatic-rust), curated by Matthias Endler and contributors and dedicated to the public domain under CC0 1.0. The rules here are a summary in our own words. No text is copied from the sources below.

- [Rust API Guidelines](https://rust-lang.github.io/api-guidelines/) (MIT or Apache-2.0)
- [The Rust Programming Language](https://doc.rust-lang.org/book/) (MIT or Apache-2.0)
- [Rust Design Patterns](https://rust-unofficial.github.io/patterns/) (MPL-2.0)
- [Canonical's Rust best practices](https://canonical.github.io/rust-best-practices/) (GPL-3.0, ideas restated). Where it differs from the other sources, the reference says so and names the rule this skill picks.
- [Clippy lint list](https://rust-lang.github.io/rust-clippy/stable/index.html). Lint groups were checked against Clippy 1.97.
- [thiserror](https://github.com/dtolnay/thiserror) and [anyhow](https://github.com/dtolnay/anyhow) by David Tolnay
- [Error Handling in Rust](https://burntsushi.net/rust-error-handling/) by Andrew Gallant
- [To panic or not to panic](https://www.ncameron.org/blog/to-panic-or-not-to-panic/) by Nick Cameron
- [Context-preserving error handling](https://kazlauskas.me/entries/errors) by Simonas Kazlauskas
- [Designing Error Types in Rust Libraries](https://d34dl0ck.me/rust-bites-designing-error-types-in-rust-libraries/index.html) by Sven Kanoldt
- [Elegant Library APIs in Rust](https://deterministic.space/elegant-apis-in-rust.html) by Pascal Hertleif
- [When should I use String vs &str?](https://steveklabnik.com/writing/when-should-i-use-string-vs-str/) by Steve Klabnik
- corrode.dev articles by Matthias Endler: [Be Simple](https://corrode.dev/blog/simple/), [Defensive Programming](https://corrode.dev/blog/defensive-programming/), [Pitfalls of Safe Rust](https://corrode.dev/blog/pitfalls-of-safe-rust/)
