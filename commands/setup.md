---
description: Interactive setup for pragmatic-engineer/playbook. Wires safety guards, seeds or merges settings.json, and asks whether to install the shell launchers and system prompt.
allowed-tools: Bash, Read, AskUserQuestion
argument-hint: "[--install-aliases] [--use-system-prompt] [--yes]"
model: sonnet
effort: low
---

# Setup

Wire the always-on safety guards and seed or merge settings.json. Optionally
install the shell launchers (cc/ccd) and the custom system prompt. Each step
is idempotent; re-running /setup is safe and only changes what is missing.

## Step 1: Parse arguments

Parse `$ARGUMENTS`.

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
- question: "Install the cc and ccd shell launchers?"
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
    description: "Installs the senior-engineer persona and rules; cc loads it each session. Recommended for the full experience."
  - label: "No"
    description: "Skip the persona. The plugin content still works without it."

## Step 3: Build the flag list and run

Build the flag list. Note the names differ on purpose: this command takes
`--install-aliases` and `--use-system-prompt`, while `setup-local.sh` takes
`--aliases` and `--system-prompt`. Translate, don't pass through.

- Add `--aliases` if Q1 answer is "Yes (Recommended)" OR Q2 answer is
  "Yes (Recommended)" OR `--install-aliases` or `--use-system-prompt` was in
  `$ARGUMENTS`.
- Add `--system-prompt` if Q2 answer is "Yes (Recommended)" OR
  `--use-system-prompt` was in `$ARGUMENTS`.

The script always runs (guards and settings run regardless of the answers):

```bash
bash "${CLAUDE_PLUGIN_ROOT}/shell/setup-local.sh" [flags]
```

## Step 4: Report

Report the script's stdout output verbatim. It prints one line per item with
its status (for example, "already up to date" when nothing changed).

End your report with:

"Re-running /setup is safe and only changes what is missing. Run /doctor to
verify the full status."
