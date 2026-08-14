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
- **The four safety guards stay bash** (`hooks/rm-workspace-guard.sh`, `hooks/no-dash-guard.sh`, `hooks/bg-await-guard.sh`, `hooks/precommit-check.sh`). They fire on the `Bash`/`Edit` fast path and must fail safe. `bg-await-guard.sh`, `no-dash-guard.sh` and `precommit-check.sh` source `hooks/lib/common.sh`; `rm-workspace-guard.sh` deliberately does not, so a guard that blocks `rm` keeps working even if the shared library is broken or missing.

Both `common.py` and `common.sh` exist on purpose and expose the same helpers (payload field extraction, session dir, atomic append, the `emit_*` JSON shapes). Edit the one your hook's language uses; keep the two in step when you change a shared behaviour.

**Which language for a new hook?** Default to python on `common.py`. Choose bash only for a guard that must block on the `Bash` or `Edit` path and fail safe with the fewest dependencies. Do not choose bash for speed: see the timing note below, where a real guard measures 26 ms against python's 29 ms cold start.

**Timing (re-measured 2026-08-12, macOS arm64, average of 10 fires each).** An earlier note here claimed bash costs 7 ms against python's 35 to 41 ms. That understated bash by roughly 4x. Real per-fire cost:

| | ms |
|---|---:|
| `bash -c true` (floor) | 10 |
| `bg-await-guard.sh` | 26 |
| `python3` cold start | 29 |
| `rebuild-memory-graph.py` | 41 |
| `post-edit-track.py` | 46 |
| `search-counter.py` | 46 |
| `memory-anchors.py` | 53 |

A real bash guard costs 26 ms, within 3 ms of a bare python cold start, because it shells out to `jq` per field through `common.sh`. The python-versus-bash gap is 15 to 27 ms, not the ~30 ms claimed before.

**Hooks for one event run in parallel**, so an event costs about as much as its slowest hook, not the sum. Measured against live transcripts: `PreToolUse:Read` has a p50 of 57 ms over 731 recorded events while each of its three python hooks measures 46 to 53 ms alone. Serial would be ~145 ms.

So "choose bash for speed" is weaker than it looks. Pick bash for a guard when you want it to fail safe with the fewest moving parts, not because it is meaningfully faster.

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
