# Antipatterns

A review checklist. Each entry names the smell and the fix, and the check where a rule exists. The reference named in brackets holds the full rule.

1. **Mutable default argument.** `def f(items=[])`. Fix: default to `None` and build the list inside. [functions-and-classes]
   Check: `B006`.

2. **`except Exception: pass`.** The error disappears with no trace. Fix: catch the named exception and handle it, log it, or use `contextlib.suppress` for one expected case. [error-handling]
   Check: `S110`, `BLE001`, and `E722` for the bare `except:` form.

3. **Star imports.** `from utils import *`. Fix: import the names you use, or the module. [style-and-naming]
   Check: `F403`, `F405`.

4. **Global mutable state.** A module-level dict or client that functions change, or a `global` statement. Fix: pass the state in, or own it in an object the caller creates. [functions-and-classes]
   Check: `PLW0603`.

5. **Stringly typed code.** A `str` stands in for a status, a mode or an ID, and an `if` chain compares it to literals. Fix: an `Enum` or a `NewType`, parsed at the boundary. [data-modeling, types-and-typing]

6. **Raw `dict` through the whole program.** `payload["user"]["email"]` five layers deep. Fix: parse the dict into a model or dataclass at the edge. [data-modeling]

7. **`cast` or `Any` to quiet the type checker.** Fix: narrow with `isinstance`, validate the data, or fix the annotation. [types-and-typing]
   Check: `ANN401` for `Any` in a signature, `PGH003` for a bare ignore. No rule flags a wrong `cast`. Review each one.

8. **Class as a namespace.** A class with only `@staticmethod` members and no state. Fix: module-level functions. [functions-and-classes]

9. **Fat class.** One class reads config, calls the network, formats output and holds the cache. Fix: split by responsibility and compose. [functions-and-classes]
   Check: `PLR0904` (preview) for too many public methods.

10. **Reinventing the standard library.** A hand loop for `any`, `sum` or `max`, a hand-written counter dict, string slicing for paths. Fix: the built-in, `collections`, `itertools` or `pathlib`. [idioms]
    Check: `SIM110`, `PERF401`, the `PTH` rules.

11. **Index loops.** `for i in range(len(xs))` to read `xs[i]`. Fix: loop over the items, with `enumerate` or `zip`. [idioms]

12. **`print` for diagnostics.** Fix: a module logger with `%` arguments. [error-handling]
    Check: `T201`.

13. **f-strings in log calls.** Fix: `logger.info("user %s", user_id)`. [error-handling]
    Check: `G004`.

14. **Translating an exception and losing the cause.** `raise ConfigError("bad")` inside `except`. Fix: `raise ConfigError(...) from err`. [error-handling]
    Check: `B904`.

15. **Naive datetimes.** `datetime.now()` or `datetime.utcnow()` with no zone. Fix: `datetime.now(UTC)`. [idioms]
    Check: `DTZ005`, `DTZ003`.

16. **Files opened without `with`.** Fix: a `with` block, or `Path.read_text`. [idioms]
    Check: `SIM115`.

17. **Getters and setters.** `get_name()` and `set_name()` around a plain attribute. Fix: a public attribute, and a `@property` when logic arrives. [functions-and-classes]

18. **`assert` for input checks.** `python -O` removes every `assert`. Fix: `if` and `raise ValueError`. Keep `assert` for tests and internal invariants. [functions-and-classes]
    Check: `S101`. Turn it off for test files, and mark a kept invariant with `# noqa: S101` and a reason.
