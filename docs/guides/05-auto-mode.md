# Auto mode

Auto mode lets a command run without asking you questions. It takes the recommended answer at each decision, lists it under Assumptions in the final output, and keeps going. The default is `ask`, where every command asks as usual.

## Turn it on

Playbook picks the first of these it finds:

1. The command flag: `--auto` or `--ask`.
2. The `PLAYBOOK_MODE` environment variable.
3. The `mode` config key.

```bash
playbook mode auto      # on for this repo, until you turn it off
playbook mode ask       # off
playbook mode status    # the mode and where it came from
```

Only you should run these. If auto comes from `PLAYBOOK_MODE`, unset the variable to leave it.

## What changes

- **Plan design.** Plain `--auto` never approves a design. `/playbook:plan --auto` answers only the Work Unit and Segment questions and stops at the design approval. Pass `--auto-design` to approve the design too. Every choice is logged.
- **Land boundary.** `/playbook:implement` never picks the `land` boundary on its own. Pass `--boundary=land`.

## What auto will not do

- `/playbook:setup`, `/playbook:adr` and `/playbook:address-pr-comments` refuse to run in auto and say why.
- Reviews never post to GitHub. `/playbook:quick-review` and `/playbook:deep-review` run as `--self`.
- `/playbook:learn-project` stages its candidates and writes nothing to live memory.
- Auto never force-pushes.
- Auto does not run the self-review on a new pull request.
- `playbook update` refuses unless you pass `--yes`.

## The hooks

Two hooks watch every session whose resolved mode is `auto`.

**Question hook (`auto-guard`).** It denies the question tool, so the model takes the recommended option. In the default permission mode it also advises, once per session, launching with a trusted permission mode. The standing auto-mode rule comes from `session-init` at SessionStart.

**Spend cap hook (`auto-cost`).** It totals what the session has cost. At `auto.warnPct` (default 70) of `auto.budgetUsd` (default 5, in US dollars) it warns once. At the cap it denies every tool except reading files, `git status`, `git diff`, `playbook mode status`, and writing a short note about where the work stopped.

- Cost is the larger of the session transcript total (subagent files included) and the cost the status line reports.
- If the cost cannot be read, the hook treats it as over the cap: it stops the session and asks the model to report, without a park note.
- The cap is per session, so `/clear` or a resume starts a new count.
- Change it with `playbook config set auto.budgetUsd <usd>`.

While auto is on, the model cannot switch the mode, change the `mode`, `auto.*` or `fix.*` settings, or edit the config files. Only you can, by typing the command yourself. At the cap it also cannot raise the cap. The one exception is an unreadable cost: then it may run `playbook mode ask` to leave auto.

## Flags do not reach hooks

The hooks read only `PLAYBOOK_MODE` and the config, never your command flags. If you run a command with `--auto` while the resolved mode is `ask`, you get auto behavior in the command text only, with no question hook and no spend cap. `playbook mode status --flag auto` prints a warning when the flag and the hooks disagree. To get the hooks, run `playbook mode auto` or set `PLAYBOOK_MODE=auto`. The reverse also holds: `--ask` while the mode is `auto` still has the question tool denied.

## Limits

The cap and the hooks guard against runaway spend and honest mistakes. They are not a security boundary against a model that tries to defeat them. Auto does not answer permission prompts, so run it in a permission mode you trust, or the session can stall.

`PLAYBOOK_HEADLESS` is a separate setting and does not turn auto on. See [Headless mode](../internals/05-headless-mode.md).

## See also

- [Config keys](04-config-keys.md): `mode`, `auto.budgetUsd`, `auto.warnPct`.
- [Plan and implement](01-plan-and-implement.md): how `--auto` flows through the commands.
