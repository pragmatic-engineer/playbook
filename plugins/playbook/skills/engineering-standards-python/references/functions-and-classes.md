# Functions and classes

Function signatures, defaults, classes, properties, special methods and module state. A Ruff rule named here needs its group or code in `lint.select`. A rule marked (preview) also needs `preview = true`.

## Contents

- Functions: rules 1 to 9
- Classes: rules 10 to 18

## Functions

1. **One function, one job.** It fits on a screen, and its name is a verb phrase that says what it returns or does. Split a function when you need "and" to name it.
   Check: `C901` (complexity), `PLR0912` (branches), `PLR0915` (statements). Set the limits in `pyproject.toml`.

2. **No mutable default arguments.** The default is built once, when Python runs the `def`, and every call shares it. Use `None` and build the value inside.

   ```python
   # wrong: every call appends to the same list
   def add_tag_wrong(tag: str, tags: list[str] = []) -> list[str]:
       tags.append(tag)
       return tags

   # right
   def add_tag(tag: str, tags: list[str] | None = None) -> list[str]:
       tags = [] if tags is None else list(tags)
       tags.append(tag)
       return tags

   assert add_tag_wrong("a") == ["a"] and add_tag_wrong("b") == ["a", "b"]
   assert add_tag("a") == ["a"] and add_tag("b") == ["b"]
   ```

   Check: `B006`, and `B008` for a function call in a default (`now: datetime = datetime.now()` runs once).

3. **Make parameters after the first one or two keyword-only.** Put `*` in the signature. Do not take a `bool` as a positional flag.

   ```python
   def export(path: str, *, overwrite: bool = False, compress: bool = False) -> None: ...

   export("out.csv", overwrite=True)  # reads clearly at the call site
   ```

   Why: `export("out.csv", True, False)` does not say which flag is which.
   Check: `FBT001`, `FBT002`, `FBT003`, and `PLR0913` for too many arguments.

4. **Return a value, do not mutate an argument.** When a function must change its input, say so in the name (`sort_in_place`) and return `None`.
   Why: a caller who passes a list does not expect it to change.

5. **Handle bad input and trivial cases first, and return or raise early.** The main path then runs at one level of indentation. Check input with `if` and `raise`. `python -O` removes every `assert`.
   Check: `RET505` for an `else` after a `return`, `PLR5501` for `else` then `if`.

6. **Return one type.** Do not return a value on one path and `None` by accident on another. When `None` means "not found", annotate `X | None` and return it explicitly.
   Check: `RET503` for a missing explicit `return`.

7. **Keep side effects at the edges.** A function that computes a result does not also write files, call the network or read the clock. Pass the clock, the client or the path in.
   Why: the pure part can be tested with plain values, which is the base rule to inject dependencies.

8. **Use `*args` and `**kwargs` only to forward arguments** to another callable, and type them with `ParamSpec` in a decorator. Do not use them to avoid naming parameters.

9. **Wrap a decorator's inner function with `functools.wraps`.**
   Why: the wrapped function keeps its name and docstring for logs, tracebacks and tests.

## Classes

10. **A class holds state and the behavior that uses it.** A class with only static methods is a module. A class with one method and no state is a function.
   Why: extra classes add a layer the reader must step through.

11. **Composition over inheritance follows the base rule in engineering-standards.** In Python, hold the collaborator as an attribute set in `__init__` and call it. A mixin is inheritance too: keep it to one small, stateless behavior.
    Why: a deep tree spreads one behavior across files, and a change in the base breaks subclasses you cannot see.

12. **Set every instance attribute in `__init__`.** Do not add attributes later from other methods.
    Why: the reader and the type checker find the full shape of the object in one place.

13. **Do not put mutable values on the class body.** `items: list[str] = []` at class level is shared by every instance. Use `ClassVar` for a real class constant, or `field(default_factory=list)` in a dataclass.
    Check: `RUF012`.

14. **Expose plain attributes, and add a `@property` later when you need logic.** Do not write `get_x` and `set_x` methods. A property is cheap, has no side effects, and raises no surprise errors.
    Why: callers keep writing `obj.x` when the logic arrives.

15. **Write `__repr__` for any class you will see in a log or a debugger.** Dataclasses write it for you.

16. **Define `__eq__` and `__hash__` together.** A class that defines `__eq__` alone becomes unhashable. Hash only fields that do not change.
    Check: `PLW1641`.

17. **Do not put `functools.cache` or `lru_cache` on a method.** The cache holds a reference to `self`: `cache` keeps every instance alive, and `lru_cache` keeps up to `maxsize` of them. Cache a module-level function, or use `functools.cached_property` for a per-instance value. `cached_property` needs an instance `__dict__`, so it fails on a `slots=True` dataclass. Compute the value in `__post_init__` there.
    Check: `B019`.

18. **Avoid module-level mutable state and `global`.** Pass state in, or hold it on an object the caller owns.
    Why: global state couples tests to each other and to import order.
    Check: `PLW0603`.
