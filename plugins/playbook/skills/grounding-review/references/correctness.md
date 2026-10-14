# Functionality / Correctness

- Logic that doesn't match the stated intent (PR description, ticket, comments).
- Missing guard clauses for null, empty, or out-of-range inputs.
- Silent fallbacks that hide incorrect behaviour (default values masking bugs).
- Off-by-one errors, incorrect boundary conditions, race conditions.
- Type coercion or implicit conversion that changes semantics.
