# Idioms

Loops, comprehensions, resources, strings and paths, written the way Python expects. A Ruff rule named here needs its group or code in `lint.select`. A rule marked (preview) also needs `preview = true`.

## Contents

- Loops and collections: rules 1 to 8
- Resources and paths: rules 9 to 12
- Strings, dates and checks: rules 13 to 19

## Loops and collections

1. **Loop over the items, not the indexes.** Use `enumerate` when you need the position and `zip` to walk two sequences together.

   ```python
   names = ["ada", "bob"]
   # wrong
   for i in range(len(names)):
       print(i, names[i])
   # right
   for i, name in enumerate(names):
       print(i, name)
   ```

   Check: `PLC0206` for dict loops that index with the key, `SIM118` for `in d.keys()`.

2. **Pass `strict=True` to `zip` when the inputs must have the same length.**
   Why: plain `zip` stops at the shortest input and drops the rest without a word.
   Check: `B905`.

3. **Build a list, set or dict with a comprehension** when the loop only maps and filters. Go back to a `for` loop when the body has side effects, needs more than one `if`, or the comprehension needs two levels of nesting. Do not call a function inside a comprehension only for its side effect.
   Why: a short comprehension states the result in one place. A long one is harder to read than the loop.
   Check: `PERF401` and `PERF403` find loops that should be comprehensions. The `C4` rules (`C400` to `C420`) find comprehensions and calls that do extra work.

4. **Use a generator expression when the result is consumed once,** for example by `sum`, `any`, `all` or `max`. Do not build a list first. `str.join` is the exception: it builds a list from its argument anyway, and a list comprehension is faster there.

   ```python
   orders = [120, 80, 45]
   # wrong: builds a list only to throw it away
   total = sum([price for price in orders])
   # right
   total = sum(price for price in orders)
   assert total == 245
   ```

   Check: `C419` covers `any` and `all`, and `sum`, `min` and `max` only in preview.

5. **Reach for the built-ins and `itertools` before a hand loop.** `any`, `all`, `sum`, `min`, `max`, `sorted(key=...)`, `itertools.chain`, `itertools.pairwise`, `itertools.batched` (3.12+), `collections.Counter` and `defaultdict`.
   Check: `SIM110` for a loop that `any` or `all` replaces, `RUF007` for `zip(xs, xs[1:])`.

6. **Unpack instead of indexing.** `first, *rest = parts` and `for key, value in mapping.items():`. Use `next(iter(x))` to take the first item of an iterable.
   Check: `RUF015`.

7. **Use `dict.get`, `setdefault` and `defaultdict` for lookups with a fallback.** Do not check `in` and then index.
   Check: `SIM401` for an `if key in d` block that `d.get` replaces, `RUF019` for a redundant key check.

8. **Do not change a list or dict while you loop over it.** Loop over a copy, or build a new collection.
   Why: removing items during iteration skips items, and a dict raises `RuntimeError`.

## Resources and paths

9. **Open every resource in a `with` block:** files, locks, sockets, database connections, temporary folders. When two resources open at the same point, put both in one `with` statement.

   ```python
   import tempfile
   from pathlib import Path

   with tempfile.TemporaryDirectory() as tmp:
       target = Path(tmp) / "out.txt"
       with target.open("w", encoding="utf-8") as fh:
           fh.write("done\n")
       assert target.read_text(encoding="utf-8") == "done\n"
   ```

   Why: the resource closes when the block ends, also when an exception is raised.
   Open a resource in the function that uses it, and return data to the caller. Return an open handle only when the caller owns closing it.
   Check: `SIM115` for `open` outside a `with`, `SIM117` for nested `with` blocks.

10. **Write your own context manager with `contextlib.contextmanager`** for paired setup and cleanup. Put the cleanup in a `finally` around the `yield`. Use `contextlib.ExitStack` for a number of resources known only at run time. To ignore one named exception, see error-handling, rule 3.

11. **Use `pathlib.Path` for file system paths.** Join with `/`, read with `read_text`, list with `iterdir` or `glob`.
    Why: one object type with methods for every path task, and no string joining bugs across platforms.
    Check: the `PTH` rules, for example `PTH118` for `os.path.join` and `PTH123` for `open()`.

12. **Pass `encoding="utf-8"` when you open text files.**
    Why: the default encoding depends on the platform and the locale until Python 3.15 makes UTF-8 the default.
    Check: `PLW1514` (preview).

## Strings, dates and checks

13. **Format strings with f-strings.** Keep `%` style only in logging calls (error-handling, rule 13). Use `str.join` to build a string from many parts.
    Check: `UP031` for `%` formatting, `UP032` for `.format`.

14. **Use timezone-aware datetimes.** `datetime.now(UTC)` (3.11+, or `timezone.utc`) and `zoneinfo.ZoneInfo` for local zones. Store and log UTC.
    Why: a naive datetime has no zone. Ordering or subtracting it against an aware datetime raises `TypeError`, and `==` between the two is `False`.
    Check: the `DTZ` rules, for example `DTZ005` for `datetime.now()` with no zone.

15. **Ask forgiveness for one narrow operation (EAFP).** Error-handling rules 2 and 4 say how narrow the `try` and the `except` are.

    ```python
    config = {"port": "8080"}
    try:
        port = int(config["port"])
    except (KeyError, ValueError):
        port = 8000
    assert port == 8080
    ```

    Check first (LBYL) when the action is costly or cannot be undone, when failure is the common case, or when the check is cheap and nothing can change between check and use.
    Sources agree on EAFP as the default. The Real Python tutorial on the topic lists these cases as the exceptions. This skill keeps both.
    Why: a check followed by an action races with other processes (a file can vanish between `exists()` and `open()`).

16. **Use the `__name__ == "__main__"` guard in a module that is also run as a script.** Put the work in a `main()` function and call it from the guard.
    Why: importing the module, for example from a test, then does not run the script.

17. **Use the walrus operator `:=` only where it removes a repeated call or a pre-loop read.** `while chunk := stream.read(8192):` is a good use. Do not nest it inside complex expressions.

18. **Write a range check as one chained comparison.** `if 0 <= rate <= 1:` reads as the math does.
    Check: `PLR1716` for `a < b and b < c`.

19. **Branch on one value with `match` or a dict of handlers** when an `if` and `elif` chain grows past a few cases.
    Why: each case sits on its own line, and adding a case does not touch the others.
