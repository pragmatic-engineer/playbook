# ADR 0001: Package the toolkit as an opt-in Claude Code plugin

**Status:** Accepted
**Date:** 2026-07-23

## Context

Today the toolkit is installed by copying this repo into `~/.claude`, wiring hooks through `settings.json` (the user's untracked live file) seeded from the tracked `settings.shared.json`. That works for the author but it is all or nothing: there is no clean way for someone else to adopt the skills, commands, agents, and hooks and later back out, and the shipped seed carries hook wiring that a fresh install applies globally.

Claude Code already has a first class plugin system, and this machine uses it (the `claude-plugins-official` and `cloudflare` marketplaces are installed, and `superpowers` is an installed plugin). A plugin is the native unit of opt-in: users add a marketplace, install a plugin, and enable or disable it per project or globally. The repo root already matches the plugin component layout (`skills/`, `commands/`, `agents/`, `hooks/`), so most of the work is metadata plus translating hook wiring.

## Decision

Ship the whole toolkit as a single plugin named `playbook`, distributed through a one plugin marketplace (now in the separate `pragmatic-engineer/marketplace` repo). Keep per feature opt-out on the existing environment variables rather than splitting into multiple plugins.

Rationale for one plugin (not modular): the skills, commands, agents, and hooks are cross referential (commands spawn the agents, hooks feed the memory the skills rely on). A single enable or disable is the simplest mental model, and the environment gates already give finer control without the maintenance cost of several manifests and cross plugin dependencies.

## Scope of this PR

This PR delivers the design and a working, installable scaffold. It does not yet flip the toolkit over to the plugin as the sole delivery path. Concretely:

Added:
- `.claude-plugin/plugin.json`: the plugin manifest (metadata only; components are auto discovered from the standard subdirs).
- `.claude-plugin/marketplace.json`: a marketplace listing this plugin, sourced from GitHub `pragmatic-engineer/playbook`.
- `hooks/hooks.json`: the plugin hook wiring, a faithful translation of the current `settings.shared.json` hooks with paths rewritten to `${CLAUDE_PLUGIN_ROOT}`.

Deliberately deferred to a follow up (see Migration): removing the `hooks` block from `settings.shared.json`, teaching `shell/gen-shared-settings.sh` to strip it, and enabling the plugin by default. Because the plugin is not added to `enabledPlugins` here, nothing double fires until a user opts in.

## Plugin layout

The repo root doubles as the plugin root. Only three files are new; every existing component dir is discovered as is.

```
/ (repo root = plugin root)
  .claude-plugin/
    plugin.json          NEW: manifest
    marketplace.json     NEW: one plugin marketplace, github source
  hooks/
    hooks.json           NEW: plugin hook wiring via ${CLAUDE_PLUGIN_ROOT}
    *.sh, lib/common.sh  existing, unchanged
  skills/ commands/ agents/  existing, auto discovered
```

Hook scripts source their library with `. "$(dirname "$0")/lib/common.sh"`, which resolves relative to the script, so it keeps working from the plugin cache with no edit. Runtime and memory state stay in `${HOME}/.claude/runtime` and `${HOME}/.claude/memory` because those paths are absolute in the scripts, so state lives in one canonical place whether a script runs from `~/.claude/hooks` or from the plugin cache.

## Hook wiring translation

`hooks/hooks.json` mirrors the current `settings.shared.json` wiring event for event: `session-init` on SessionStart; `rm-workspace-guard` (with `if: Bash(rm:*)`), `bg-await-guard`, and `no-dash-guard` on PreToolUse Bash; `preread-edit-check` and `preread-size-check` on PreToolUse Read; `search-counter` on the read and edit matcher; `post-edit-track` and `rebuild-memory-graph` on PostToolUse; `auto-model-detect` on UserPromptSubmit; `precompact-warn` on PreCompact; `session-clean-exit` on Stop and SessionEnd. Matchers, the `rm` guard `if`, and timeouts carry over unchanged.

One entry is intentionally not shipped: the `rtk hook claude` PreToolUse Bash entry. `rtk` is a personal external tool with no script in this repo, so it stays only in the author's `settings.json`.

Keep `hooks/hooks.json` in step with the `settings.shared.json` hook list: when a hook is added or removed there, mirror the change here (translating the path to `${CLAUDE_PLUGIN_ROOT}`). The natural long term fix is to have `shell/gen-shared-settings.sh` generate one from the other so the two never drift.

## Opt-out

Per feature opt-out is unchanged and works the same under the plugin, since the scripts read these from the process environment: `SKILLS_PRIMER`, `AUTO_LEARN_NUDGE`, `AUTO_LEARN_MIN_EDITS`, `AUTO_LEARN_MAX_AGE_DAYS`, `BG_AWAIT_GUARD`, and `ASYNC_DISCIPLINE` (plus `NO_DASH_GUARD` once #45 merges). Set any of them in the `env` block of your `settings.json` or your shell.

## Migration (follow up, not in this PR)

To make the plugin the canonical delivery and avoid double firing for anyone who enables it:

1. Remove the `hooks` block from `settings.shared.json`; keep `permissions`, `env`, `statusLine`, and `enabledPlugins`.
2. Update `shell/gen-shared-settings.sh` to `del(.hooks)` when generating the seed (so regenerating from a personal `settings.json`, which still holds `rtk` and local wiring, never reintroduces hooks), and update its paired tests.
3. Add `"playbook@pragmatic-engineer": true` to the seed's `enabledPlugins` so fresh installs enable the plugin.
4. Document the adoption path in the README: `/plugin marketplace add pragmatic-engineer/marketplace`, then `/plugin install playbook@pragmatic-engineer`, then remove any legacy `~/.claude/hooks/*` wiring from a personal `settings.json` to avoid double firing.
5. Update the `settings-distribution-model` memory fact to record that hook wiring now ships through the plugin, not the seed.

Existing users are not clobbered: `install.sh` seeds `settings.json` only when absent, so removing hooks from the seed does not retroactively unwire anyone; it only changes what a fresh install applies.

## Risks

- **Double firing hooks.** If a user enables the plugin while their `settings.json` still wires the same scripts, every hook runs twice (harmless for the idempotent guards, but it double counts the search and edit trackers). Mitigated by the migration above and a README note. Because `settings.json` is user owned and untracked, this cannot be fixed for users programmatically.
- **`rtk` portability.** Excluded from the plugin on purpose; documented so `rtk` users keep it local.
- **Versioning.** `plugin.json` pins `version`, so users get updates only when it is bumped; the GitHub marketplace source without a ref tracks the default branch tip. Pick a release cadence and bump on change.
- **Trust.** The hooks run shell, so users must trust the source before enabling.
- **State path coupling.** Runtime and memory stay under `~/.claude` by absolute path. Do not rewrite those to the plugin root, since `learn-project` and the statusline read `~/.claude/memory`.

## Consequences

Anyone can adopt the toolkit with two commands and back out by disabling one plugin. The author keeps a personal `settings.json` for machine specific bits (`rtk`, model routing, permissions posture). The cost is a second wiring surface to keep in sync (`hooks/hooks.json` next to `settings.shared.json`) until the migration removes the seed's hooks; `shell/gen-shared-settings.sh` is the natural place to generate one from the other and remove that drift.

## References

- `plugin.json` and `marketplace.json` real examples under `plugins/cache/claude-plugins-official/superpowers/` and `plugins/marketplaces/*/.claude-plugin/`.
- Current hook wiring: `settings.shared.json` `hooks` block.
- Distribution model: the `settings-distribution-model` memory fact.
