# Iterators and collections

Iterator chains, `collect`, and how to pick a collection. A lint marked (pedantic), (nursery) or (restriction) is off by default.

## Rules

1. **Prefer iterator adapters to index loops.**

   ```rust
   // wrong
   for i in 0..items.len() { total += items[i].price; }
   // right
   let total: u64 = items.iter().map(|item| item.price).sum();
   ```

   Why: no off-by-one errors, and fewer bounds checks.
   Check: `clippy::needless_range_loop`.

2. **Do not `collect` only to loop again or to call `len`, `is_empty` or `contains`.**
   Why: the extra `Vec` costs an allocation and gives nothing back.
   Check: `clippy::needless_collect` (nursery).

3. **Return `impl Iterator<Item = T>` when callers only loop over the result.**
   Why: the caller decides whether to allocate.

4. **Collect `Result` items into `Result<Vec<T>, E>`.**

   ```rust
   let ports: Result<Vec<u16>, _> = args.iter().map(|a| a.parse::<u16>()).collect();
   ```

   Why: it stops at the first error with no hand-written loop. The same works for `Option`.
   Use `partition` when you need every error. `filter_map(Result::ok)` drops errors in silence, so use it only when that is the intent.

5. **Pick `iter`, `iter_mut` or `into_iter` on purpose.** Borrow by default. `into_iter()` and `for x in v` move `v`.
   Check: `clippy::into_iter_on_ref`.

6. **Clone after you filter.** Call `.cloned()` or `.copied()` after `filter` or `take`, not before.
   Why: only the items you keep get cloned.
   Check: `clippy::iter_overeager_cloned`.

7. **Use `extend` to add items to an existing collection.**
   Why: it reuses the buffer and can reserve space once.
   Check: `clippy::extend_with_drain`.

8. **Use the entry API for insert-or-update.**

   ```rust
   // wrong: two lookups
   if !counts.contains_key(&word) { counts.insert(word.clone(), 0); }
   *counts.get_mut(&word).unwrap() += 1;
   // right
   *counts.entry(word).or_insert(0) += 1;
   ```

   Check: `clippy::map_entry`.

9. **Use `retain` to remove items in place.**
   Check: `clippy::manual_retain`.

10. **Use `Vec` and `HashMap` unless the access pattern asks for another type.** `VecDeque` for a queue with work at both ends, `BTreeMap` for sorted keys or range queries, `BinaryHeap` for a priority queue. `LinkedList` is rarely the right choice.
    Check: `clippy::linkedlist` (pedantic).

11. **Do not depend on `HashMap` order.** Sort the output, or use `BTreeMap`, when order matters.
    Why: iteration order changes between runs and breaks tests and diffs.

12. **A custom iterator implements `size_hint`** (or `ExactSizeIterator` when the length is known).
    Why: `collect` and `extend` can allocate once.
