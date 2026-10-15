# API design

A short form of the Rust API Guidelines checklist for public items, with the guideline id in brackets. Use it for any `pub` item a caller outside the module or crate sees. A lint marked (pedantic), (nursery) or (restriction) is off by default.

## Contents

- Naming: rules 1 to 6
- Shape of the API: rules 7 to 12
- Visibility and semver: rules 13 to 16
- Documentation: rules 17 to 20

## Naming

1. **Follow the standard casing [C-CASE].** `UpperCamelCase` for types, traits and enum variants. `snake_case` for functions, methods, modules and locals. `SCREAMING_SNAKE_CASE` for constants and statics. An acronym is one word: `Uuid`, `HttpClient`.
   Check: rustc `non_camel_case_types` and `non_snake_case` (warn by default). `clippy::upper_case_acronyms` flags only all-capital names until you set `upper-case-acronyms-aggressive = true` in `clippy.toml`. It also skips public items while `avoid-breaking-exported-api` is true (the default).

2. **Conversion prefixes state the cost [C-CONV].** `as_` is a free borrowed view, `to_` does work or allocates, `into_` consumes `self`.
   Check: `clippy::wrong_self_convention`.

3. **Getters have no `get_` prefix [C-GETTER].** Write `fn name(&self)`, and `set_name` for a setter. A plain `get` is fine when there is one obvious thing to get, as in `Cell::get`.

4. **Collections offer `iter`, `iter_mut` and `into_iter` [C-ITER].** The iterator types are named `Iter`, `IterMut` and `IntoIter` [C-ITER-TY]. A type with `len` also has `is_empty`.
   Check: `clippy::len_without_is_empty`, and `clippy::iter_without_into_iter` (pedantic).

5. **A method named like a standard trait method implements the trait.** Write `impl FromStr`, not an inherent `from_str`.
   Check: `clippy::should_implement_trait`.

6. **Feature names say what they turn on [C-FEATURE].** Use `serde` or `std`. Do not use `use-serde` or `with-std`.

## Shape of the API

7. **A function with a clear receiver is a method [C-METHOD].**
   Why: callers find it with `.` and get auto-borrowing.

8. **Return values, do not write to out-parameters [C-NO-OUT].** Return a tuple or a struct. Reusing a caller's buffer is the exception.

9. **Let the caller decide on copies [C-CALLER-CONTROL].** Take ownership only when you keep the value. The ownership reference has the details.

10. **Take a generic reader or writer by value [C-RW-VALUE].** `fn load<R: Read>(reader: R)`.
    Why: `&mut R` also implements `Read`, and callers can still lend it.

11. **Keep public types `Send` and `Sync` where possible [C-SEND-SYNC].** Add a compile-time check in a test.

    ```rust
    #[test]
    fn client_is_send_and_sync() {
        fn assert_send_sync<T: Send + Sync>() {}
        assert_send_sync::<Client>();
    }
    ```

    Why: losing `Send` breaks callers that spawn tasks, and nothing else tells you.

12. **Overload operators only when the math meaning holds [C-OVERLOAD].** `Add` for a money type is fine. `Add` to push onto a list is not.

## Visibility and semver

13. **Fields are private unless the type is plain data [C-STRUCT-PRIVATE].** Use `pub(crate)` for items the rest of the crate needs and outside crates do not.
    Why: a public field fixes the layout and cannot protect an invariant.

14. **Hide complex return types [C-NEWTYPE-HIDE].** Return `impl Iterator<Item = T>` or a newtype, not a long adapter chain type.
    Why: you can change the inside without a breaking change.

15. **Mark public enums and structs that will grow `#[non_exhaustive]`.**
    Check: `clippy::manual_non_exhaustive` flags the old private-field workaround.

16. **Treat the public surface as a semver contract.** Types from your public dependencies are part of your API, and a 1.0 crate needs its public dependencies at 1.0 or later [C-STABLE]. Before a release, check the change with `cargo semver-checks`, and follow the Cargo book's SemVer compatibility chapter.

## Documentation

17. **Every public item has a doc comment, and the crate root has an overview with an example [C-CRATE-DOC].** The first line is a short summary in the third person: "Returns the user's id".
    Sources differ here. Canonical's guide shows a summary that starts with "This function". The rustdoc convention (RFC 1574) and the standard library use a third-person verb. This skill follows RFC 1574.
    Check: rustc `missing_docs` (allow by default, turn it on for libraries).

18. **Examples use `?`, not `unwrap` [C-QUESTION-MARK].** Hide a `fn main() -> Result<...>` wrapper with `#` lines.
    Why: readers copy examples, and doc tests run them.

19. **Document failure modes [C-FAILURE].** Add `# Errors` for a fallible function, `# Panics` for one that can panic and `# Safety` for an `unsafe fn`.
    Check: `clippy::missing_errors_doc` (pedantic), `clippy::missing_panics_doc` (pedantic), and `clippy::missing_safety_doc`.

20. **Link related items with intra-doc links [C-LINK].** Write `` [`Client::connect`] ``. Build docs with warnings denied to catch broken links.
    Check: `RUSTDOCFLAGS="-D warnings" cargo doc --no-deps`.
