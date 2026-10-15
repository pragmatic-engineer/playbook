# Project structure

Repository layout, packages, modules and entry points. A Ruff rule named here needs its group or code in `lint.select`.

## Rules

1. **Use the src layout for anything you build and install as a package:** `src/<package>/`, with `tests/` and `pyproject.toml` at the root. A service or a CLI that you do not build as a wheel may use the flat layout.

   ```text
   pyproject.toml
   src/shop/__init__.py
   src/shop/orders.py
   tests/test_orders.py
   ```

   Sources disagree here. The Hitchhiker's Guide argues against a `src` folder. Stuart Ellis keeps src for wheels and flat for applications. The Packaging User Guide and pytest explain why src catches packaging bugs. This skill takes src for packages and allows flat for apps, because the bug src prevents (tests passing against the working tree instead of the installed package) only exists when the package is installed.

2. **Keep one package per distribution, named after the project.** Put the code in a package, not loose modules at the root.

3. **Group modules by domain.** `orders/`, `billing/` and `shipping/` say more than `utils/`, `helpers/` and `models/` that collect unrelated code.
   Why: a domain module keeps the code that changes together in one place.

4. **Dependencies point one way, per the base rule.** Domain code does not import the web framework or the database driver. Adapters at the edge import the domain.
   Check: `import-linter` contracts, or `TID251` with `lint.flake8-tidy-imports.banned-api` to ban an import in a module.

5. **A cycle between modules is a design problem.** Move the shared piece into its own module. Do not hide the cycle with an import inside a function.
   Check: `PLC0415` for imports that are not at the top level.

6. **Keep the entry point thin.** `__main__.py` or a `main()` function parses arguments, sets up logging and config, calls the library, and maps errors to an exit code. Register CLIs under `[project.scripts]`.

   ```toml
   [project.scripts]
   shop = "shop.cli:main"
   ```

   Why: the logic stays importable and testable without a subprocess.

7. **Keep `__init__.py` small.** Re-export the public API and set `__all__`. Do not run work at import time: no network calls, no file reads, no logging setup.
   Why: every importer pays for import-time work, and a test cannot stop it.

8. **Config enters in one place.** Read environment variables and files once at startup into a typed settings object (a dataclass or `pydantic-settings`), and pass it down. Do not call `os.environ` deep in the code.
   Why: this is the base boundary rule for config. A test then passes settings in.

9. **A library ships its types (types-and-typing, rule 16) and one version number.** Keep the version in `pyproject.toml` or derive it from the VCS tag. Read it at run time with `importlib.metadata.version("shop")`.
