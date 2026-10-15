# Performance

Measure first, then fix the hot path. A Ruff rule named here needs its group or code in `lint.select`. A rule marked (preview) also needs `preview = true`.

## Rules

1. **Profile before you optimize.** Use `cProfile` (`python -m cProfile -s cumtime app.py`) or a sampling profiler such as `py-spy` on a running process. Time one small piece with `python -m timeit`.
   Why: the slow part is rarely where you guess, and a change that does not touch the hot path adds complexity for nothing.

2. **Keep a benchmark for code you tune,** and compare before and after on the same machine. A plugin such as `pytest-benchmark` keeps the numbers next to the tests.

3. **Pick the data structure for the access pattern.** A `set` or `dict` for membership and lookup, `collections.deque` for a queue, `heapq` for a priority queue, `bisect` on a sorted list for range search.

   ```python
   allowed = {"admin", "editor"}  # O(1) membership, a list is O(n)
   assert "editor" in allowed
   ```

   Check: `PLR6201` (preview) for a membership test against a list or tuple literal.

4. **Stream large data.** Iterate over a file line by line, use generators, and process in batches (`itertools.batched`, 3.12+). Do not load the whole input when you only need one pass.
   Check: `FURB129` for `readlines()` in a `for` loop.

5. **Build strings with `str.join` or a list of parts.** Do not grow a string with `+=` in a loop.
   Why: CPython's in-place append is an implementation detail. Other runtimes copy the string each time.

6. **Hoist work out of loops.** Compile a regex once at module level, look up an attribute once before the loop, and do not call `list(...)` on something you only iterate.
   Check: `PERF101` for a needless `list()` cast, `PERF102` for `.items()` when you use only keys or values, `PERF401` for a loop that should be a comprehension.

7. **Cache pure functions with `functools.cache` or `lru_cache(maxsize=...)`.** Cache only functions whose result depends on the arguments alone. Bound the cache when the argument space is large. Methods are covered in functions-and-classes, rule 17.

8. **Use the built-ins and the standard library.** `sum`, `min`, `max`, `sorted`, `collections.Counter`, `itertools` and `math.fsum` run in C.

9. **Move hot numeric work to a library built for it.** NumPy, pandas or Polars for arrays and tables. Do not loop over array elements in Python.

10. **Use `__slots__` (or `slots=True` on a dataclass) for classes with very many instances.**
    Why: each instance drops its `__dict__` and uses less memory.

11. **Try a newer Python before you rewrite code.** CPython 3.11 made many workloads faster, and later releases kept adding speedups. Measure your own workload on the new version.

12. **Write the clear version first.** Keep an optimization only when the benchmark shows a gain on the hot path, and leave a comment with the numbers.
