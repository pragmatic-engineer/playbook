---
description: Use on first install or to repair the local setup. Wires safety guards, seeds or merges settings.json, and asks whether to install the shell launchers and system prompt.
allowed-tools: Bash, Read, AskUserQuestion
argument-hint: "[--install-aliases] [--use-system-prompt] [--yes] [--auto] [--ask]"
model: sonnet
effort: low
---

# Setup

Wire the always-on safety guards and seed or merge settings.json. Optionally
install the shell launchers (ccc/ccd) and the custom system prompt. Each step
is idempotent; re-running /playbook:setup is safe and only changes what is missing.

## Step 0: Read the run mode

Do this first. If `command -v playbook` finds nothing, stop with one line: "playbook is not on PATH. If you just installed it, open a new terminal and run /playbook:setup again; otherwise run `curl -fsSL https://raw.githubusercontent.com/pragmatic-engineer/playbook/main/install.sh | bash` first." Otherwise read the mode from the CLI:

```bash
playbook mode status --json
```

If the arguments contain `--auto`, add `--flag auto`. If they contain `--ask`, add `--flag ask`. If both are present, stop with one line: "--auto and --ask conflict; pass one." The JSON has four keys: `mode` (`ask` or `auto`), `source` (where it came from), `hook_mode` and `warning`. If `warning` is not empty, print it once. If the command fails, do not stop and do not retry. Run in ask mode and print one line. When the error is `unrecognized subcommand 'mode'`, the installed binary is older than this plugin: print "playbook binary is older than the plugin, run `playbook update`."

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
- question: "Install the ccc and ccd shell launchers?"
- options:
  - label: "Yes (Recommended)"
    description: "Adds cc/ccd to your shell (session resume, model routing, transcript prune). Bash and zsh both supported."
  - label: "No"
    description: "Skip the launchers; run claude directly. Skills, commands, and hooks still work."

**Question 2**

- header: "System prompt"
- question: "Install the custom system prompt?"
- options:
  - label: "Yes (Recommended)"
    description: "Installs the senior-engineer persona and rules; ccc loads it each session. Recommended for the full experience."
  - label: "No"
    description: "Skip the persona. The plugin content still works without it."

## Step 3: Build the flag list and run

Build the flag list. Note the names differ on purpose: this command takes
`--install-aliases` and `--use-system-prompt`, while `playbook init` takes
`--aliases` and `--system-prompt`. Translate, don't pass through.

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

`playbook init` always runs. This command has no terminal, so init does not ask its own questions and uses its defaults (hooks, settings and PATH on), plus the launcher and system prompt flags you built above. It also marks `~/.config/playbook` as trusted in an existing `~/.claude.json`, so Claude Code's trust dialog never blocks that folder. Then check the tools the plugin needs and install only the missing ones:

```bash
CLAUDE_PLUGIN_ROOT="${CLAUDE_PLUGIN_ROOT}" playbook init [flags]
playbook deps ensure "${CLAUDE_PLUGIN_ROOT}/Brewfile"
```

If `playbook init` exits non-zero, still run the second command, then report which init steps failed. If it fails with "unexpected argument", the installed binary is older than the plugin: run `playbook init` again without that flag and tell the user to upgrade with `curl -fsSL https://raw.githubusercontent.com/pragmatic-engineer/playbook/main/install.sh | bash`. If `playbook deps --help` fails the same way, skip the second command and say so.

## Step 4: Report

Report the output of both commands verbatim. `playbook init` prints one line per
step with its status (for example, "ok - already up to date" when nothing changed).

End your report with:

"Re-running /playbook:setup is safe and only changes what is missing. Run /playbook:doctor to
verify the full status."
