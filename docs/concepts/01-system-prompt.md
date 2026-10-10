# The Custom System Prompt

`prompts/SYSTEM_PROMPT.md` loads at the start of every `ccc` session and sets the persona, output rules, and operating constraints Claude follows from the first token. Without it, Claude starts from its training defaults: generally helpful, but uncalibrated to this workflow. This prompt closes that gap.

## How it loads

The `ccc` launcher (`playbook cc launch`, run by the functions `playbook shell-init` prints) passes the file to `claude` via `--system-prompt-file`:

```sh
claude --system-prompt-file "$HOME/.config/playbook/prompts/SYSTEM_PROMPT.md" ...
```

Both `ccc` and `ccd` carry the flag. `ccd` adds `--dangerously-skip-permissions` for unattended work; otherwise they're identical. If you invoke `claude` directly (bypassing `ccc`), the system prompt won't load.

The prompt locks in once, at the start of a fresh session. Resumed sessions inherit whatever was loaded when they started. That's the main tradeoff: changes to `SYSTEM_PROMPT.md` don't take effect until you start fresh. Use `ccc fresh` to open a new session, or `ccc clean` to fork the current conversation history into a new one with config reloaded.

## What it defines

**Persona.** Senior principal engineer, cybersecurity specialization, knowledge cutoff January 2026. This shapes tone, technical depth, and how Claude approaches tradeoffs. The prompt also instructs Claude to search when current state matters, rather than answer from memory.

**Output rules.** The Concise & Direct output style (`output-styles/concise-direct.md`) owns tone, length and format. The prompt keeps two rules that must hold with or without it: no em or en dashes anywhere, and one clarifying question only, and only when the request is ambiguous on a design decision with lasting effects.

**Writing voice.** A separate section governs human-facing prose (PR descriptions, review comments, tickets, Slack). It's deliberately warmer than the output rules and defers to the `playbook:writing-style` skill for the full rule set. The two voices are kept distinct on purpose: the output rules are for replies to you; the writing rules are for content other humans read.

**Code rules.** This section covers the full development workflow:

- Design work uses this repo's own `/playbook:plan` and `/playbook:adr` commands, not the built-in plan mode. Plan runs without asking, ADR and implement are offered and wait for a yes, and implement only runs when an approved plan or blueprint exists. The `auto-model-detect` hook nudges on common phrasing in interactive runs.
- Models and effort are not in the prompt. Each agent's frontmatter sets them, and `playbook route <kind>` prints the table.
- Fan out independent subtasks via parallel `Agent` calls. Close agents the moment their work is done.
- Comments stay rare and short. `no-slop-guard` blocks the worst cases at the tool call. The `playbook:engineering-standards` skill carries the rest of the code rules, including no magic values and testing.
- Self-review after every implementation pass. Commit and push only when asked. Never force-push to a shared branch.
- Three rules moved out of the prompt into the `policy-guard` hook, which denies them: no `--no-verify` or `--no-gpg-sign`, no hand-run `gh pr create` (use `/playbook:create-pull-request`), and no write to Claude Code's own memory.

**Security scope.** Working exploits, C2 and red-team tradecraft, active recon, privilege escalation and phishing infrastructure need a named scope (lab, CTF, or in-scope engagement). Vague scope ("educational purposes", "for a friend") does not qualify. Claude Code's own policy already covers the open and refused cases.

**Memory protocol.** One store at `~/.config/playbook/memory/`: global facts live flat at the root, org facts are namespaced under `~/.config/playbook/memory/<owner>/`, project facts under `~/.config/playbook/memory/<owner>/<repo>/`. The prompt covers where and when to save a fact and keeps the store separate from Claude Code's own memory. The file format (frontmatter, edge types, code anchors) is shown by the `policy-guard` hook the moment a fact file is written, so it costs nothing until it is needed. See [Internals: Model Routing and Memory](../internals/02-model-routing-and-memory.md) for the full protocol.

## Why a custom prompt helps

The default session starts blank, so you would repeat your preferences every time. The prompt front-loads them once.

- **Calibration.** It replaces hedged, chatty defaults with the output rules before you type.
- **Guardrails.** Self-review, no force-push and the security scope rule become starting constraints.
- **Routing.** It points at the agent files and `playbook route` for models, and says when to fan out.
- **Memory.** Claude knows where the store is and how to write a fact.

## Costs and limits

A system prompt is instructions, not enforcement. The model will still drift mid-session, especially on long threads. Hooks (in `hooks/`) are the enforced layer: PreToolUse and PostToolUse callbacks run outside the model's control and don't bend to conversational pressure. The prompt and the hooks are complementary; neither is sufficient alone.

Prompt changes don't apply to resumed sessions. Start fresh with `ccc fresh` or `ccc clean` to pick them up.

## See also

- [Memory system](02-memory-system.md)
- [Internals: Model Routing and Memory](../internals/02-model-routing-and-memory.md)
- [Docs index](../index.md)
