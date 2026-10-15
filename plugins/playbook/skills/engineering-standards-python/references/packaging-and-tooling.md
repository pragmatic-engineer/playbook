# Packaging and tooling

`pyproject.toml`, environments, lock files, versions, and the checks CI runs. A Ruff rule named here needs its group or code in `lint.select`.

## Contents

- Project files and environments: rules 1 to 5
- Dependencies: rules 6 to 10
- Python versions: rules 11 and 12
- Checks and CI: rules 13 to 17

## Project files and environments

1. **Keep project metadata and tool config in `pyproject.toml`.** Use the standard `[project]` table, a `[build-system]` table, and `[tool.<name>]` tables for Ruff, mypy, pytest and coverage. Do not add `setup.py` unless a build step needs code.
   Why: one file that every modern tool reads.

2. **Use the project's tool for environments and installs.** For a new project, use uv: it installs Python versions, creates `.venv`, resolves and locks dependencies, and runs scripts. Poetry, PDM and Hatch are fine where a project already uses them. Do not start new projects on Pipenv or hand-run `setup.py` commands.
   Sources disagree here. Stuart Ellis recommends uv in one section and Poetry in another. The Packaging User Guide makes no blanket choice and its tool page does not list uv. This skill picks uv for new projects, because one tool covers Python versions, environments, the lock file and scripts, and the guide lists its `uv_build` backend.

3. **Work in a virtual environment.** Do not install into the system Python, and do not commit `.venv`.
   Why: the system Python belongs to the operating system's tools.

4. **Declare development tools in `[dependency-groups]`** (PEP 735), such as a `dev` group with pytest, Ruff and mypy. Keep `[project.optional-dependencies]` for extras that users install.

5. **Run one-off scripts with inline metadata.** A single-file script lists its dependencies in a `# /// script` block and runs with `uv run script.py` (or `pipx run`).

## Dependencies

6. **An application commits a lock file and installs from it.** `uv.lock` (or `poetry.lock`, `pdm.lock`), or `pylock.toml` (PEP 751) when tools must share it. CI installs with `uv sync --locked`, which fails when the lock no longer matches `pyproject.toml`.
   Why: every machine and every deploy gets the versions you tested.

7. **A library declares version ranges.** Set a lower bound you test, such as `httpx>=0.27`. Add an upper bound only for a known break.
   Why: a library that pins `==` conflicts with every other package that needs a different patch release.
   Sources disagree here. Many guides say "pin your dependencies" with no split. This skill pins exactly in the application lock file and uses ranges in a library's `dependencies`, because only the application decides the final set.

8. **Install with hashes where you deploy from requirements files.** Generate them with `uv pip compile --generate-hashes` or `pip-compile --generate-hashes`, and install with `pip install --require-hashes`.
   Why: a hash stops a changed or swapped package from installing.

9. **Add a dependency only when it earns its place.** Check its maintenance, release history, supported Python versions and license first, and remove the ones you stop using.
   Why: each dependency is code you now maintain and a way in for an attacker.

10. **Upgrade dependencies on a schedule,** with a bot (Dependabot or Renovate) that opens one pull request per change and runs CI.

## Python versions

11. **Support only CPython versions that still get fixes.** Check the [status page](https://devguide.python.org/versions/). Set the floor in `requires-python` with no upper bound, and test every supported minor version in CI.
    A scientific library may follow SPEC 0, which drops a Python version three years after its release.

12. **An application pins one Python version** in `.python-version` (uv and pyenv read it) and in the container image, and upgrades it in its own pull request.

## Checks and CI

13. **Run Ruff for lint and format.** Ruff replaces flake8, its plugins, isort and Black in one tool.

    ```toml
    [tool.ruff]
    line-length = 88

    [tool.ruff.lint]
    select = ["E4", "E7", "E9", "F", "I", "N", "UP", "B", "S", "SIM", "PT", "RUF"]

    [tool.ruff.lint.per-file-ignores]
    "tests/**" = ["S101"]
    ```

    Sources disagree here. Real Python's formatting page offers Black with isort, or Ruff. This skill picks Ruff, because one fast tool covers all three with the same rules and a Black-compatible formatter.
    Note: `ruff format` does not sort imports. Run `ruff check --fix` for `I001` first.

14. **CI runs the same checks as the local hooks, and fails on any finding:** `ruff check`, `ruff format --check`, the type checker, and the tests with coverage.

15. **Use pre-commit hooks for fast checks only,** such as Ruff and a secret scanner. Pin each hook's `rev`. Keep the type checker and tests in CI, or in a pre-push hook if the team wants them locally.
    Why: a slow commit hook gets skipped.

16. **Every `noqa` and `type: ignore` names the rule.** Turn on `RUF100` to remove `noqa` comments that no longer match anything.
    Check: `PGH004` for a bare `noqa`, `RUF100`.

17. **Build and publish from CI.** Build with `uv build` or `python -m build`, tag the release with a valid version, and publish to PyPI with Trusted Publishing instead of a stored API token.
