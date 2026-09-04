# Authoring Commands, Skills, and Hooks

Three extension points let you add behavior to this config: slash commands, skills, and hooks. Each has a specific directory and a specific format. Templates below are derived from the real files.

## Slash commands

Commands live in `commands/<name>.md` and run as `/<name>` inside a session.

Frontmatter fields that appear in the existing commands:

| Field | Purpose |
|---|---|
| `description` | Both the picker summary and the trigger. Claude Code lists commands in the same available-skills listing it uses for `skills/`, matching this field against what the user asked for, so a command can load from a plain-English request with no slash typed. Write it as a "use when..." sentence, the same as a skill's. |
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

A command's `description` does double duty. It is the one line a human reads in the picker, and it is the text Claude matches a plain-English request against when deciding whether to reach for the command. `adr.md` shows the shape: "Use when recording a significant, hard-to-reverse architectural decision." Name the situation, not the mechanics. The mechanics belong in the body, which only loads once the command actually runs. A description that is a feature list ("Executes X, delegating to Y, then Z") reads fine in the picker and matches nothing.

Weigh the cost of a wrong match before sharpening a trigger. A skill loads text and nothing else, so a false positive costs a few tokens. A command runs a procedure, sometimes a long interview or a write-and-commit pass, so a false positive costs the user's turn. For a heavyweight command, describe the precondition ("Use when an approved plan already exists") rather than the casual phrase that might have prompted it, and let `prompts/SYSTEM_PROMPT.md` decide whether a match launches or offers. A description names its own trigger; the system prompt sets the policy for what happens on a match.

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

Hooks are commands registered in `settings.json` under the `hooks` key. Each entry maps an event to one or more commands.

Events wired in this config: `SessionStart`, `PreToolUse`, `PostToolUse`, `UserPromptSubmit`, `PreCompact`, `Stop`, `SessionEnd`. `PreToolUse` and `PostToolUse` accept an optional `matcher` to filter by tool name (e.g., `"Bash"`, `"Read|Grep|Glob|Edit|Write|NotebookEdit"`).

### One binary, one module per hook

All fifteen hooks are Rust functions compiled into the single `playbook` binary, one module per hook under `src/hooks/<name>.rs`. `src/hooks/mod.rs` declares every module and an exhaustive `dispatch` match over the `HookName` enum in `src/lib.rs`, so adding a hook the CLI cannot invoke fails the build instead of silently doing nothing at runtime.

There is no language split left to choose between. The data-shaping hooks (memory graph rebuild, anchor lookup, frontmatter parsing) and the fast safety guards (`rm-workspace-guard`, `no-slop-guard`, `bg-await-guard`, `precommit-check`) are all plain functions sharing the same `src/common/` helpers (payload field extraction, session dir, atomic append, the `emit_*` JSON shapes) and the same compiled binary, so there is no per-hook cold start to weigh a language choice against.

**Registering a hook in `settings.json`:**

```json
"hooks": {
  "PreToolUse": [
    {
      "matcher": "Bash",
      "hooks": [
        {
          "type": "command",
          "command": "playbook hook rm-workspace-guard"
        }
      ]
    }
  ]
}
```

`src/init/wire.rs` writes this bare `playbook hook <name>` form for every hook when it generates `settings.json`: no absolute path to a script, since the binary is already on `PATH`.

**Input/output contract:**

Every hook receives a JSON payload on stdin. `main.rs` parses it once into a `Payload` and passes it to the dispatched module, which reads fields with `field`:

```rust
let tool = payload.field(".tool_name");            // PreToolUse: which tool fired
let path = payload.field(".tool_input.file_path"); // Read: the file being read
let source = payload.field(".source");              // SessionStart: "startup" or "resume"
```

To inject output back to Claude, write JSON to stdout:

```json
{
  "systemMessage": "Text shown to the user in the session.",
  "hookSpecificOutput": {
    "hookEventName": "PreToolUse",
    "additionalContext": "Context injected into Claude's next turn."
  }
}
```

The `src/common/emit.rs` helpers (`emit_pre_context`, `emit_pre_deny`, `emit_prompt_context`, `emit_system_message`, `emit_block`) build this JSON for you. A hook must never break Claude Code, so swallow errors and emit nothing rather than panicking; `hooks::dispatch` is tested against malformed and missing-field payloads for every hook name to hold that guarantee.

**Minimal hook template:**

```rust
// SPDX-FileCopyrightText: 2026 Your Name
// SPDX-License-Identifier: MIT

use crate::common::emit_pre_context;
use crate::common::payload::Payload;

pub fn run(payload: &Payload) {
    let tool = payload.field(".tool_name");

    // Your logic here.

    // Emit context when needed.
    emit_pre_context("PreToolUse", "your message");
}
```

Wire it in by adding a `pub mod` line and a match arm in `src/hooks/mod.rs`'s `dispatch`, plus a variant on `HookName` in `src/lib.rs`. Both are exhaustive, so a missing arm fails the build rather than the hook silently doing nothing.

Hook and settings changes take effect on a fresh session, not a resumed one. After editing `settings.json` or rebuilding the binary, run `cc fresh` (or plain `claude`). Resumed sessions run the config snapshot from their original startup; `cc` warns you when the config has drifted.

## See also

- [Internals: Launcher and Hooks](../internals/01-launcher-and-hooks.md): the hook lifecycle and launcher internals.
- [Decisions and Memory](../guides/03-decisions-and-memory.md): authoring memory facts.
- [Docs index](../index.md)
