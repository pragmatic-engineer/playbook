# Data modeling

Classes that hold data, closed sets of values, interfaces, and parsing input into objects. A Ruff rule named here needs its group or code in `lint.select`. A rule marked (preview) also needs `preview = true`.

## Contents

- Records: rules 1 to 6
- Closed sets and interfaces: rules 7 to 10
- Boundaries: rules 11 to 14

## Records

1. **Use a `dataclass` for a class that mostly holds data.** It writes `__init__`, `__repr__` and `__eq__` for you.
   Check: `B903` (preview) flags a class whose `__init__` only stores its arguments.

2. **Freeze a record unless the code must change it in place.** `@dataclass(frozen=True, slots=True)`. Return a changed copy with `dataclasses.replace` (or `copy.replace` on 3.13+).

   ```python
   from dataclasses import dataclass, replace

   @dataclass(frozen=True, slots=True)
   class Money:
       amount_cents: int
       currency: str

   price = Money(500, "EUR")
   discounted = replace(price, amount_cents=450)
   assert price.amount_cents == 500 and discounted.amount_cents == 450
   ```

   Why: a frozen object can be shared, hashed and used as a dict key, and no caller can change it under you. `slots=True` saves memory and turns a typo in an attribute name into an error.
   Note: freezing is shallow. A `list` field can still change. Use `tuple` or `frozenset` fields in a frozen record.

3. **Give a mutable default with `field(default_factory=...)`.** A dataclass raises `ValueError` for an unhashable default such as a `list`, `dict` or `set`, and accepts a mutable object that is hashable.
   Check: `RUF008` for mutable defaults, `RUF009` for a function call in a default.

4. **Validate in `__post_init__` only for invariants of the record itself** (an amount is not negative, a range has start before end). Raise `ValueError` with the field name.

5. **Use `NamedTuple` only when the record must also behave as a tuple,** for example to unpack the result of a function or to keep an API that returned a plain tuple. Use a dataclass for everything else.
   Why: a named tuple compares equal to any tuple with the same items, and can be indexed by position.

6. **Choose attrs or Pydantic over a dataclass only for a feature you need.** attrs adds validators and converters on fields. Pydantic adds parsing and serialization. Keep one choice per layer.
   Sources disagree here. Stuart Ellis lists Pydantic, attrs and dataclasses as equal options. This skill picks the standard library dataclass inside the program and Pydantic at the edges (rule 11), because the dataclass needs no dependency and the edges need parsing.

## Closed sets and interfaces

7. **Use an `Enum` for a fixed set of values.** `StrEnum` (3.11+) when the value goes into JSON or a database as text. Do not pass bare strings for statuses or modes.

   ```python
   from enum import StrEnum, auto

   class Status(StrEnum):
       PENDING = auto()
       PAID = auto()

   assert Status("paid") is Status.PAID
   assert f"{Status.PENDING}" == "pending"
   ```

   Why: a typo in an enum member is an `AttributeError`. A typo in a string passes until production.
   Check: `UP042` for a class that inherits from both `str` and `Enum`, `PIE796` for duplicate enum values.

8. **Match on an enum with every case named, and close it with `assert_never`.** The type checker then reports a new member that has no branch (types-and-typing, rule 10).

9. **Describe what a function needs with a `Protocol`.** A parameter typed with a protocol accepts any object that has the methods, with no inheritance.

   ```python
   from typing import Protocol

   class Clock(Protocol):
       def now_ms(self) -> int: ...

   class FixedClock:
       def now_ms(self) -> int:
           return 1_000

   def deadline(clock: Clock, budget_ms: int) -> int:
       return clock.now_ms() + budget_ms

   assert deadline(FixedClock(), 500) == 1_500
   ```

   Why: the caller and a test fake share no base class, and the checker still verifies the shape. This is the base rule to inject dependencies, in Python form.

10. **Use an abstract base class (`abc.ABC` and `@abstractmethod`) when subclasses must share code** or when `isinstance` checks against the interface matter at run time.
    Check: `B024` for an ABC with no abstract method, `B027` for an empty method in an ABC that is not marked abstract.

## Boundaries

11. **Parse untrusted data into a typed object at the boundary.** HTTP bodies, config files, queue messages and API responses go through a Pydantic model (or the project's validator) once. Code inside takes the model or a dataclass. It does not take the raw `dict`.

    ```python
    from pydantic import BaseModel, ConfigDict, Field

    class CreateUser(BaseModel):
        model_config = ConfigDict(extra="forbid", frozen=True)
        email: str = Field(max_length=254)
        age: int = Field(ge=0)

    user = CreateUser.model_validate({"email": "a@example.com", "age": 30})
    ```

    Why: one schema checks the data at run time and gives the type checker the shape. This is the base rule on boundaries, in Python form.
    Note: this snippet needs Pydantic 2 and ran under Pydantic 2.14.

12. **Forbid unknown fields at a strict boundary** (`extra="forbid"`). Allow them only where the producer may add fields and you must not break.

13. **Use `TypedDict` to type a `dict` you do not own,** such as JSON you pass through or keyword arguments. A `TypedDict` checks nothing at run time.

14. **Keep the boundary model and the domain model apart when they differ.** Convert once in the adapter. Do not let an ORM row or a request model leak through every layer.
    Why: a change to the wire format then touches one function.
