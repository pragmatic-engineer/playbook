# Authoring Commands, Skills, and Hooks

Three extension points let you add behavior to this config: slash commands, skills, and hooks. Each has a specific directory and a specific format. Templates below are derived from the real files.

## Slash commands

Commands live in `commands/<name>.md` and run as `/<name>` inside a session.

Frontmatter fields that appear in the existing commands:

| Field | Purpose |
|---|---|
| `description` | Short summary shown in the command picker. |
| `allowed-tools` | Comma-separated tools the command is permitted to use. |
| `effort` | Effort level: `low`, `medium`, `high`, `xhigh`. |
| `model` | Optional. Pin to a model (e.g., `opus`). Omit to inherit the session default. |
| `argument-hint` | Optional. Usage hint shown for `$ARGUMENTS`. |

The body is the instruction set Claude runs when the command is invoked. Write it as a numbered procedure or a set of rules. Reference `$ARGUMENTS` to access anything the user typed after the command name.

```markdown
---
description: One-line description of what this command does
allowed-tools: Bash, Read, Edit
effort: medium
---

# My Command

What it does and when to use it.

## Step 1

Instruction for Claude. Reference `$ARGUMENTS` here if needed.

## Step 2

Instruction for Claude.
```

`scope.md` adds `model: opus` because the planning interview needs that capability. `commit-and-push.md` omits `model` and inherits the session default. Only set `model` when the command always needs a specific model.

## Skills

Skills live in `skills/<name>/SKILL.md` and load on demand when the session decides a task matches.

The frontmatter has two fields:

| Field | Purpose |
|---|---|
| `name` | Machine identifier used to reference the skill. |
| `description` | The trigger. Claude reads this to decide whether to load the skill. |

The body is the content Claude gets when the skill loads: rules, templates, formats, decision tables. Write it as self-contained prose because the skill loads without surrounding context.

```markdown
---
name: my-skill
description: Use when doing X or Y. Covers rule-set Z.
---

# My Skill

Rules and formats go here.

## Section

- Rule one.
- Rule two.
```

The `description` field does all the targeting. Write it as a "use when..." sentence that names the task clearly. A vague description means the skill loads at the wrong time or not at all.

## Hooks

Hooks are executable scripts registered in `settings.json` under the `hooks` key. Each entry maps an event to one or more commands.

Events wired in this config: `SessionStart`, `PreToolUse`, `PostToolUse`, `UserPromptSubmit`, `PreCompact`, `Stop`, `SessionEnd`. `PreToolUse` and `PostToolUse` accept an optional `matcher` to filter by tool name (e.g., `"Bash"`, `"Read|Grep|Glob|Edit|Write|NotebookEdit"`).

### Two languages: python hooks, bash guards

The `hooks/` directory is deliberately mixed-language (ADR 0005):

- **The eleven non-guard hooks are python** (`hooks/*.py`), sharing `hooks/lib/common.py`. Python is the default for any new hook: the data-shaping hooks (memory graph rebuild, anchor lookup, frontmatter parsing) were carrying jq and awk that are far clearer as stdlib python, and one language for that work is easier to maintain.
- **The three safety guards stay bash** (`hooks/rm-workspace-guard.sh`, `hooks/no-dash-guard.sh`, `hooks/bg-await-guard.sh`), sharing `hooks/lib/common.sh`. They fire on the `Bash`/`Edit` fast path and must fail safe with the lowest possible startup cost.

Both `common.py` and `common.sh` exist on purpose and expose the same helpers (payload field extraction, session dir, atomic append, the `emit_*` JSON shapes). Edit the one your hook's language uses; keep the two in step when you change a shared behaviour.

**Which language for a new hook?** Default to python on `common.py`. Choose bash on `common.sh` only for a guard that runs on every single tool call and blocks on a hot path, where the interpreter startup cost matters (see the timing note below).

**Timing (measured 2026-08-11, macOS, average of 40 fires):** a python hook costs roughly 35 to 41 ms per fire versus 7 ms for the equivalent bash, almost entirely python interpreter startup. That ~30 ms is acceptable for the advisory non-guard hooks (they inject context, they do not block), and it is exactly why the guards stay bash: a guard pays that cost on the critical path of every `Bash` and `Edit`.

**Registering a hook in `settings.json`:**

```json
"hooks": {
  "PreToolUse": [
    {
      "matcher": "Bash",
      "hooks": [
        {
          "type": "command",
          "command": "~/.claude/hooks/my-hook.py"
        }
      ]
    }
  ]
}
```

**Input/output contract:**

Hook scripts receive a JSON payload on stdin. A python hook imports `lib/common.py` and reads fields with `field`:

```python
import os, sys
sys.path.insert(0, os.path.join(os.path.dirname(os.path.abspath(__file__)), "lib"))
import common as c

tool = c.field(".tool_name")               # PreToolUse: which tool fired
path = c.field(".tool_input.file_path")    # Read: the file being read
source = c.field(".source")                # SessionStart: "startup" or "resume"
```

A bash guard sources `lib/common.sh` instead and uses the same-named helper `hi_field '.tool_name'`. To inject output back to Claude, write JSON to stdout:

```json
{
  "systemMessage": "Text shown to the user in the session.",
  "hookSpecificOutput": {
    "hookEventName": "PreToolUse",
    "additionalContext": "Context injected into Claude's next turn."
  }
}
```

The `common` helpers (`emit_pre_context`, `emit_pre_deny`, `emit_prompt_context`, `emit_system_message`) build this JSON for you. Exit 0 in all normal cases; a hook must never break Claude Code, so swallow errors and emit nothing rather than raising.

**Minimal hook template (python, the default):**

```python
#!/usr/bin/env python3
# SPDX-FileCopyrightText: 2026 Your Name
# SPDX-License-Identifier: MIT
import os, sys
sys.path.insert(0, os.path.join(os.path.dirname(os.path.abspath(__file__)), "lib"))
import common as c


def main() -> int:
    tool = c.field(".tool_name")

    # Your logic here.

    # Emit context when needed.
    c.emit_pre_context("PreToolUse", "your message")
    return 0


if __name__ == "__main__":
    sys.exit(main())
```

Make it executable (`chmod +x`) and give it the `#!/usr/bin/env python3` shebang so `settings.json` can invoke it directly. A new safety guard instead uses `#!/usr/bin/env bash`, sources `lib/common.sh`, and builds output with `jq -cn`.

Hook and settings changes take effect on a fresh session, not a resumed one. After editing `settings.json` or a hook script, run `cc fresh` (or plain `claude`). Resumed sessions run the config snapshot from their original startup; `cc` warns you when the config has drifted.

## See also

- [Internals: Launcher and Hooks](../internals/01-launcher-and-hooks.md): the hook lifecycle and launcher internals.
- [Decisions and Memory](../guides/03-decisions-and-memory.md): authoring memory facts.
- [Docs index](../index.md)
