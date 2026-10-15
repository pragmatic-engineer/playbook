# Style and naming

Layout, names, imports, docstrings and comments. The project's existing style wins over this list. A Ruff rule named here needs its group or code in `lint.select`. A rule marked (preview) also needs `preview = true`.

## Rules

1. **Let the formatter own layout.** Run `ruff format` (or Black, if the project already uses it) and do not argue about spaces in review. Keep the line length at 88, the formatter default, unless the project sets another.
   Sources disagree here. PEP 8 says 79, allows up to 99 when a team agrees, and keeps comments and docstrings at 72. Google says 80. Ruff and Black default to 88. This skill picks 88, because it needs no config and matches what the formatter writes.
   Check: `ruff format --check`. Leave `E501` off when the formatter runs: the formatter wraps code, and `E501` then flags only lines it cannot wrap.

2. **Names follow PEP 8.** `snake_case` for functions, methods, variables and modules. `CapWords` for classes and type variables. `UPPER_CASE` for module constants. Acronyms stay upper case in class names (`HTTPError`).
   Check: `N801`, `N802`, `N803`, `N806`, `N815`, `N816`.

3. **A `_` prefix marks a name as internal.** `_cache`, `_parse_row`. Use a `__` prefix only to avoid a name clash in a class built for subclassing. Do not invent new `__dunder__` names. A library module lists its public names in `__all__` as well. Do not import another package's `_` names.
   Why: users of a module rely on every name without the `_` prefix.
   Check: `SLF001` flags code that reaches into another object's `_private` names.

4. **Exceptions end in `Error`** when they report an error (`ConfigError`).
   Check: `N818`.

5. **No single-letter `l`, `O` or `I`.** Do not shadow built-ins such as `list`, `id`, `type` or `input`, or a standard library module name.
   Why: `l` looks like `1`, and a shadowed `list` breaks the next call to `list()` in that scope.
   Check: `E741`, `A001`, `A002`, `A005`.

6. **Name constants and make them immutable.** A module constant is `UPPER_CASE`, typed `Final`, and holds an immutable value (a number, a string, a `tuple`, a `frozenset`). A literal a reader would have to look up gets a name, per the base rule on magic values.
   Check: `PLR2004` for magic values in comparisons. The type checker reports a reassigned `Final` name.

7. **Name what a value means.** `retry_delay_seconds` says more than `delay` or `d`. Put the unit in the name when the type does not carry it. Use a `timedelta` where you can. Keep one type per name: a `total` that holds a number does not later hold its formatted string.

8. **Imports sit at the top in three groups:** standard library, third party, then local, with a blank line between. One module per `import` line.
   Check: `I001` (with `ruff check --fix`), `E401`, `E402`.

9. **Import by absolute path.** `from shop.orders import Order`. Relative imports of siblings (`from .models import Order`) are fine inside one package. Do not climb with `..`.
   Sources disagree here. PEP 8 allows explicit relative imports, Google bans all of them. This skill bans only parent-relative imports, because they break when a module moves and hide where a name lives.
   Check: `TID252` with `lint.flake8-tidy-imports.ban-relative-imports = "parents"`.

10. **No `from module import *`** outside a package `__init__.py` that re-exports an internal module.
   Why: the reader and the linter can no longer tell where a name came from.
   Check: `F403`, `F405`.

11. **Every public module, class, function and method has a docstring.** The first line is one sentence in the imperative mood that ends with a period (`"""Return the order total in cents."""`). Add argument, return and raise sections when the signature does not make them clear. Do not repeat the types the annotations already give.
    Check: the `D` rules, for example `D100` to `D103`.

12. **Pick one docstring style per project and tell Ruff.** Google style is the default for new projects. NumPy style suits scientific code.

    ```toml
    [tool.ruff.lint.pydocstyle]
    convention = "google"
    ```

    Sources disagree here. PEP 257 asks for the imperative mood and takes no side on sections. Google allows the descriptive mood ("Fetches rows."). This skill keeps PEP 257's imperative mood, because Ruff's `D401` checks it.
    Check: `D401` (on under the `numpy` and `pep257` conventions, off under `google`; select it by name).

13. **Comments say why.** A comment that restates the code goes. Keep comments true when the code changes. Delete commented-out code: version control keeps it.
    Check: `ERA001` for commented-out code.

14. **Compare to `None` with `is`, and test truth directly.** `if items:` over `if len(items) > 0:`, `if ready:` over `if ready == True:`. Use `is not None` when zero or an empty string is a valid value.

    ```python
    def label(count: int | None) -> str:
        # wrong: `if count:` treats 0 as missing
        # right
        if count is not None:
            return f"{count} items"
        return "unknown"
    ```

    Check: `E711`, `E712`.

15. **Use `isinstance` for type checks.** `type(x) == Foo` ignores subclasses.
    Check: `E721`.

16. **Write `def`, not a lambda bound to a name.** `square = lambda x: x * x` loses the name in tracebacks.
    Check: `E731`.
