---
description: Check the four playbook layers and print a status table with a remediation hint for each miss.
allowed-tools: Bash, Read
argument-hint: ""
model: sonnet
effort: low
---

# Doctor

Run all four checks below. Do not stop early if one fails. Then print a
status table with one row per layer.

## Layer 1: Plugin enabled

```bash
claude plugin list 2>/dev/null | grep -qi 'playbook'
```

Pass if the output contains "playbook" and the status shows it is enabled.

Remediation hint on miss: "run: claude plugin marketplace add pragmatic-engineer/marketplace && claude plugin install playbook@pragmatic-engineer"

## Layer 2: Safety guards wired

```bash
jq '[.hooks.PreToolUse[]?.hooks[]?.command]
    | map(select(test("rm-workspace-guard|bg-await-guard|no-dash-guard")))
    | length' ~/.claude/settings.json 2>/dev/null
```

Pass if the result is 3 or more.

Remediation hint on miss: "run /setup to seed settings.json with the guard hooks"

## Layer 3: Launcher (opt-in)

Detect the current shell:

```bash
basename "${SHELL:-}"
```

For zsh: pass if BOTH conditions hold:
1. `grep -qF 'shell/zsh/cc.zsh' ~/.zshrc 2>/dev/null`
2. `test -f ~/.claude/shell/zsh/cc.zsh`

For bash: pass if BOTH conditions hold:
1. `grep -qF 'shell/bash/cc.sh' ~/.bashrc 2>/dev/null`
2. `test -f ~/.claude/shell/bash/cc.sh`

For any other shell: report "shell not detected" and skip this check.

This layer is opt-in. Report "not installed (opt-in; run /setup)" rather than
a hard fail when either condition is false.

Remediation hint when not installed: "run /setup and choose Yes for the launcher question"

## Layer 4: System prompt (opt-in)

```bash
test -f ~/.claude/prompts/SYSTEM_PROMPT.md
```

This layer is opt-in. Report "not installed (opt-in, recommended)" rather than
a hard fail when the file is absent.

Remediation hint when not installed: "run /setup and choose Yes for the system prompt question"

## Output format

Print a table with one row per layer. Use a clear status marker and a brief
label. For opt-in layers that are not installed, use a neutral marker (for
example INFO or SKIP) rather than FAIL. For each failing or missing item add a
one-line remediation hint. Example shape:

```
PASS  plugin enabled
PASS  safety guards wired (3 of 3)
INFO  launcher not installed (opt-in; run /setup)    -- run /setup and choose Yes for the launcher question
INFO  system prompt not installed (opt-in, recommended) -- run /setup and choose Yes for the system prompt question
```

If all required layers pass and optional layers are installed, say so in one
line. If any required layer fails, end with: "Run /setup to fix the items
above."
