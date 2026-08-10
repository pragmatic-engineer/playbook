# The Custom System Prompt

`prompts/SYSTEM_PROMPT.md` loads at the start of every `cc` session and sets the persona, output rules, and operating constraints Claude follows from the first token. Without it, Claude starts from its training defaults: generally helpful, but uncalibrated to this workflow. This prompt closes that gap.

## How it loads

The `cc` launcher (`shell/zsh/cc.zsh`) passes the file to `claude` via `--system-prompt-file`:

```zsh
cc()  { _claude --system-prompt-file "$HOME/.claude/prompts/SYSTEM_PROMPT.md" "$@"; ... }
ccd() { _claude --dangerously-skip-permissions --system-prompt-file "$HOME/.claude/prompts/SYSTEM_PROMPT.md" "$@"; ... }
```

Both wrappers carry the flag. `ccd` adds `--dangerously-skip-permissions` for unattended work; otherwise they're identical. If you invoke `claude` directly (bypassing `cc`), the system prompt won't load.

The prompt locks in once, at the start of a fresh session. Resumed sessions inherit whatever was loaded when they started. That's the main tradeoff: changes to `SYSTEM_PROMPT.md` don't take effect until you start fresh. Use `cc fresh` to open a new session, or `cc clean` to fork the current conversation history into a new one with config reloaded.

## What it defines

**Persona.** Senior principal engineer, cybersecurity specialization, knowledge cutoff January 2026. This shapes tone, technical depth, and how Claude approaches tradeoffs. The prompt also instructs Claude to search when current state matters, rather than answer from memory.

**Output rules.** Spartan: no filler words, no pleasantries, no hedging, no trailing summaries. No em or en dashes. Expansion is reserved for security warnings or multi-step sequences where order matters. One clarifying question only, and only when the request is ambiguous on a design decision with lasting effects.

**Writing voice.** A separate section governs human-facing prose (PR descriptions, review comments, tickets, Slack). It's deliberately warmer than the output rules and defers to the `writing-style` skill for the full rule set. The two voices are kept distinct on purpose: the output rules are for replies to you; the writing rules are for content other humans read.

**CLI environment.** RTK (Rust Token Killer) runs as a PreToolUse hook and rewrites Bash commands to cut token use. The prompt names the available meta-commands (`rtk gain`, `rtk discover`, `rtk proxy`) so Claude knows what's wired in without needing to discover it each session.

**Code rules.** This section covers the full development workflow:

- Plan mode first for design work. `settings.json` routes plan mode to Opus and execution to Sonnet.
- Haiku for spawned subagents on mechanical or search tasks (3x cheaper than Sonnet). Escalate to Sonnet for real coding, Opus for architecture.
- Fan out independent subtasks via parallel `Agent` calls. Close agents the moment their work is done.
- Read surrounding code and trace request paths before writing anything.
- Verify empirically, don't guess. Test-driven development. Self-review after every implementation pass.
- Never force-push to shared branches. Never `--no-verify`. Commit and push only when asked.

**Security tiers.** Three tiers govern offensive-security requests. Tier 1 is open: defensive engineering, CVE analysis, malware analysis, CTF write-ups. Tier 2 requires a named scope (lab, CTF, or in-scope engagement): working exploits, C2 and red-team tradecraft, active recon, privilege escalation. Tier 3 is refused: attacks on named third parties without authorization, deployment-ready malware, mass-impact payloads. Vague scope ("educational purposes", "for a friend") doesn't qualify for Tier 2.

**Memory protocol.** One store at `~/.claude/memory/`: global facts live flat at the root, project facts are namespaced under `~/.claude/memory/<owner>/<repo>/`. The prompt covers when and where to save a fact, the YAML frontmatter format, edge types (`supersedes`, `depends_on`, `relates_to`, `contradicts`), traversal rules, and code anchors. See [Internals: Model Routing and Memory](../internals/02-model-routing-and-memory.md) for the full protocol.

## Why a custom prompt behaves better

The default Claude session starts blank. To get consistent behavior you'd re-explain preferences, remind it not to guess, tell it to self-review. Every session. A system prompt front-loads all of that once, so the first message in a new session starts from the same baseline as the hundredth.

**Consistent calibration from the first message.** Claude's defaults produce hedged, verbose prose with pleasantries and trailing summaries. The Output rules replace that baseline before you type a word. You don't spend the first few exchanges correcting tone or re-stating preferences you've already written down.

**Guardrails the default doesn't include.** The verify-before-claiming rule, ruthless self-review, no force-push, security tier gating: Claude's training doesn't reliably enforce these. Without the verify rule, Claude asserts confidently without checking. Without self-review, it ships the first draft. Without the security tiers, it asks no questions on offensive requests. The prompt makes each of these an explicit starting constraint, not something you re-negotiate per session.

**Model routing by task type.** Opus for architectural planning, Sonnet for coding, Haiku for mechanical subagents. The prompt also defines when to fan out to parallel agents vs. stay inline, and when to escalate. Without clear routing guidance, model selection is ad-hoc and costs climb with it.

**Memory protocol in scope.** Claude knows the store exists, where to write facts, and how to do it correctly. Without this, durable facts get re-explained at the start of each session or lost entirely. The global root carries cross-project preferences; project facts are namespaced under `~/.claude/memory/<owner>/<repo>/`, isolated to a single repo and never committed.

**Net effect.** Fewer wrong-default choices. Less repeated instruction. More consistent output across sessions.

## Costs and limits

A system prompt is instructions, not enforcement. The model will still drift mid-session, especially on long threads. Hooks (in `hooks/`) are the enforced layer: PreToolUse and PostToolUse callbacks run outside the model's control and don't bend to conversational pressure. The prompt and the hooks are complementary; neither is sufficient alone.

Prompt changes don't apply to resumed sessions. Start fresh with `cc fresh` or `cc clean` to pick them up.

## See also

- [Memory system](02-memory-system.md)
- [Internals: Model Routing and Memory](../internals/02-model-routing-and-memory.md)
- [Docs index](../index.md)
