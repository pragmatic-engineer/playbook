# ADR-0017: Per-Component Effort Ceilings

- **Status:** Accepted
- **Date created:** 2026-10-09
- **Date modified:** 2026-10-09

## Context

Issue #573 asks for a maximum effort per skill, command and agent, on top of the global ceiling that already exists (`maxEffortLevel`, shipped in #590, #592 and #598). Two limits of Claude Code shape the answer. The `Agent` tool has no per-call effort override, and a hook cannot change a subagent's effort. An agent's effort is fixed by its file, so enforcement for agents can only mean choosing which file to dispatch. Commands and skills set effort in their own frontmatter, and a skill sets none on purpose (`docs/concepts/03-why-the-pieces-are-shaped-this-way.md`).

## Decision

1. **Config keys.** One key per component: `effort.agents.<name>`, `effort.commands.<name>`, `effort.skills.<name>`. The value is `auto`, `low`, `medium`, `high`, `xhigh` or `max`, the same enum as `maxEffortLevel`. `<name>` is lowercase letters, digits and hyphens. Set with `playbook config set --global effort.agents.fact-checker medium`. Only the global tier counts, as for `maxEffortLevel`.
2. **Unknown names warn.** The key shape is validated when it is written. Whether the name exists is checked against the plugin files by `playbook effort list`, which flags a configured name that matches no component. A stale name never fails a command.
3. **Precedence.** The ceiling for a component is the lowest of: Claude Code's `maxEffortLevel` (always the top authority, read only), playbook's `maxEffortLevel`, and the component's own key. `auto` and `max` add no limit. The effective effort is the lower of the component's shipped effort and that ceiling. A ceiling never raises anything. A component that ships no effort (`effort:` absent) inherits the session's effort, so only the session ceiling applies to it.
4. **Agents: choose a variant.** `playbook effort resolve agents <name> --json` returns the shipped effort, the ceiling, the effective level and the agent file to dispatch (`reviewer`, `reviewer-medium`, `reviewer-low`, and so on): the strongest variant whose effort is at or below the ceiling. `playbook agents gen` writes `-low`, `-medium` and `-xhigh` variants. There is no `-high` variant because every base agent that has variants ships at `high`, so the base file is the high variant. The `delegating-subagents` skill tells the orchestrator to run `resolve` before a spawn and to dispatch the file it names.
5. **Commands and skills: a lookup.** `playbook effort resolve commands <name>` gives the allowed level. A command's own frontmatter cannot be rewritten per user, so the command text calls `resolve` in its first step and uses the result to decide which agent variants to spawn. The launcher still passes the global ceiling to the session with `--settings`.
6. **Status.** `playbook effort list` prints every component with its shipped effort, configured ceiling and effective level, in a table or as `--json`.

## Alternatives considered

- **Nested `effort.perComponent` object.** Rejected: `playbook config set` takes one dotted key and one scalar, and a nested object would need a second write path.
- **Rewriting agent frontmatter at launch.** Rejected: it edits plugin files, which are replaced on every plugin update, and it cannot differ per session.
- **A `-high` variant.** Rejected as redundant with the base file.
- **Failing on an unknown component name.** Rejected: plugin files change between versions and a stale key must not break a session.

## Consequences

- Every agent with variants gains a `-medium` file, so the agents directory grows by five files.
- A user can hold `fact-checker` at `medium` and leave `deep-review` at `xhigh`, as long as Claude Code's own ceiling allows it.
- Enforcement for commands and skills is advisory text plus the global launcher ceiling. The strong guarantee exists for agents only, because only agents have a selectable file.
