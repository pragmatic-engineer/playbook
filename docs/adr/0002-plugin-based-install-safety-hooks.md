# ADR 0002: Plugin based install with always-on safety hooks

- **Status:** Accepted, amended by ADR 0006 and ADR 0007 (hook delivery now goes through `playbook init`; hooks.json carries only the worktree hooks and the migration check)
- **Date created:** 2026-07-30
- **Date modified:** 2026-07-30
- **Amends:** ADR 0001 (package the toolkit as an opt-in plugin)

## Context

ADR 0001 scaffolded the toolkit as a Claude Code plugin and left, as a follow up, the question of how installation and hook wiring should work once the plugin is the delivery vehicle. Two forces shaped the answer:

1. The plugin should be the single source of the components (skills, commands, agents) and the functional hooks, so nothing loads twice.
2. The protective guards (blocking a dangerous `rm`, a backgrounded install that will be raced, an em or en dash in a post) should fire everywhere, not only where the plugin happens to be enabled. Plugin hooks are scoped to where the plugin is enabled; a guard that is sometimes off is not a guard.

## Decision

Split hook delivery by purpose, and make `install.sh` plugin based and interactive.

**Safety guards ship wired directly, always on.** The three guards (`rm-workspace-guard`, `bg-await-guard`, `no-dash-guard`) are the only hooks left in `settings.shared.json`, so the seeded `settings.json` wires them from `~/.claude/hooks`. They do not depend on the plugin.

**Everything else ships with the plugin.** The components and the functional hooks (session-init, preread checks, search-counter, post-edit-track, rebuild-memory-graph, auto-model-detect, precompact-warn, session-clean-exit) load from the plugin via its `hooks/hooks.json`. That file no longer wires the three guards, so enabling the plugin does not double fire them.

**`install.sh` is plugin based and interactive.** It adds the marketplace and installs plus enables the plugin, installs the safety guards and the other local configs (settings.json, statusline, shell launchers, dependencies), and asks before each optional step. It no longer copies `skills/`, `commands/`, or `agents/` into `~/.claude` (the plugin owns them), and it retires any such directories left by an older direct install so they cannot shadow the plugin. Prompts read from `/dev/tty` so they work under `curl | bash`; `--yes` takes every default, and `--skip-plugin`, `--skip-deps`, `--skip-shell`, and `--no-setup` remain.

**The generator enforces the split.** `shell/gen-shared-settings.sh` reduces `.hooks` to the three safety guards when it produces the seed, so regeneration never reintroduces the functional hooks or the personal `rtk` entry.

## What this supersedes in ADR 0001

- 0001 proposed the plugin `hooks/hooks.json` mirror the full `settings.shared.json` wiring. It now carries the functional hooks only; the guards are wired directly.
- 0001 deferred stripping the hooks block from `settings.shared.json`. That is done here, reduced to the guards rather than removed entirely.

## Consequences

- Guards are always on for anyone who runs the installer, independent of plugin enablement per project.
- The toolkit has one source (the plugin), so no duplicate skills, commands, agents, or double fired functional hooks.
- Two hook lists still exist (the guards in `settings.shared.json`, the rest in the plugin `hooks/hooks.json`) and must stay in step; the generator keeps the seed side honest, and the split is stable because the guard set rarely changes.
- Installing without the `claude` CLI yields the configs and guards but not the toolkit; the installer says so and prints the two commands to finish.

## References

- ADR 0001: `docs/adr/0001-package-toolkit-as-plugin.md`.
- Generator: `shell/gen-shared-settings.sh` and its test.
- Installer: `install.sh`; seed validator `shell/check-shared-settings.sh`.
