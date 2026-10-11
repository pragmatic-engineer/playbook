# ADR-0023: The Opus and Sonnet Commands Ship at Medium

- **Status:** Accepted
- **Date created:** 2026-10-11
- **Date modified:** 2026-10-11

## Context

After [ADR-0021](0021-reviewer-at-opus-medium.md) and the bench campaign, the `reviewer`, `critic` and `implementer` agents ship at `medium`. Seven Opus and Sonnet commands still shipped at `high`: `plan`, `adr`, `deep-review`, `address-pr-comments`, `learn-project`, `implement` and `fix`. The `auditor` agent also shipped at `high`. The evidence on the lower setting:

| Component | Result |
| --- | --- |
| `reviewer` (Opus) | medium 199/200, high 200/200, 19% cheaper |
| `implementer` (Sonnet) | medium 200/200, high 198/200 on 20 repo tasks |
| `critic` (Sonnet) | medium 89/90, high 71/90 (false blocks on clean plans) |

No bench exists for the design commands or for the auditor. A headless bench of a whole interview, an orchestrated swarm or a repository audit has not been built.

## Decision

1. The maintainer lowered `commands/plan.md`, `commands/adr.md`, `commands/deep-review.md`, `commands/address-pr-comments.md`, `commands/learn-project.md`, `commands/implement.md` and `commands/fix.md` from `high` to `medium` on 2026-10-11. Models stay as they are (Opus for the first five, Sonnet for `implement` and `fix`).
2. `agents/auditor.md` is deliberately kept at `high` by the same maintainer decision of 2026-10-11, and its floor in `src/effort/floor.rs` stays at `high`. No bench covers it, and a missed finding in a whole repository audit is the cost.
3. `agents/git.md` is unchanged (Haiku, `xhigh`).
4. The main session follows the same rule through the launcher. `ccc` and `ccd` add `--effort medium` to the `claude` command line when the session states no effort (no `--effort`, no `effortLevel` in the user settings, no `CLAUDE_CODE_EFFORT_LEVEL`), so high is used only when asked. Measured use before the change was medium 55%, high 44%. Playbook never edits Claude Code's own settings or memory, so a session started without the launcher keeps the model default.
5. The policy tables in `docs/internals/02-model-routing-and-memory.md` follow the files.

## Alternatives considered

- **Keep `high`.** Safe, but it costs more on every run, and the three benched components showed no loss at medium.
- **Build a bench first.** A fair bench of these commands needs multi turn tool use. It was not built before the change.

## Consequences

- Runs of these commands are cheaper. How much was not measured.
- Quality on design work, swarm orchestration and main thread work is unproven at medium (the main session was not benched). The decision rests on the results of other components, not on the commands it changes.
- Signals to watch: rework after `implement` and `fix` (follow up commits, reopened tasks), findings missed by `deep-review` that a later review catches, and the output of `playbook effort suggest`.

## How to revert

For the launcher default, remove the `session_default_effort` call in `src/cc/launch.rs` (and `SESSION_DEFAULT_EFFORT` in `src/models/mod.rs`). For the commands:

Set `effort: high` on line 6 of each file, then restore the matching rows in the policy tables of `docs/internals/02-model-routing-and-memory.md` (the test `tests/policy_table.rs` fails until they agree):

- `commands/plan.md`
- `commands/adr.md`
- `commands/deep-review.md`
- `commands/address-pr-comments.md`
- `commands/learn-project.md`
- `commands/implement.md`
- `commands/fix.md`
