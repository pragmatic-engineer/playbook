# Maintainability

- Mixed concerns in a single function, class, or module.
- Magic numbers or string literals that should be named constants.
- Inline types or schemas that duplicate existing definitions.
- Naming that diverges from established project conventions.
- Bypassing infrastructure helpers (logging, config, HTTP clients) with ad-hoc alternatives.
- Injecting raw functions as dependencies when the function belongs to an existing service interface.
