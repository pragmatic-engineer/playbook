# Ownership and borrowing

Who owns a value, how long a borrow lives, and when shared ownership is the right tool. A lint marked (pedantic), (nursery) or (restriction) is off by default.

## Rules

1. **Borrow to read, own to keep.** Take `&T` when the function only reads the value. Take `T` when it stores or consumes it.
   Why: the caller decides when to copy or allocate.
   Check: `clippy::needless_pass_by_value` (pedantic).

2. **Take the borrowed form in parameters.** Use `&str` over `&String`, `&[T]` over `&Vec<T>` and `&Path` over `&PathBuf`.

   ```rust
   // wrong
   fn word_count(text: &String) -> usize { text.split_whitespace().count() }
   // right
   fn word_count(text: &str) -> usize { text.split_whitespace().count() }
   ```

   Why: deref coercion accepts more callers, with one less indirection.
   Check: `clippy::ptr_arg`.

3. **Ask for ownership in the signature when you keep the value.** A function that takes `&str` and then calls `.to_string()` to store it hides a copy.
   Why: the caller can pass a `String` it no longer needs and skip the copy.
   Check: in review, look for `.to_owned()`, `.to_string()` or `.clone()` on a borrowed parameter that ends up in a struct.

4. **Strings: own in fields, borrow in parameters.** Store `String`, take `&str`, and return `String` unless the result borrows from an input.
   Why: this covers most code without lifetime parameters on your types.

5. **Use `Cow<'_, T>` when a value is usually borrowed and sometimes built.**
   Why: you allocate only on the path that changes the value.

6. **Move a value out of `&mut` with `mem::take`, `mem::replace` or `Option::take`.**

   ```rust
   // wrong: copies the buffer, then clears it
   let batch = self.pending.clone();
   self.pending.clear();
   // right
   let batch = std::mem::take(&mut self.pending);
   ```

   Why: no copy, and the borrow checker accepts it.
   Check: `clippy::mem_replace_with_default` and `clippy::mem_replace_option_with_none` catch the `mem::replace` form. No lint catches clone then clear, so look for it in review.

7. **Shrink the borrow before you clone.** End the borrow in a block, copy a small `Copy` value out first, or split a large struct into parts that borrow on their own.
   Why: a borrow lasts for its scope. A shorter scope removes the conflict with no copy.
   Check: in review, ask for a reason on each `.clone()` added to fix a borrow error (E0499, E0502, E0505).

8. **A clone must be deliberate.** Cloning an `Arc`, a small value or a value off the hot path is fine. A clone whose only job is to silence the borrow checker is a design smell.
   Sources disagree here. corrode.dev accepts clones that keep code simple, and Rust Design Patterns lists clone-to-satisfy-the-borrow-checker as an anti-pattern. This skill asks for a visible reason on each clone of shared or mutable state, because two copies of that state can drift apart.
   Check: `clippy::redundant_clone` (nursery), `clippy::implicit_clone` (pedantic), and `clippy::clone_on_ref_ptr` (restriction) to write `Arc::clone(&x)` so the cheap clone is obvious.

9. **Prefer single ownership.** Use `Rc` or `Arc` only when ownership is shared, and add `RefCell` or `Mutex` only when that shared value must change. Message passing between owners is another option.
   Why: `RefCell` moves the borrow check to run time and panics on a conflict.
   Check: review. No lint flags `Rc<RefCell<_>>`, so ask why each one cannot be a plain owner.

10. **Use an arena for trees and graphs.** Keep nodes in a `Vec<Node>` and link them by index (or use a crate such as `slotmap`).
    Why: one owner for all nodes, no runtime borrow panics and no reference cycles.

11. **Use `Weak` for back links** (child to parent, observer to subject) when you do share ownership.
    Why: an `Rc` cycle leaks memory, and the compiler does not reject it.

12. **Prefer `Rc<str>` and `Arc<[T]>` to `Rc<String>` and `Arc<Vec<T>>`** for shared data that does not change.
    Why: one less allocation and indirection.
    Check: `clippy::rc_buffer` (restriction).

13. **Write lifetimes only where elision cannot work.** When a struct needs a lifetime parameter, first ask whether it should own its data. With more than one lifetime, name them after the data (`'src`, `'req`).
    Why: a lifetime on a type spreads to every user of that type.
    Check: `clippy::needless_lifetimes`, and `clippy::elidable_lifetime_names` (pedantic).

14. **Keep `mut` in the smallest scope.** Build the value in a block with a local `let mut`, then bind the result without `mut`.

    ```rust
    let ids = {
        let mut ids: Vec<_> = rows.iter().map(|r| r.id).collect();
        ids.sort_unstable();
        ids.dedup();
        ids
    };
    ```

    Why: a later change cannot mutate the value by accident.
