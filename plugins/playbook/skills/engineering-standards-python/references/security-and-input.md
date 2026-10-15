# Security and input

Injection, unsafe parsing, secrets, paths, network calls and dependencies. The base rules in engineering-standards on boundaries and secrets apply. A Ruff rule named here needs its group or code in `lint.select`, and most are in the `S` group, ported from Bandit.

## Contents

- Injection: rules 1 to 4
- Parsing and files: rules 5 to 8
- Secrets and crypto: rules 9 to 11
- Network and dependencies: rules 12 to 15

## Injection

1. **Bind SQL parameters.** Pass values as parameters to the driver or the ORM. Do not build SQL with f-strings, `%` or `+`.

   ```python
   import sqlite3

   conn = sqlite3.connect(":memory:")
   conn.execute("CREATE TABLE users (name TEXT)")
   name = "x' OR '1'='1"
   # wrong: conn.execute(f"SELECT * FROM users WHERE name = '{name}'")
   rows = conn.execute("SELECT * FROM users WHERE name = ?", (name,)).fetchall()
   assert rows == []
   ```

   Why: a bound value stays data and cannot change the query.
   Check: `S608`. A table or column name cannot be a parameter: check it against an allowlist.

2. **Run processes with an argument list and no shell.** `subprocess.run(["git", "log", ref], check=True, timeout=30)`. Do not pass `shell=True` with any value a user can touch, and do not use `os.system`.
   Why: the shell reads `;`, `|` and `$()` in the input as commands.
   Check: `S602`, `S604`, `S605`, `S607` for a partial executable path, `PLW1510` for `run` without `check`.

3. **No `eval` or `exec` on input.** Parse literals with `ast.literal_eval` or `json.loads`.
   Check: `S307`, `S102`.

4. **Autoescape templates that render HTML.** Jinja2 needs `autoescape=True` or `select_autoescape()` outside a web framework that sets it.
   Check: `S701`.

## Parsing and files

5. **Do not unpickle data you did not produce.** Use JSON or a schema format for anything that crosses a trust boundary. `shelve` and `multiprocessing` connections also use pickle.
   Why: unpickling can run any code the data asks for.
   Check: `S301`.

6. **Load YAML with `yaml.safe_load`.** Parse XML from outside with `defusedxml`.
   Check: `S506` for `yaml.load` with an unsafe loader, `S314` and its neighbors for the standard `xml` parsers.

7. **Keep user-supplied paths inside the base folder.** Resolve the joined path and check that it is still under the base.

   ```python
   import tempfile
   from pathlib import Path

   def safe_join(base: Path, user_path: str) -> Path:
       target = (base / user_path).resolve()
       if not target.is_relative_to(base.resolve()):
           raise ValueError(f"path escapes {base}: {user_path!r}")
       return target

   with tempfile.TemporaryDirectory() as tmp:
       assert safe_join(Path(tmp), "a/b.txt").name == "b.txt"
       try:
           safe_join(Path(tmp), "../etc/passwd")
       except ValueError:
           pass
       else:
           raise AssertionError("traversal not caught")
   ```

   Extract archives with `tarfile` `filter="data"` (3.12+, the default from 3.14) and cap the size of what you unpack.
   Check: `S202` for `extractall` with no filter.

8. **Create temporary files with `tempfile.NamedTemporaryFile` or `TemporaryDirectory`.** Do not use `tempfile.mktemp` or fixed paths under `/tmp`.
   Check: `S306`, `S108`.

## Secrets and crypto

9. **Read secrets from the environment or a secret store at startup.** Do not hard-code them, and keep them out of logs, exceptions and `__repr__`. Wrap a secret in a type whose `repr` hides the value, such as Pydantic's `SecretStr`.
   Check: `S105`, `S106`, `S107` for literal passwords. Run a secret scanner in CI.

10. **Make tokens with `secrets`.** `secrets.token_urlsafe(32)`, not `random`. Compare tokens and signatures with `hmac.compare_digest`.
    Why: `random` is predictable, and `==` stops at the first different byte, which leaks timing.
    Check: `S311` for `random` in a security context.

11. **Hash passwords with a slow password hash** (Argon2 or bcrypt, through a maintained library). Do not use `hashlib.md5` or `sha1` for anything that must resist an attacker.
    Check: `S324`.

## Network and dependencies

12. **Give every HTTP call a timeout.** `requests` waits forever by default. `httpx` defaults to 5 seconds, and an explicit value still documents the intent.
    Check: `S113` (covers `requests` and `httpx`).

13. **Keep TLS verification on.** Do not pass `verify=False`, even in a script.
    Check: `S501`.

14. **Audit dependencies in CI.** Run `pip-audit` against the lock file or the exported requirements, and document each ignored advisory with a reason and a date.
    Why: a known-vulnerable package is the easiest way in.

15. **Bind a server to `127.0.0.1` in development.** Bind to all interfaces only where the deployment needs it.
    Check: `S104`.
