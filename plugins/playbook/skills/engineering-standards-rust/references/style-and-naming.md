# Style and naming

Local style that `rustfmt` does not fix: names, imports, pattern matching, layout inside a file and comments. The project's existing style wins over this list. A lint marked (pedantic), (nursery) or (restriction) is off by default.

## Rules

1. **Naming follows the base rule in engineering-standards** and the project's existing scheme. Canonical's guide adds house rules such as UK spelling and single-letter type parameters. Adopt those only as a team decision.

2. **No glob imports from other crates.** `use super::*;` in a `#[cfg(test)]` module is the usual exception.
   Why: after an upgrade, a new name in the glob can clash or hide where a name came from.
   Check: `clippy::wildcard_imports` (pedantic), `clippy::enum_glob_use` (pedantic).

3. **Group imports: `std`, then external crates, then `crate`, `super` and `self`.** Leave a blank line between groups.
   Why: the reader sees where each name comes from.
   Check: the rustfmt option `group_imports = "StdExternalCrate"` does this, but it needs nightly rustfmt. On stable, keep the groups by hand.

4. **Use `let else` and `if let` in place of a check followed by `unwrap`.**

   ```rust
   // wrong
   if user.is_some() { greet(user.unwrap()); }
   // right
   if let Some(user) = user { greet(user); }
   // right, when the rest of the function needs the value
   let Some(user) = user else { return Ok(()) };
   ```

   Check: `clippy::unnecessary_unwrap`, and `clippy::manual_let_else` (pedantic).

5. **Destructure a struct with every field when the code must handle every field.** A hand-written `PartialEq`, `Hash` or `Ord` is the usual case. Write `field: _` for a field you skip on purpose.
   Why: a new field breaks the build and forces a decision.

6. **Name tuple parts by destructuring.** Prefer `let (min, max) = range;` to `range.0` and `range.1`. A single-field newtype may use `.0` inside its own `impl`.

7. **Let `if`, `match` and `loop` produce the value.** Avoid `let x;` followed by an assignment in each branch.
   Check: `clippy::needless_late_init`.

8. **Use `.copied()` and `.cloned()` in place of `.map(|x| *x)`.**
   Sources disagree here. One example in Canonical's guide uses `.map(|x| *x)`. Clippy flags that by default. This skill follows Clippy.
   Check: `clippy::map_clone`.

9. **Inline simple format arguments.** Write `format!("{path}/{name}")`.
   Check: `clippy::uninlined_format_args` (pedantic).

10. **Order a file from the top down.** Public and important items come first, helpers below. An inherent `impl` sits right after its type.
    Why: a reader can stop once they reach the details.

11. **Limit shadowing.** Rebinding the same value in the same scope is fine (`let line = line.trim();`). Avoid long chains of shadows that change the type more than once.
    Check: `clippy::shadow_unrelated` (restriction) for strict projects.

12. **Comments explain why.** Do not restate the code. An `unsafe` block has a `// SAFETY:` comment. Doc comment rules are in the API design reference, rule 17.
    Check: `clippy::undocumented_unsafe_blocks` (restriction) for the `SAFETY` comment. The rest is review.
