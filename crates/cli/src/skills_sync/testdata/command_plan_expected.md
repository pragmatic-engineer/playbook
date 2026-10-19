---
name: playbook-plan
description: Turn an idea into a plan.
---

This is the playbook `/plan` command for Codex. Run it as `$playbook-plan`. The text after the skill name is `$ARGUMENTS`. Where a step names the `Agent` tool, `subagent_type`, `SendMessage` or `AskUserQuestion` and they are not available, do that step yourself or ask in chat.

# Plan

```sh
claude -p "$playbook-fix --auto"
```

Then run $playbook-implement.
