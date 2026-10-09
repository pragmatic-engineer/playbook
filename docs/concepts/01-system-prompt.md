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

**Output rules.** Spartan: no filler words, no pleasantries, no hedging, no trailing summaries. No em or en dashes. Expansion is reserved for security warnings or multi-step sequences where order matters. One clarifying question only, and only when the request is ambiguous on a design decision with lasting effects.

**Writing voice.** A separate section governs human-facing prose (PR descriptions, review comments, tickets, Slack). It's deliberately warmer than the output rules and defers to the `playbook:writing-style` skill for the full rule set. The two voices are kept distinct on purpose: the output rules are for replies to you; the writing rules are for content other humans read.

**Code rules.** This section covers the full development workflow:

- Design work uses this repo's own `/playbook:plan` and `/playbook:adr` commands, not the built-in plan mode; those already run on Opus by their own frontmatter.
- Casual phrasing maps to the planning pipeline: any idea or feature request, whether raw or already settled, runs `/playbook:plan`, and `/playbook:implement` only runs when an approved plan or blueprint already exists.
- Haiku for spawned subagents on mechanical or search tasks. Escalate to Sonnet for real coding, Opus for architecture. `playbook route <kind>` gives the table.
- Fan out independent subtasks via parallel `Agent` calls. Close agents the moment their work is done.
- Read surrounding code and trace request paths before writing anything.
- No magic values: a number or string a reader would have to look up, or that repeats, gets a name.
- Verify empirically, don't guess. Test-driven development. Self-review after every implementation pass.
- Never force-push to shared branches. Never `--no-verify`. Commit and push only when asked.

**Security tiers.** Three tiers govern offensive-security requests. Tier 1 is open: defensive engineering, CVE analysis, malware analysis, CTF write-ups. Tier 2 requires a named scope (lab, CTF, or in-scope engagement): working exploits, C2 and red-team tradecraft, active recon, privilege escalation. Tier 3 is refused: attacks on named third parties without authorization, deployment-ready malware, mass-impact payloads. Vague scope ("educational purposes", "for a friend") doesn't qualify for Tier 2.

**Memory protocol.** One store at `~/.config/playbook/memory/`: global facts live flat at the root, org facts are namespaced under `~/.config/playbook/memory/<owner>/`, project facts under `~/.config/playbook/memory/<owner>/<repo>/`. The prompt covers when and where to save a fact, the YAML frontmatter format, edge types (`supersedes`, `depends_on`, `relates_to`, `contradicts`), traversal rules, and code anchors. See [Internals: Model Routing and Memory](../internals/02-model-routing-and-memory.md) for the full protocol.

## Why a custom prompt helps

The default session starts blank, so you would repeat your preferences every time. The prompt front-loads them once.

- **Calibration.** It replaces hedged, chatty defaults with the output rules before you type.
- **Guardrails.** Verify before claiming, self-review, no force-push and security tiers become starting constraints.
- **Model routing.** Opus for design, Sonnet for coding, Haiku for mechanical subagents, plus when to fan out.
- **Memory.** Claude knows where the store is and how to write a fact.

## Costs and limits

A system prompt is instructions, not enforcement. The model will still drift mid-session, especially on long threads. Hooks (in `hooks/`) are the enforced layer: PreToolUse and PostToolUse callbacks run outside the model's control and don't bend to conversational pressure. The prompt and the hooks are complementary; neither is sufficient alone.

Prompt changes don't apply to resumed sessions. Start fresh with `ccc fresh` or `ccc clean` to pick them up.

## See also

- [Memory system](02-memory-system.md)
- [Internals: Model Routing and Memory](../internals/02-model-routing-and-memory.md)
- [Docs index](../index.md)
