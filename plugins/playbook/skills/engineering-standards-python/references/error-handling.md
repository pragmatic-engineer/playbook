# Error handling

How to raise, catch, chain and log errors. A Ruff rule named here needs its group or code in `lint.select`. A rule marked (preview) also needs `preview = true`.

## Contents

- Catching: rules 1 to 6
- Raising: rules 7 to 11
- Logging: rules 12 to 17

## Catching

1. **No bare `except:`.** It also catches `KeyboardInterrupt` and `SystemExit`, so Ctrl+C and `sys.exit` stop working.
   Check: `E722`.

2. **Catch the narrowest exception the code can handle, at the level that can act on it.** Low-level code raises. The edges (a request handler, a worker loop, `main`) catch, log and report. `except Exception` belongs only at such an edge.
   Check: `BLE001`.

3. **Do not pass on an exception in silence.** Handle it, log it, or let it rise. When you mean to ignore one named exception, write `contextlib.suppress(FileNotFoundError)` so the intent is visible.

   ```python
   import contextlib
   from pathlib import Path

   # wrong
   try:
       Path("/tmp/missing.lock").unlink()
   except Exception:
       pass
   # right
   with contextlib.suppress(FileNotFoundError):
       Path("/tmp/missing.lock").unlink()
   ```

   Check: `S110` for `try`, `except`, `pass`, `S112` for `try`, `except`, `continue`, `SIM105` for the `suppress` form.

4. **Keep the `try` block to the one call that can fail.** Move the code that runs on success into `else`.
   Why: a wide `try` catches the same exception from a line you did not mean to guard and hides the bug.
   Check: `TRY300`.

5. **Put cleanup in `finally` or a `with` block.** Do not `return`, `break` or `continue` inside `finally`.
   Why: a jump out of `finally` throws away the exception in flight. Python 3.14 warns about it (PEP 765).
   Check: `B012`.

6. **Use `except*` and `ExceptionGroup` only where code raises groups,** such as `asyncio.TaskGroup`. Do not use exceptions for normal control flow, such as ending a loop that a `break` or a return value can end.

## Raising

7. **Raise a project exception that inherits from one base class.** The base inherits from `Exception`. Callers can then catch everything from your package with one `except`.

   ```python
   class ShopError(Exception):
       """Base class for errors raised by the shop package."""

   class OutOfStockError(ShopError):
       def __init__(self, sku: str) -> None:
           super().__init__(f"sku {sku} is out of stock")
           self.sku = sku

   try:
       raise OutOfStockError("A-1")
   except ShopError as err:
       assert isinstance(err, OutOfStockError) and err.sku == "A-1"
   ```

   Why: callers branch on the type and read the fields. Parsing the message text breaks when the wording changes.
   Check: `TRY002` for `raise Exception(...)`, `N818` for the `Error` suffix.

8. **Use a built-in exception when it fits the meaning.** `ValueError` for a bad value, `TypeError` for a wrong type, `KeyError` for a missing key, `NotImplementedError` for an abstract method. Define your own when callers need to tell it apart.

9. **Chain with `raise ... from err` when you translate an exception.** Use `from None` only to hide a cause that tells the reader nothing, and then put the useful detail in the new message.

   ```python
   class ConfigError(Exception):
       pass

   def read_port(raw: str) -> int:
       try:
           return int(raw)
       except ValueError as err:
           raise ConfigError(f"port must be a number, got {raw!r}") from err

   try:
       read_port("eighty")
   except ConfigError as err:
       assert isinstance(err.__cause__, ValueError)
   ```

   Check: `B904`.

10. **Re-raise the same exception with a bare `raise`.** Use `err.add_note(...)` (3.11+) to add context without changing the type.
    Check: `TRY201` for `raise err` inside the handler.

11. **Write messages that say what failed, with the value, and without secrets or personal data.** `f"order {order_id} has no items"`. The base rule in engineering-standards covers PII.
    Sources disagree here. Ruff's `EM101`, `EM102` and `TRY003` ask you to build the message outside the `raise` line or inside the exception class. This skill leaves them off and keeps the message at the `raise` line, because the reader sees the cause in one place.

## Logging

12. **Each module gets its own logger.** `logger = logging.getLogger(__name__)` at module level. Do not log through the root logger (`logging.info(...)`).
    Check: `LOG015` for root logger calls, `LOG002` for a `getLogger` argument other than `__name__`.

13. **Pass log values as `%` arguments.** `logger.info("charged order %s", order_id)`. Do not pre-format with an f-string.
    Why: logging formats the string only when a handler emits it, and the constant template groups the same event in log search.
    Sources disagree here. Some teams accept f-strings in logs for readability. This skill keeps `%` arguments, because the logging HOWTO and structured log tools rely on the template.
    Check: `G004` for f-strings, `G001` for `.format`, `G003` for `+`.

14. **Log a caught exception with `logger.exception(...)`** inside the `except` block. It adds the traceback.
    Check: `TRY400` for `logger.error` inside an `except` block, `G201` for `error(..., exc_info=True)`.

15. **Log or raise, not both.** Log where you handle the error. Code that re-raises does not log, or the same failure shows up once per layer.

16. **A library adds no handlers and sets no levels.** It may attach `logging.NullHandler()` to its top logger. The application configures logging once, at start, with `logging.config.dictConfig` and `"disable_existing_loggers": False`.
    Why: handlers and levels belong to the application. A library handler duplicates or hides the application's output.

17. **No `print` in library or service code.** A CLI may print its result to stdout. Diagnostics go to the logger.
    Check: `T201`, `T203`.
