# Testing

Python specifics on top of the base testing rules in engineering-standards. A Ruff rule named here needs its group or code in `lint.select`. A rule marked (preview) also needs `preview = true`.

## Contents

- Layout and config: rules 1 to 4
- Writing tests: rules 5 to 10
- Doubles and isolation: rules 11 to 15
- Timing and flakiness: rules 16 to 18

## Layout and config

1. **Use pytest.** Write plain `assert` statements and test functions. Keep `unittest.TestCase` only in a suite that already uses it.
   Why: pytest shows the values on both sides of a failed `assert`, and its fixtures replace `setUp` chains.

2. **Tests live in `tests/` next to `src/`, and run against the installed package.** Set `--import-mode=importlib` for new projects. Then `tests/` needs no `__init__.py`, and two test files may share a name.
   Why: the default `prepend` mode puts the test folder on `sys.path` and can import the working tree instead of the installed package.

   ```toml
   [tool.pytest.ini_options]
   addopts = ["--import-mode=importlib", "--strict-markers"]
   testpaths = ["tests"]
   xfail_strict = true
   filterwarnings = ["error"]
   ```

3. **Turn on the strict options.** `--strict-markers` turns a typo in a marker name into an error, `xfail_strict` fails a test marked `xfail` that passes, and `filterwarnings = ["error"]` turns warnings into errors. Ignore a named warning on its own line when you must.
   Note: pytest 9 renamed `xfail_strict` to `strict_xfail` and kept the old name. It also added `strict = true`, which turns on every strict option. Use `strict` only when the pytest version is pinned, because a new release may add options to it.

4. **Measure coverage with `pytest-cov`, and read the report for untested branches.** Turn on branch coverage. Do not write a test only to raise the number.
   Why: a line that ran is not a line that was checked. Low-value tests add upkeep and protect nothing.
   Check: `pytest --cov --cov-branch`, with the threshold set in config. The base rule says the threshold does not go down.

## Writing tests

5. **One behavior per test, named for the behavior.** `test_refund_rejects_negative_amount`. Arrange, act, assert, in that order.

6. **Use `@pytest.mark.parametrize` for the same check over many inputs.** Give cases readable `ids`. Do not loop inside one test.

   ```python
   import pytest

   @pytest.mark.parametrize(
       ("raw", "expected"),
       [("8080", 8080), (" 80 ", 80)],
       ids=["plain", "padded"],
   )
   def test_parse_port(raw: str, expected: int) -> None:
       assert int(raw) == expected
   ```

   Why: each case passes or fails on its own, and the report names it.
   Check: `PT006` for the names format, `PT007` for the values format.

7. **Assert exceptions with `pytest.raises` and `match`.** Keep one statement inside the block.

   ```python
   import pytest

   def test_int_rejects_words() -> None:
       with pytest.raises(ValueError, match="invalid literal"):
           int("eighty")
   ```

   Check: `PT011` for a broad exception without `match`, `PT012` for more than one statement in the block, `B017` for `pytest.raises(Exception)`.

8. **Assert one condition per `assert`, and compare floats with `pytest.approx`.**
   Check: `PT018` for `assert a and b`.

9. **Build test data with fixtures and factory functions.** A fixture does one thing and cleans up after `yield`. Keep fixtures function-scoped unless setup is slow, and then make the wider-scoped fixture read-only.
   Why: a shared mutable fixture couples tests, and one test's change breaks the next.

10. **Use property tests for parsers and invariants.** Hypothesis `@given` runs the test on many generated inputs and shrinks a failure to a small case. Add the shrunk case as a plain regression test.

## Doubles and isolation

11. **Inject the dependency and pass a fake.** Take the client, clock or repository as a parameter (data-modeling, rule 8 shows a `Protocol`), and give the test a small fake class. Patch only what you cannot inject.
    Why: this is the base mocking rule in Python form. A fake checks behavior, and a patch couples the test to import paths.

12. **When you patch, patch where the name is looked up, and use `autospec`.** `mocker.patch("shop.billing.charge", autospec=True)` (pytest-mock) or `unittest.mock.patch(..., autospec=True)`.
    Why: a module that did `from payments import charge` holds its own reference, and a patch on `payments.charge` misses it. A plain `MagicMock` accepts any call, and a test stays green after the real signature changes.

13. **Use `monkeypatch` for environment variables and `tmp_path` for files.** `monkeypatch.setenv` and `monkeypatch.delenv` undo themselves after the test. `tmp_path` is a fresh folder per test.
    Why: tests then share no state and run in any order and in parallel.

14. **Run tests in random order to find hidden coupling.** `pytest-randomly` shuffles the order, reseeds `random` per test and prints the seed to replay a failure.

15. **No real network in unit tests.** Block it with a plugin such as `pytest-socket`, or a fixture that fails on connect. Integration tests that need a service start their own, per the base rule.

## Timing and flakiness

16. **No `time.sleep` to wait for work.** Poll the condition with a deadline, or wait on an event the code sets.

    ```python
    import threading
    import time
    from collections.abc import Callable

    def wait_for(condition: Callable[[], bool], timeout: float = 5.0, interval: float = 0.01) -> None:
        deadline = time.monotonic() + timeout
        while not condition():
            if time.monotonic() > deadline:
                raise TimeoutError(f"condition not met after {timeout}s")
            time.sleep(interval)

    done = threading.Event()
    threading.Timer(0.05, done.set).start()
    wait_for(done.is_set)
    ```

    Why: a fixed sleep is slow on a fast machine and flaky on a slow one. The poll returns as soon as the condition holds.
    For a thread, prefer `event.wait(timeout)` or `ThreadingMock.wait_until_called(timeout=...)` (3.13+). For asyncio, wrap the wait in `async with asyncio.timeout(...)`.

17. **Do not test time by waiting for it.** Pass a clock into the code (data-modeling, rule 8), or freeze time with a library such as `time-machine`.

18. **Treat a flaky test as a bug.** Find the cause. Do not add reruns to hide it, and do not leave it as a non-strict `xfail`.
