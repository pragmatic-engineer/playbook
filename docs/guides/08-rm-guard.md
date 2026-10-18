# The rm guard

The `rm-workspace-guard` hook stops an `rm` that would delete something outside the folders playbook treats as safe. It is a guard against accidents. It does not try to be a security boundary. [ADR 0045](../adr/0045-rm-guard-roots-and-parser.md) records the design.

## What counts as a deletion

The guard reads the command the way the shell does, with the shared shell lexer, and judges only real command words:

- `rm` in command position, also after `&&`, `||`, `;`, `|`, inside `$(...)` and backticks, and behind `command`, `builtin`, `sudo`, `doas`, `env`, `nice`, `nohup`, `xargs` and the other runners. `\rm` and `r'm'` count too.
- `find ... -delete` and `find ... -exec rm`, judged by the start paths of `find`.
- `xargs rm` fed by a `find` in the same pipeline, judged by that `find`. Other `xargs` input cannot be known, so it is blocked.
- A script given to `sh -c`, `bash -c`, `eval`, a here-string, or a heredoc that a shell reads.

The guard follows the folder the shell is in. A relative target is judged against the `cwd` Claude Code sends with the command, and after `cd`, `pushd` or `popd` against the folder the shell changed into. A `cd` inside `( ... )` or `$( ... )` ends with the group. When a `cd` could fail and the next command still runs (after `;`, `||` or a line break rather than `&&`), the target is judged in both the new and the old folder. A folder the guard cannot name, such as `cd "$X"`, `popd` with nothing pushed, or `cd name` while `CDPATH` is set, makes a relative target unresolvable.

Variables are followed when the command itself sets them to something the guard can name: `S=/x; rm -rf $S/y`, `export`, a chain such as `N=$S/z`, `+=`, and a `for` loop over literal words. It also knows `$HOME`, `$PWD`, `$OLDPWD`, `$(pwd)`, `$$` (digits only), `$TMPDIR` and `${TMPDIR:-/tmp}` (with the hook's own value, which the shell inherits from Claude Code), and a file or folder made by a plain `$(mktemp)` or `$(mktemp -d)`. An assignment that may not run, after `&&` or `||` or inside `if` or a loop, keeps the old value possible too; with no old value the variable is unknown. A variable set in a pipeline part, a background job, a child shell, by `read`, or in a command that defines a function or runs `source`, is unknown. A value holding whitespace is unknown, since the shell would split it.

Text that is only data is left alone: a quoted grep pattern, a commit message, an `echo`, a file path, and a heredoc read by `cat`, `git commit` or any program other than a shell. A redirect such as `2>/dev/null` is never a target.

Claude Code starts the hook only for a command that holds an `rm` word (`if: Bash(rm:*)`). A `find -delete` with no `rm` anywhere in the command never reaches it.

## What is allowed with no config

Paths are judged after `~`, `$HOME`, `..`, trailing slashes and every symlink are resolved, so `/tmp/../etc` is judged as `/etc`. A symlink is judged both where it sits and where it points.

| Allowed | How |
| :- | :- |
| The repo (or `PLAYBOOK_SAFE_ROOTS`) and `~/.claude` | Anything inside, and the folder itself. The repo is the one the command starts in, plus the repo of each folder it changes into with `cd`. A repo that is your home or holds it never counts, and a start folder outside any repo counts only when it is not home or above it. |
| The `.worktrees` folder next to the main checkout, and `WORKTREE_BASE_DIR` | Contents only. `.worktrees/cargo-*` build folders are covered. |
| Temp and cache folders for your OS (table below) | Contents only. |
| Playbook's scratch folders: `~/.config/playbook/tmp`, `staging`, `embedded`, `cc-state`, and any `tmp` folder deeper under `~/.config/playbook` | Contents only. |
| Playbook's install leftovers: `playbook.<version>.bak` backups and `.playbook.*` staging files in the install folder (`$PLAYBOOK_BIN_DIR` or `~/.local/bin`), and the collector's launchd or systemd file. The binary and its `pb` and `pbtop` links stay blocked, because every hook runs the binary | By exact name or prefix. A Homebrew install is left to Homebrew. |

Temp and cache folders per OS:

| OS | Folders |
| :- | :- |
| macOS | `/tmp`, `/private/tmp`, the per-user temp folder `/private/var/folders/*/*/T` (also reached as `/var/folders/...`), `$TMPDIR` and the path it resolves to, `~/Library/Caches`, `~/.cache` |
| Linux | `/tmp`, `/private/tmp`, `/var/tmp`, `$TMPDIR`, `$XDG_CACHE_HOME` when it is inside home, `~/.cache`, `/run/user/<uid>` |
| Windows | `%TEMP%`, `%TMP%`, `%LOCALAPPDATA%\Temp` |
| All | the folder `std::env::temp_dir()` returns, which covers the session scratchpad |

The Windows roots are in the code and untested: the guard's path logic has not run on Windows yet ([ADR 0045](../adr/0045-rm-guard-roots-and-parser.md)).

A temp folder from the environment (`$TMPDIR`, `%TEMP%` and the like) counts only when it is absolute, at least two folders deep, free of `..`, and neither your home nor a folder above it.

"Contents only" means the folder itself, and any parent of it, stays blocked. A glob directly under one of these folders needs a named prefix of at least three characters: `/tmp/pb-*` is allowed and `/tmp/*` is blocked.

`$TMPDIR` written in a command is expanded with the hook's value. A shell profile that exports a different `$TMPDIR` is an accepted risk: the expanded path is still judged against the roots.

## What stays blocked

Playbook's own data is never deleted by `rm`, whatever root covers it, a configured root included:

- the memory store `~/.config/playbook/memory`
- `~/.config/playbook/data`, which holds `config.db`, `memory.db` and the usage database
- plans, handoffs and per-repo config under `~/.config/playbook/repos`, and legacy org config under `orgs`
- session state under `~/.config/playbook/runtime`: the spend ledger the auto mode cap reads, the auto-learn queue and old handoffs
- the playbook binary and its `pb` and `pbtop` links
- the effort ledger and its backups under `~/.config/playbook/effort`
- the system prompt under `~/.config/playbook/prompts`, the legacy `playbook.db` and `config.json`
- `~/.config/playbook` itself and every folder above it
- any `.lock` folder under `~/.config/playbook`, which clears itself

## Reading a denial

Each denied target is listed with the rule that matched:

| Rule | Meaning |
| :- | :- |
| outside the allowed roots | The resolved path is in none of the folders above. The message shows where it resolved to. |
| scratch root itself (contents only) | The target is a root folder, such as `/tmp`. Name a path inside it. |
| bare glob | A glob the guard cannot judge, such as `/tmp/*`. |
| protected playbook data | One of the paths in the list above. |
| playbook lock folder | A `.lock` folder that clears itself. |
| unresolvable target | A command substitution, `<xargs input>`, a variable the guard cannot follow, or a relative path in a folder the guard cannot name. Run the `rm` as its own call with a literal absolute path. |
| stray quote or shell character | A quote, backslash or shell syntax left inside the path. |
| another user's home or a quoted ~ | `~user`, or a `~` inside quotes, which the shell does not expand. |
| path that cannot be resolved | A link chain too deep to follow, a link that cannot be read, or a working folder that is gone. |

For a path outside the roots, the message gives the exact command that keeps your current roots and adds the folder that holds the target. It offers this only for a folder inside your home whose first folder under home does not start with a dot, so it never suggests `/etc` or `~/.ssh`. The model cannot change the key, so it asks you to run the command yourself.

## Adding a root

`hooks.rmGuard.allowedRoots` adds folders whose contents may be deleted. It adds to the built-in list and never removes from it. There is no append command, so `config set` replaces the whole list: include the roots you already have. `playbook config get hooks.rmGuard.allowedRoots` shows them.

Each root is a plain folder path:

- absolute, or starting with `~/`
- at least two levels deep for an absolute path (`/opt/scratch` works, `/tmp` does not), and at least one level under home (`~/scratch` works, `~` does not)
- no `..`, quote, `$`, backtick, backslash or glob characters

A root covers everything under it, so `~/scratch/**` and `~/scratch` mean the same thing. A trailing `/**` or `/*` is accepted and dropped before the list is stored, so the stored list holds plain paths. Any other glob is refused.

Run it in the Claude Code prompt with `!`, or in a terminal:

```bash
! playbook config set --global hooks.rmGuard.allowedRoots '["~/code/acme/scratch","/opt/scratch"]'
```

On success it prints the stored list. A colon-separated value such as `~/a/b:/opt/scratch` works too, and an empty value clears the list.
