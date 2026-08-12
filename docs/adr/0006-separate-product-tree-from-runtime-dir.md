# ADR 0006: Separate the product tree from the Claude Code runtime directory

- **Status:** Accepted
- **Date created:** 2026-08-12
- **Date modified:** 2026-08-12
- **Amends:** ADR 0001 (package the toolkit as a plugin), ADR 0002 (plugin based install with always-on safety hooks)

## Context

Until now the git working tree for this repository *was* `~/.claude`, the live Claude Code
runtime directory. The product and the runtime state shared one directory.

That arrangement is the root cause of a cluster of defects found by the 2026-08-11 repository
audit, which named it finding A1 and traced S1, S2, S3, S5, S8, D1 and D2 back to it:

- **The public install seed is built from the maintainer's live personal config.**
  `Makefile:13` sets `SRC ?= $(HOME)/.claude/settings.json` and
  `shell/gen-shared-settings.py` filters it with a five-key denylist, so every other key
  passes through to installers. Verified: 21 of the 24 shipped top-level keys were byte
  identical to the personal config, including an `autoMode.allow` prose string instructing
  the permission evaluator to ignore an `rtk` prefix, and an `enabledPlugins` block
  force-enabling four third-party plugins.
- **The build is not reproducible.** `make settings.shared.json` reads a file that is not in
  the repository, so no contributor can regenerate the seed.
- **Machinery exists solely to contain the overlap.** `.gitignore` opens with `/*` and
  re-allowlists 15 paths; `shell/check-manifest.sh` exists only to stop runtime state
  leaking into git; `shell/setup-local.sh` needs `-ef` self-copy guards because source and
  destination can be the same inode.
- **Editing a hook breaks the running session.** The untracked `settings.json` wires 12
  hooks to `~/.claude/hooks/*.py` and the statusline to `~/.claude/statusline.sh`, all
  resolving into the working tree. Deleting or renaming a hook on a branch breaks the live
  session immediately, at working-tree-delete time rather than merge time. This is recorded
  as the `hook-rename-lockstep-settings` gotcha and was hit again on 2026-08-11 during the
  ADR 0005 migration.
- **The documented install path was never exercised.** `install.sh` and `/setup` have never
  run on a machine where the repo did not already sit at `~/.claude`. Two defects survive
  precisely because of that blind spot: the skills primer reads `$HOME/.claude/skills`,
  which the plugin path never creates (audit C1), and `install.sh` copies inert `.py`
  duplicates into `~/.claude/hooks/` that are never executed (audit A4).

Patching these individually was the original plan. Each patch bounds a symptom; none
removes the cause.

## Decision

**Move the product tree out of the runtime directory.**

```
~/Workspace/pragmatic-engineer/playbook       <- this repository
~/Workspace/pragmatic-engineer/marketplace    <- the marketplace repository
~/.claude                                     <- runtime state ONLY
```

`~/.claude` keeps exactly what Claude Code owns: `settings.json`, `sessions/`, `projects/`,
`runtime/`, `plugins/`, `memory/`, `history.jsonl`, plus the small set of files the install
path *copies* there on purpose (the three safety guards, `statusline.sh`, the launchers
under `shell/`, and `prompts/SYSTEM_PROMPT.md`).

**The maintainer consumes the toolkit the same way a stranger does**: via
`claude plugin marketplace add` and `claude plugin install playbook@pragmatic-engineer`,
then `/setup`. Development happens in the new location; the installed plugin is what runs.

## Decision drivers

1. **Distribution correctness.** A public artifact must not be derived from a private file
   that happens to sit in the same directory.
2. **Dogfooding is the only honest test.** Audit findings C1 and A4 are invisible from
   inside the shared directory and reproduce immediately outside it.
3. **Removing a cause beats bounding a symptom.** The allowlist inversion planned for the
   seed generator is still worth doing, but it is defence in depth once the input is no
   longer the maintainer's live file by construction.
4. **Editing the product should never risk the running session.**

## Considered alternatives

**Keep the shared directory, invert the generator to an allowlist.** (~1 day.) This was the
original plan and is still partially retained. Rejected as the whole answer: it bounds the
key space that can leak but leaves the build unreproducible, keeps `check-manifest.sh` and
the `/*` allowlist, keeps the hook-edit hazard, and keeps the install path untested.

**Keep the shared directory, hand-author `settings.shared.json`.** (~2 hours.) Removes the
leak permanently but is the weakest option on every other axis, and lets the shipped seed
silently drift from the config the maintainer actually runs.

**Move, and stop dogfooding** (develop in the new location, keep a hand-wired `~/.claude`).
Rejected: it preserves the blind spot that hid C1 and A4. The point of moving is to make the
maintainer's machine representative of an installer's.

**Symlink `~/.claude` at the new location.** Rejected: it recreates the shared inode with
extra indirection, and `-ef` self-copy guards would still be required.

## Consequences

**Positive**

- `make settings.shared.json` becomes reproducible or is deleted; either way the seed stops
  being a function of one machine's private state. (Which of the two is settled by the
  re-scoped seed work, not here.)
- `.gitignore` collapses from a `/*` denylist-plus-allowlist to an ordinary ignore file.
- `shell/check-manifest.sh` loses its reason to exist. It is retained until the re-scope
  confirms nothing else depends on it, then removed.
- Editing any hook is safe: the working tree is no longer on any live hook path.
- The install path gets exercised on every machine, including the maintainer's.
- Audit findings S5 and S8 shrink: `~/Workspace` and `rtk` stop being ambient facts of the
  build environment.

**Negative / costs**

- A one-time migration with a real breakage window: 13 paths in the live `settings.json`
  dangle between moving the files and re-running `/setup`. Mitigated by an idempotent
  `migrate.sh` with a dry-run mode and a backup, run from a shell that is not inside a
  Claude Code session.
- The maintainer now runs the *installed* plugin, so a local edit needs a plugin reinstall
  (or an explicit dev override) before it takes effect. This is a real day-to-day friction
  cost and is the main argument the rejected "move but keep hand-wiring" option had going
  for it.
- Two working copies exist until the old tree is removed.

**Neutral**

- The plugin delivery model of ADR 0001 and the guards-are-always-on rule of ADR 0002 are
  both unchanged. This ADR changes where *development* happens, not how the product is
  delivered or how the guards are wired.

## What this amends in ADR 0001 and ADR 0002

- ADR 0001 assumed the repo and `~/.claude` were the same tree when it described
  `install.sh` copying the tracked tree into `~/.claude`. That copy is now a genuine copy
  between two distinct locations, which is what the code always claimed to do.
- ADR 0002's split (guards wired directly into `settings.json`, functional hooks via the
  plugin) is unchanged and becomes *enforceable*: with the trees separate, a functional hook
  can no longer be accidentally wired by absolute path into `settings.json`, which is
  exactly how the live config ended up wiring all 12.
- ADR 0001's note to "keep `hooks/hooks.json` in step with the `settings.shared.json` hook
  list" still stands.

## Migration

Two halves, split by whether a step can break a running session.

**Non-destructive (done 2026-08-12):** clone both repos into
`~/Workspace/pragmatic-engineer/`, import the outstanding feature branch, and verify. The
full suite passes from the new location (36/36) along with every static gate, confirming the
product carries no dependency on living at `~/.claude`.

**Destructive (scripted, run outside a session):** install the marketplace and plugin;
rewrite `settings.json` to drop the 12 Python hook entries (the plugin supplies them) and
keep the three guards; run `setup-local.sh` from the new location to re-copy the guards,
statusline and launchers; back up the tracked product tree to
`~/.claude/backups/pre-relocation-<stamp>/` and remove it from `~/.claude`, preserving all
runtime state.

`~/.zshrc:72` sources `$HOME/.claude/shell/zsh/cc.zsh` and stays valid, because
`--install-aliases` copies the launchers into `~/.claude`. The statusline path stays valid
for the same reason.

## References

- Repository audit, 2026-08-11 (finding A1 and its dependents S1, S2, S3, S5, S8, D1, D2)
- ADR 0001: package the toolkit as a plugin
- ADR 0002: plugin based install with always-on safety hooks
- Project memory: `hook-rename-lockstep-settings`, `settings-distribution-model`,
  `settings-seed-allowlist-inversion`
