# ADR-0015: Multi-Agent Support via a Thin Agent-Neutral Core and Per-Agent Adapters

- **Status:** Accepted
- **Date created:** 2026-10-07
- **Date modified:** 2026-10-07

## Context

Issue #301 asks for playbook to work with agents other than Claude Code (Cursor, OpenAI Codex CLI). Its own text says it needs a scoping pass before any implementation. No ADR on agent support exists, so each partial step risks encoding a different assumption.

**What already shipped toward this goal**

- ADR-0012 moved playbook-owned state from `~/.claude` to `$HOME/.config/playbook/` and cites #301 as the motivation. The storage prerequisite is done (#320, #321, #325, #327, #330).
- `UsageSource` (`src/usage/mod.rs`, #418) is the agent-neutral extension point for usage ingest. Its contract is `name()` plus `events_since(watermark)` returning normalized `UsageEvent` and `ToolInvocationEvent` values. The only implementation is `ClaudeCodeSource` (`src/usage/claude_code.rs`).

**What is still Claude-specific**

- `init` keys on `CLAUDE_PLUGIN_ROOT` (`src/init/self_root.rs`).
- Hooks, the statusline and the launcher are wired through Claude Code's `settings.json` and plugin cache.
- Commands, skills and agents ship as a Claude Code plugin (ADR-0001).
- No Cursor or Codex code exists anywhere in the repo.

**What the other agents offer today** (checked 2026-10-07; each row cites its source)

| Capability | Claude Code | Cursor | Codex CLI |
|---|---|---|---|
| Skills (`SKILL.md` with `name` and `description`) | `skills/<name>/SKILL.md` | `.cursor/skills/`, invoked as `/skill-name` (1) | `.agents/skills`, `~/.agents/skills`, invoked as `$skill-name` (2) |
| Slash commands | `commands/*.md` | Commands being folded into skills, `/migrate-to-skills` (1) | `~/.codex/prompts/*.md` is deprecated in favour of skills (3) |
| Subagents | `agents/*.md` | `.cursor/agents/`, also reads `.claude/agents/` and `.codex/agents/` (4) | Not confirmed in the docs reviewed |
| Hooks, pre and post tool | `hooks.json` in the plugin, `PreToolUse` and `PostToolUse` | `.cursor/hooks.json`, `preToolUse`, `postToolUse`, `beforeShellExecution`, `sessionStart`, `stop` (5) | `~/.codex/hooks.json` or `config.toml`, plus project `.codex/`, with `PreToolUse`, `PostToolUse`, `SessionStart`, `UserPromptSubmit`, `Stop` (6) |
| Hook protocol | JSON on stdin, JSON decision on stdout | JSON on stdin and stdout, `allow`, `deny` or `ask` (5) | JSON on stdin, JSON output with `continue` and `systemMessage` (6) |
| Settings file | `settings.json` | `.cursor/` files and `~/.cursor/` | `~/.codex/config.toml` (2) |
| Always-on instructions | `CLAUDE.md` | `.cursor/rules/*.mdc` and `AGENTS.md` (7) | `AGENTS.md` |
| Session transcripts | `~/.claude/projects/**/*.jsonl` | Not confirmed; no public transcript format found | `~/.codex/sessions/**/rollout-*.jsonl`, with `event_msg` entries of type `token_count` carrying cumulative `total_token_usage` (8, 9) |

Sources: (1) https://cursor.com/docs/agent/chat/commands, (2) https://learn.chatgpt.com/docs/build-skills, (3) https://learn.chatgpt.com/docs/custom-prompts, (4) https://cursor.com/docs/agent/subagents, (5) https://cursor.com/docs/hooks, (6) https://learn.chatgpt.com/docs/hooks, (7) https://cursor.com/docs/context/rules, (8) https://ccusage.com/guide/codex/, (9) https://github.com/openai/codex/issues/9660.

Two caveats on the evidence. The Codex session format comes from a third-party reader (ccusage) and an upstream issue, not from an OpenAI schema document, so it can change between Codex releases. Interactive sessions on older Codex builds did not always emit `token_count` events (9). Cursor's agent transcript format is unverified, which rules it out as a first usage source.

The convergence is the useful finding. All three agents use the same `SKILL.md` shape, the same JSON-over-stdio hook protocol with an allow or deny decision, and `AGENTS.md`-style instructions. The differences are the file locations, the event names and the settings format.

## Decision Drivers

- The maintainer's stated goal is to share the memory store and tooling across agents, with Claude Code staying the primary target.
- The first step must be reversible and must not change behaviour for existing Claude Code users.
- `UsageSource` already exists, so a second implementation tests the abstraction at the lowest cost.
- Agent file formats are still moving (Codex prompts are already deprecated), so playbook should not hard-code more than one thin seam per agent.
- Hooks are the highest-risk layer because they can block tool calls, so they should come only after a read-only slice proves the adapter shape.

## Considered Alternatives

### A. Stay Claude-only (effort: none)

- How it works: close #301 as out of scope and keep the current layout.
- Trade-offs: no cost, but the ADR-0012 storage work then has no consumer, and the plugin keeps its single-vendor dependency.

### B. Thin agent-neutral core in the `playbook` binary plus per-agent adapters, Claude-first (effort: L over several slices) (chosen)

- How it works: the binary owns what has no agent in it (memory graph, usage store, gate, state under `$HOME/.config/playbook/`). Each agent gets a small adapter that translates that agent's hook payloads, settings files and transcripts into the core's types. Claude Code stays the reference adapter and is never regressed.
- Trade-offs: needs one adapter per agent and a rule for what belongs in the core. Slices ship independently, and each is useful on its own.

### C. Rewrite everything to a lowest-common-denominator format and generate per-agent output (effort: XL)

- How it works: author commands, skills, agents and hooks once in a neutral schema, then generate Claude, Cursor and Codex files.
- Trade-offs: a generator is a large new surface that must track three moving formats. Most of the value (skills share one format already) arrives without it. Rejected as premature.

## Decision

Alternative B. Playbook gets a thin agent-neutral core in the `playbook` binary and per-agent adapters, Claude-first.

**Layer mapping**

| Layer | Class | Reason |
|---|---|---|
| Skills (`skills/*/SKILL.md`) | Portable | Same `SKILL.md` frontmatter in all three agents. Needs only install paths (`.agents/skills`, `.cursor/skills`), no content change. |
| Commands (`commands/*.md`) | Needs an adapter | Cursor and Codex fold commands into skills. Rendering each command as a skill is mechanical, but invocation syntax differs (`/x`, `$x`). |
| Agents (`agents/*.md`) | Needs an adapter | Cursor reads Claude-style subagent files. Codex support is unconfirmed, and tool and model fields differ. |
| Hooks (safety guards, session init, memory capture) | Needs an adapter | Same stdin and stdout JSON idea, but event names, matcher syntax and the settings file that registers them differ. The Rust hook logic is reusable behind a payload translator. |
| Statusline | Claude only | It is a Claude Code `statusLine` setting with no equivalent surfaced by the other two agents. |
| Usage ingest | Needs an adapter | `UsageSource` is the seam. Codex rollout files are the first non-Claude source. |
| Memory (graph, facts, recall) | Portable | Already agent-neutral on disk under `$HOME/.config/playbook/` (ADR-0012). Injection into a session goes through hooks, so injection is covered by the hooks adapter. |

**Sequence of slices**

1. A second `UsageSource` for Codex session logs (`~/.codex/sessions/**/rollout-*.jsonl`). It is read-only, adds no behaviour change for existing users, and exercises the abstraction against a real second format. The `Events` contract must be extended only if Codex's cumulative `token_count` totals cannot be turned into per-event deltas inside the source.
2. A hooks adapter: translate Cursor and Codex hook payloads into the existing Rust hook inputs, and write each agent's hook registration file during `init`. The first target is the safety guards and session-init, because they matter most and are already tested.
3. Commands and skills: install skills to the per-agent paths, and render commands as skills where the agent has no command concept.

Each slice is its own issue and PR. Slice 1 is the only one this ADR authorizes to start immediately, and slices 2 and 3 each get a short design note before work.

## Out of Scope

- Cursor transcript ingest, until a stable public transcript format is confirmed.
- A statusline for Cursor or Codex.
- A neutral authoring schema with code generation (Alternative C).
- Changing `init` to autodetect the host agent. That belongs to slice 2.
- Windows paths for other agents' config directories.
- Any change to how Claude Code installs or loads the plugin.
- Pricing tables for non-Anthropic models beyond what slice 1 strictly needs to avoid reporting wrong cost.
- Migrating an existing user's data. ADR-0012 already moved the storage.

## Consequences

- **Positive:** the adapter boundary becomes explicit, and the first slice proves it at low risk. Memory and usage data become genuinely shared. Skills are reusable across agents with nearly no work.
- **Negative:** the Codex rollout format is not an OpenAI-documented contract, so the Codex source can break on a Codex upgrade and needs tolerant parsing with a clear skip-and-warn path. Playbook takes on tracking three agents' hook event names.
- **Risk:** a hooks adapter that translates payloads incorrectly could silently disable a safety guard. Slice 2 must therefore fail closed (deny on a payload it cannot parse) and carry golden-payload tests per agent.
- **Follow-up:** open one issue per slice and link them from #301. This ADR leaves the exact Codex delta computation, and the per-agent registration file format, to those slices.
