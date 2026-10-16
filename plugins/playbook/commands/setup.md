---
description: Use on first install or to repair the local setup. Wires safety guards, settings.json, launchers and system prompt.
allowed-tools: Bash, Read, AskUserQuestion
argument-hint: "[--install-aliases] [--use-system-prompt] [--yes] [--auto] [--ask]"
model: haiku
effort: low
---

# Setup

Wire the optional playbook binary, once it is installed: the always-on safety guards, settings.json, and the tool map printed at session start. Optionally
add the `playbook` shell wrapper (it follows the session launcher into a new worktree) and install the custom system prompt. Each step
is idempotent; re-running /playbook:setup is safe and only changes what is missing.

The plugin's skills and commands work without the binary: where a step can use a tool, they fall back to plain git, gh and file steps. This command, like /playbook:doctor, needs the binary because it is the binary's wiring step. Install the binary first with the hint that /playbook:doctor prints for layer 6.

## Step 0: Read the run mode

Do this first. If `command -v playbook` finds nothing, stop with one line: "playbook is not on PATH. If you just installed it, open a new terminal and run /playbook:setup again; otherwise run `curl -fsSL https://raw.githubusercontent.com/pragmatic-engineer/playbook/main/install.sh | bash` first." Otherwise read the run mode for this run's arguments. Intent: run-mode. If the session's tool map (printed at session start) lists it, use that command, then read `skills/delegating-subagents/references/run-context.md` in this plugin and follow it for the warning, the both-flags exit, any other failure, and an outdated binary. Otherwise do this: if `$ARGUMENTS` holds both `--auto` and `--ask`, stop with one line saying they conflict; `--auto` or `--ask` alone sets the mode; with neither, use `PLAYBOOK_MODE` when it is `auto` or `ask`; else the mode is `ask`.

If the mode is `auto`, stop here. Print one line and nothing else: "/playbook:setup needs a person to run it, because it changes your global settings and shell files outside this repo." Do not run any later step.

In `ask` mode, behave exactly as this file describes.

## Step 1: Parse arguments

Parse `$ARGUMENTS`. Step 0 already read `--auto` and `--ask`, so they are not typos.

If `$ARGUMENTS` contains any of `--install-aliases`, `--use-system-prompt`, or
`--yes`, run non-interactively. Skip the questions in Step 2. Build the flag
list from the arguments and go straight to Step 3.

Anything else in `$ARGUMENTS` is ignored with a one-line warning. Don't abort:
a typo'd flag must not silently fall through to the interactive path, because
the user asked for a non-interactive run.

## Step 2: Ask (interactive mode only)

If no flags were found in `$ARGUMENTS`, call the AskUserQuestion tool ONCE
with these two questions:

**Question 1**

- header: "Aliases"
- question: "Add the playbook shell wrapper for the session launchers?"
- options:
  - label: "Yes (Recommended)"
    description: "Adds a playbook shell function so the session launcher can leave you in a new worktree. Bash and zsh both supported."
  - label: "No"
    description: "Skip the wrapper. The launcher still works, it just will not cd into a new worktree. Skills, commands, and hooks still work."

**Question 2**

- header: "System prompt"
- question: "Install the custom system prompt?"
- options:
  - label: "Yes (Recommended)"
    description: "Installs the senior-engineer persona and rules; the session launcher loads it each session. Recommended for the full experience."
  - label: "No"
    description: "Skip the persona. The plugin content still works without it."

## Step 3: Build the flag list and run

Build the flag list. The names differ on purpose: the `/playbook:setup` slash
command takes `--install-aliases` and `--use-system-prompt`, and the
`playbook setup` binary command takes `--aliases` and `--system-prompt`.
Translate, don't pass through.

`--yes` alone (no `--install-aliases`, no `--use-system-prompt`) means "don't
ask, take the recommended answer to every question" - the standard meaning of
`-y`/`--yes` on a CLI. Since both Q1 and Q2's recommended option is "Yes",
bare `--yes` is equivalent to passing `--use-system-prompt` for the purposes
of the two bullets below (which already cascades to `--aliases`, since
installing the system prompt implies the launchers). It is NOT equivalent to
passing neither flag: silently skipping both optional layers on a
non-interactive run would contradict what `--yes` says it does.

- Add `--aliases` if Q1 answer is "Yes (Recommended)" OR Q2 answer is
  "Yes (Recommended)" OR `--install-aliases`, `--use-system-prompt`, or
  `--yes` was in `$ARGUMENTS`.
- Add `--system-prompt` if Q2 answer is "Yes (Recommended)" OR
  `--use-system-prompt` or `--yes` was in `$ARGUMENTS`.
- Always add `--hooks` and `--statusline`.

`playbook setup` always runs. This command has no terminal, so setup asks none of its own questions. Without a terminal and without `--yes`, setup leaves the hooks, the status line and telemetry off unless a flag turns them on, which is why the list always has `--hooks` and `--statusline`. PATH stays on by default. It also marks `~/.config/playbook` as trusted in an existing `~/.claude.json`, so Claude Code's trust dialog never blocks that folder. Then check the tools the plugin needs and install only the missing ones:

```bash
playbook setup [flags]
playbook deps ensure --builtin
```

The binary carries its own settings template, system prompt and Brewfile, so neither command needs a plugin file.

When the setup command fails, check these cases in order:

1. It fails with "unrecognized subcommand": the binary predates the rename (older versions: `playbook init`). Run `playbook init` with the same flags, and use that name for the rest of this run. The two names run the same code.
2. It fails with "unexpected argument": the installed binary is older than the plugin. Run the same command again without that flag, and tell the user to upgrade with `curl -fsSL https://raw.githubusercontent.com/pragmatic-engineer/playbook/main/install.sh | bash`.
3. It reports the settings step skipped with "no template shipped": the binary is also older than the plugin. Give the same upgrade advice, because no guards were wired.
4. Any other non-zero exit: still run the second command, then report which setup steps failed.

If `playbook deps ensure --builtin` fails with "unrecognized subcommand" or "unexpected argument", skip it and say so.

## Step 4: Report

Report the output of both commands verbatim. Then say in one line what this run installed or changed: the binary (if the shim installed it just now), the hooks and status line in `settings.json`, `PATH`, and the launcher or system prompt only if you added them. `playbook setup` prints one line per
step with its status (for example, `[=] statusline  already up to date` when nothing changed), then the totals and next steps.

End your report with:

"Re-running /playbook:setup is safe and only changes what is missing. Run /playbook:doctor to
verify the full status."
