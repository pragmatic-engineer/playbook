# Antipatterns

A review checklist. Each entry names the smell and the fix, and the check where a lint exists. The reference named in brackets holds the full rule.

1. **Clone to quiet the borrow checker.** Fix: shorten the borrow, use `mem::take`, or split the struct. [ownership-and-borrowing]
   Check: `clippy::redundant_clone` (nursery), `clippy::implicit_clone` (pedantic).

2. **`Rc<RefCell<T>>` as the default for shared data.** Fix: one owner, an arena with indices, or message passing. [ownership-and-borrowing]
   Check: review. No lint flags `Rc<RefCell<T>>`.

3. **Stringly typed code.** A `String` or an integer stands in for an ID, a unit or a closed set of options. Fix: a newtype or an enum, parsed at the boundary. [types-and-traits]

4. **Boolean arguments.** `send(msg, true, false)`. Fix: an enum or an options struct. [types-and-traits]
   Check: `clippy::fn_params_excessive_bools` (pedantic), with `max-fn-params-bools` lowered in `clippy.toml`.

5. **Panics in a library on caller input.** `unwrap`, `expect`, `panic!` or slice indexing on data the caller controls. Fix: return a `Result` and document the error. [error-handling]
   Check: `clippy::unwrap_used` (restriction), `clippy::expect_used` (restriction), `clippy::indexing_slicing` (restriction).

6. **Errors dropped in silence.** `let _ = save();` or `.ok()` with no comment. Fix: handle it, log it with context, or return it. [error-handling]
   Check: `clippy::let_underscore_must_use` (restriction), `clippy::unused_result_ok` (restriction).

7. **Over-generic code.** Traits and type parameters for an implementation that does not exist yet. Fix: write the concrete type, and make it generic when a second type arrives. [types-and-traits]

8. **`Deref` to fake inheritance.** Fix: composition with explicit methods, or a trait. [types-and-traits]

9. **`#![deny(warnings)]` in source.** A new compiler or Clippy release adds lints and breaks the build on the next toolchain update. Fix: deny warnings in CI, or deny named lints in the `[lints]` table of `Cargo.toml`. [tooling-and-ci]

10. **Truncating `as` casts.** `len as u32`. Fix: `u32::try_from(len)` with the error mapped to your type. [types-and-traits]
    Check: `clippy::cast_possible_truncation` (pedantic).

11. **`&String`, `&Vec<T>` or `&Box<T>` parameters.** Fix: `&str`, `&[T]`, `&T`. [ownership-and-borrowing]
    Check: `clippy::ptr_arg`, `clippy::borrowed_box`.

12. **Lifetimes added until the error goes away.** Fix: let the struct own its data, or restructure the borrow. [ownership-and-borrowing]
    Check: `clippy::needless_lifetimes`.

13. **`_` arms and `..Default::default()` that hide a new case.** A new enum variant or struct field compiles with no decision made. Fix: exhaustive matches on your own enums, and full struct literals where each field matters. [types-and-traits]
    Check: `clippy::wildcard_enum_match_arm` (restriction).

14. **Public fields on a type with an invariant.** Fix: private fields, a validating constructor and accessors. [api-design]

15. **A secret in `Debug` output.** A derived `Debug` on a struct that holds a token or password. Fix: a manual `Debug` or a secret wrapper type. [security-and-input]
