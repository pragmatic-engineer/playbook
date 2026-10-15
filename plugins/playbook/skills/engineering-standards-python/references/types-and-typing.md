# Types and typing

Annotations, the type checker and its config, generics, narrowing and the escape hatches. A Ruff rule named here needs its group or code in `lint.select`. A rule marked (preview) also needs `preview = true`.

## Contents

- Checker setup: rules 1 to 4
- Writing annotations: rules 5 to 11
- Escape hatches: rules 12 to 16

## Checker setup

1. **Run one type checker in strict mode in CI, with the same config the editor uses.** For mypy, `strict = true` and `warn_unreachable = true` under `[tool.mypy]`. For pyright, `typeCheckingMode = "strict"` under `[tool.pyright]`.

   ```toml
   [tool.mypy]
   strict = true
   warn_unreachable = true
   enable_error_code = ["ignore-without-code", "redundant-expr", "truthy-bool"]
   ```

   Why: mypy's default skips the body of any function without annotations, and `--strict` turns that off together with twelve other checks. `warn_unreachable` is not part of `--strict`.
   Check: `mypy` (2.4) or `pyright` as a CI step.

2. **Pick the checker the project already runs.** For a new project, pick mypy when you need a plugin (Pydantic, Django, SQLAlchemy) and pyright when the team wants the editor (Pylance) and CI to report the same errors.
   Sources disagree here. Stuart Ellis prefers the newer checkers (pyright, Pyrefly) for speed. Real Python shows mypy first. This skill picks neither by default, because the deciding factor is the plugin need, and the two checkers report different errors on the same code.

3. **Bring legacy code under strict mode one module at a time.** Keep the global setting strict and loosen named modules with `[[tool.mypy.overrides]]`. For pyright, put `# pyright: basic` at the top of each legacy file. Do not turn strict off globally.
   Why: the debt then has a list of names, and new modules start clean.

4. **Silence a missing stub per module.** Use `ignore_missing_imports = true` in an override for the one untyped library, or install its `types-*` package. Do not set it globally.
   Why: the global flag also hides a typo in an import, which then becomes `Any`.

## Writing annotations

5. **Annotate every public function and method, including `-> None`.** Annotate `__init__` with `-> None`. Let the checker infer local variables.
   Check: `ANN001`, `ANN201`, `ANN204`, or mypy `disallow_untyped_defs` (part of strict).

6. **Write `X | None` and the built-in generics.** `list[str]`, `dict[str, int]`, `type[User]`, and abstract types from `collections.abc`. Do not import `List`, `Dict` or `Optional` from `typing`.
   Sources disagree here. PEP 484 and many stdlib examples use `Optional[X]`. The typing best practices page, Ruff and Google prefer `X | None`. This skill picks `X | None`, because it needs no import and Python 3.10 and later evaluate it at run time.
   Check: `UP006`, `UP007`, `UP035`, `UP045`.

7. **Spell `None` out when it is allowed.** `def find(name: str, limit: int | None = None)`. A `None` default does not make the type optional.
   Check: `RUF013`, and both mypy and pyright reject the implicit form by default.

8. **Accept abstract types, return concrete ones.** Take `Iterable[str]`, `Sequence[int]` or `Mapping[str, object]`, return `list[str]` or `dict[str, int]`.
   Why: callers can pass a tuple or a generator, and callers of the result get every method of the real type.

9. **Use `object` for a parameter that accepts anything, and avoid `Any`.** `Any` turns checking off in both directions. `object` keeps it on and forces a check before use.
   Check: `ANN401` flags `Any` in a signature.

10. **Make a `match` or `if` chain over an enum or a union exhaustive with `assert_never`.**

    ```python
    from enum import Enum
    from typing import assert_never

    class Shape(Enum):
        CIRCLE = 1
        SQUARE = 2

    def corners(shape: Shape) -> int:
        match shape:
            case Shape.CIRCLE:
                return 0
            case Shape.SQUARE:
                return 4
            case _:
                assert_never(shape)
    ```

    Why: a new enum member is then a type error at every `match` that misses it.
    Check: pyright `reportMatchNotExhaustive` (on in strict), or the `assert_never` call for mypy.

11. **Use the Python 3.12 generic syntax when the minimum version allows it.** `def first[T](items: Sequence[T]) -> T:` and `type Headers = dict[str, str]`. Use `NewType` when a value must not mix with its base type (`UserId` and `OrderId` are both `int`). Use `Self` for methods that return `self`, `@override` on a method that overrides, and `Final` for constants.
    Check: `UP040` rewrites `TypeAlias` to a `type` statement, and `UP046` and `UP047` rewrite the old `TypeVar` and `Generic` forms.

## Escape hatches

12. **Narrow with a runtime check before you reach for `cast`.** `isinstance`, a `TypeIs` function (3.13+) or a validator checks the value. `cast` checks nothing and lies when the data changes.

    ```python
    from typing import Final

    MAX_PORT: Final = 65_535

    def as_port(value: object) -> int:
        # wrong: return cast(int, value)
        if isinstance(value, int) and not isinstance(value, bool) and 0 < value <= MAX_PORT:
            return value
        raise ValueError(f"port must be an integer from 1 to {MAX_PORT}, got {value!r}")

    assert as_port(8080) == 8080
    ```

    Check: mypy `warn_redundant_casts` (strict) and pyright `reportUnnecessaryCast` (strict) catch casts that no longer do anything.

13. **No `Any` across a boundary.** `json.loads`, `requests.Response.json` and untyped libraries return `Any`. Validate the value into a model or narrow it right where it enters (data-modeling, rule 10).
    Why: `Any` spreads. Every value computed from it is unchecked too.
    Check: review. Neither checker reports a declared `Any` inside a function in strict mode. mypy `warn_return_any` (strict) catches only an `Any` that is returned, and mypy `disallow_any_expr` catches every use but is noisy.

14. **Every ignore names the error code and is the narrowest one that works.** `# type: ignore[attr-defined]` or `# pyright: ignore[reportPrivateUsage]`, with a short reason when it is not obvious.
    Check: `PGH003` for a bare `# type: ignore`, mypy `ignore-without-code` (enable it as in rule 1) and `warn_unused_ignores` (strict).

15. **Put imports used only in annotations under `if TYPE_CHECKING:`** when they are costly or create an import cycle. Python 3.14 evaluates annotations lazily, and earlier versions need `from __future__ import annotations` or quoted names.
    Check: `TC001`, `TC002`, `TC003` move such imports. `TC004` catches an import inside the block that is used at run time.

16. **A library ships its types.** Add an empty `py.typed` file to the package, and re-export public names in `__all__` or with `import x as x`.
    Why: without `py.typed`, the type checker of a user treats the library as untyped. mypy's strict `no_implicit_reexport` hides names that are not re-exported.
