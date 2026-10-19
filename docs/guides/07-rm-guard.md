# The rm guard

The `rm-workspace-guard` hook stops one kind of accident: a command that deletes the filesystem root or your home folder, such as `rm -rf /` or `rm -rf ~`. It is a short, fixed rule. It is not a security boundary. [ADR 0045](../adr/0045-rm-guard-roots-and-parser.md) records the design and the 2026-10-19 amendment that cut it down.

## What it denies

An `rm`, `rmdir`, `unlink` or `shred` with any of these targets, with or without quotes, with any flags (also `--no-preserve-root` and `--`), and after `sudo`, `command`, `env`, `exec`, `nohup` or `time`:

| Target | Forms |
| :- | :- |
| The filesystem root | `/`, `/*`, `//` |
| Your home folder | `~`, `~/`, `~/*`, `$HOME`, `${HOME}`, `$HOME/`, `$HOME/*`, and the literal path of your home folder, with or without `/` or `/*` after it |

The command line is split on `;`, `&`, `|` and line breaks, and each simple command is read word by word. The denial is one fixed sentence that says the model does not run it and that you can run the command yourself with `!`.

## What it allows

Every other path, including paths inside home: `rm -rf ./target`, `rm -rf ~/projects/x/target`, `rm -rf /tmp/x` and `rm -rf /Users/other` all pass.

The guard does not follow `cd`, variables, subshells (`$(...)`, backticks), `find -delete`, `xargs`, `git clean`, interpreter one-liners, redirects, here-docs or scripts. Anything it cannot read passes. Quoted text, an `echo`, a grep pattern and a commit message are data, so `echo "rm -rf /"` passes.

## What changed

Before 2026-10-19 the guard judged every target against a set of safe roots, followed `cd` and variables, and kept a deny set of system, credential and playbook paths. It denied too many harmless commands and cost more in support than it saved. The parser, the roots and the config key `hooks.rmGuard.allowedRoots` are gone. A stored `hooks.rmGuard.allowedRoots` value stays in the store and is ignored for one release, `config import` skips it, and `config set` no longer takes it. The first command after the upgrade prints one notice about this.

With `defaultMode` set to `auto` in the seed settings, the guard no longer catches a deletion of a project folder, your `~/.ssh`, your memory store or `config.db`. Only Claude Code's own permission handling and your deny rules in settings.json stand between the model and those paths. To keep a path safe, add a deny or ask rule that names that path in your Claude Code settings.

`playbook disk clean` keeps its own list of protected playbook data and does not depend on the guard.

The model cannot change the retired `hooks.rmGuard.*` keys: the policy guard still refuses a model-run `config set` on them.

See also: [Hooks reference](../reference/hooks.md), [Docs index](../README.md).
