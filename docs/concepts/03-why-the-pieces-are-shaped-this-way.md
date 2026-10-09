# Why the Pieces Are Shaped This Way

This page explains the reasoning behind playbook's building blocks: commands, skills, agents, hooks, models and effort levels. The other docs say what each piece does. This one says why it exists in that form, so you can predict where a new piece belongs and find the setting you want to change.

## The five building blocks

| Block | Lives in | Loads when | Use it for |
|---|---|---|---|
| Command | `commands/*.md` | You type `/playbook:<name>`, or ask in plain words | A multi-step procedure with a clear start and end |
| Skill | `skills/<name>/SKILL.md` | A command, an agent or Claude decides it is relevant | Knowledge or discipline that many procedures share |
| Agent | `agents/*.md` | A command or the orchestrator spawns it | A role that needs its own model, effort or tool limits |
| Hook | `src/hooks/`, wired in `settings.json` | A tool call, a prompt or a session event | A rule that must hold even when nobody remembers it |
| Output style | `output-styles/*.md` | You select it | How replies read, not what work gets done |

The split follows one question: who has to remember to do it?

- A **hook** runs without anyone remembering. Safety rules go here (block a destructive `rm`, strip AI attribution from a commit). A rule that depends on the model choosing to follow it is a request, not a guard.
- A **command** is something you ask for. It holds the order of steps, so the order is the same on every run.
- A **skill** is shared knowledge. A review discipline or the writing rules would otherwise be copied into every command that needs them, and the copies would drift. A skill is written once and loaded by name.
- An **agent** is a role with a fixed shape. Add one only when a plain prompt cannot give you the property you need: a restricted tool set, a pinned model, or behaviour that must be identical on every run.

## Why skills carry no model and no effort

A skill is knowledge loaded into whoever uses it. If a skill set `effort: low`, it would override the choice of the command or agent that loaded it. A review run at `high` that loads a skill pinned to `low` would think less because of a reference page. So skills never set `model` or `effort`. The caller decides.

## Why agents exist at all

Commands could do everything inline. Agents exist for three reasons.

1. **Structural limits.** The read-only reviewers (`reviewer`, `critic`, `fact-checker`, `test-reviewer`, `analyst`, `cheap-checker`, `review-triage`) have no `Edit`, `Write` or `Bash` tool. A reviewer that cannot change the code it reviews is a property you can verify in the file, and `playbook agents check` enforces it in CI. A prompt that says "do not edit" cannot give you that.
2. **Cost control.** Mechanical work (run git steps, gather history, apply an approved diff) goes to Haiku at low effort. The expensive model is kept for work where a mistake is costly.
3. **Context control.** An agent's long output stays out of the main conversation. Only its result comes back.

The full list of agents, their tools and how a command binds to one is in [Authoring agents](../authoring/02-authoring-agents.md).

## Why each model is where it is

The rule is: spend on judgment, save on routine.

- **Sonnet** is the session default and covers most coding. `implementer`, `critic`, `fact-checker`, `test-reviewer` and `analyst` use it because they write code or check claims, and Sonnet at high effort does that well.
- **Haiku** runs `git`, `patch-applier`, `collector`, `cheap-checker` and `review-triage`. These apply a decision someone else already made, or classify with a safe fallback. A wrong answer is cheap to detect and rerun.
- **Opus** runs design (`/playbook:plan`, `/playbook:adr`), every `reviewer`, and the `auditor`. A missed finding in a review, or a weak design, costs far more than the extra tokens.

The routing details, including the hook that nudges design prompts toward Opus, are in [Model routing and memory](../internals/02-model-routing-and-memory.md).

## Why effort is a second dial

Model choice sets how capable the work is. Effort sets how long it thinks. They are separate because the right pairing differs by task:

- A fixed procedure (commit, push, open a PR) needs a cheap model at **low** effort. Thinking harder does not improve it.
- A review needs a strong model at **high** effort, because a reviewer that stops looking early is worse than a slow one.
- A small diff does not need an `xhigh` review, and a risky one might.

The rule that produced the current values: **lower effort where the work is mechanical or already decided, and never where a missed finding is costly.** A model that is wrong costs a rerun. A reviewer that stops looking costs a bug in production. The table of every value and its reason is in [Effort policy](../internals/02-model-routing-and-memory.md#effort-policy).

### Why there are `-low` and `-xhigh` agent variants

The `Agent` tool takes a `model` on each call but has no per-call effort. An agent's effort is fixed by its file. When one role needs two efforts (a quick review of a small diff and a deep one of a risky diff), playbook ships two files for the role.

You do not edit the variants by hand. `playbook agents gen` writes them from the base agent, and `playbook agents check` fails CI when a variant drifts from its base. The orchestrator picks the variant by diff size and risk, following the `delegating-subagents` skill.

### Who decides the effort

Playbook sets the defaults, in each file. You set the ceiling.

Playbook has its own ceiling, the config key `maxEffortLevel` (same name and values as Claude Code's), set with `playbook effort <level>`. Claude Code has one too, `maxEffortLevel`, and Claude Code always has the last word. The lower of the two applies:

- Playbook `xhigh`, Claude Code `max`: `xhigh` applies, because `xhigh` is below `max`.
- Playbook `xhigh`, Claude Code `medium`: `medium` applies. Playbook never goes above Claude Code.
- Playbook `auto` (the default): playbook adds no ceiling, and Claude Code's applies if it has one.
- A ceiling never raises anything. An agent that ships at `low` still runs at `low`.

Playbook only reads Claude Code's `maxEffortLevel` (from the user, project and local settings files) and never writes it. When playbook's ceiling is the lower one, the launcher (`pb` or `cc`) passes it to that one session with `--settings`, and Claude Code applies it to command, skill and agent effort alike. A session started without the launcher gets no playbook ceiling. `playbook effort` with no level shows both values and the winner, and `--json` prints the same for scripts. Claude Code's own `/effort` and `--effort` still work below the ceiling, and the setting needs Claude Code 2.1.267 or later.

Planned work:

- A ceiling per skill, command and agent (#573).
- A shared rule for which tier a task goes to, and when to ask you first (#575).
- A rule that every model is the 5.5 generation, with a fallback to the previous one (#574).
- Measured, then auto tuned, effort values (#576, #578).

## Why hooks are in Rust and wired by `init`

Hooks are the safety layer, so they must be fast, must not depend on a script on disk that an update could break, and must fail safe. They are subcommands of one binary (`playbook hook <name>`). `playbook init` wires them into `settings.json`, and `playbook doctor` checks they are wired. See [Launcher and hooks](../internals/01-launcher-and-hooks.md).

## Why the docs live in layers

| Layer | Answers |
|---|---|
| README | What is this, and how do I install and start? |
| Concepts | Why is it built this way? (this page, the system prompt, memory) |
| Guides | How do I do a task end to end? |
| Authoring | How do I add or change a piece? |
| Internals | How does it work underneath? |
| ADRs | Why did we choose this over the alternatives, and when? |

When you want to know why something is the way it is, start with this page, then follow its links, then check the ADR in `docs/adr/` that names the decision.

## Where to find a setting

| You want to change | Look at |
|---|---|
| Which model a command or agent uses | The `model:` line in its file, then [Model routing](../internals/02-model-routing-and-memory.md) |
| How hard it thinks | The `effort:` line in its file, or pick a `-low` or `-xhigh` variant |
| The most effort anything may use | `playbook effort <level>` |
| How far the PR flow goes | [Config keys](../guides/04-config-keys.md) |
| Whether playbook asks or decides | `playbook mode`, and the Auto mode section of the README |
| What a hook blocks | [Launcher and hooks](../internals/01-launcher-and-hooks.md) |
