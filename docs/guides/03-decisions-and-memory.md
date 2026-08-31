# Decisions and Memory

Two commands handle durable knowledge. `/playbook:adr` records architectural decisions with an optional execution blueprint. `/playbook:learn-project` analyses the repo and stores what it learns as memory facts. Both read from the same memory system, and both write only to git-ignored directories.

## /playbook:adr: Recording a Decision

Use `/playbook:adr` when you're making a real architectural choice and want a durable record: what you chose, why, and why the alternatives lost. Use `/playbook:scope` when a decision's already made and you need an implementation plan only.

The practical split: `/playbook:adr` produces a decision record plus an optional execution blueprint. `/playbook:scope` produces a plan with no decision record attached.

### The flow

1. **Investigate.** Before drafting, the command explores the codebase, reads the memory store (global and project facts), and summarises findings. It waits for your acknowledgement before moving on.
2. **Draft.** Writes the record and (unless you pass `--record-only`) the execution blueprint. Both go under revision until you explicitly approve.
3. **Quality gate.** Three sequential agent phases: fact-check (paths, signatures, dependency graph), adversarial review (simpler alternatives, scope creep, blast radius), and test review (blueprint test plan against engineering standards). A FAIL blocks finalisation.
4. **Finalise.** Status flips from Proposed to Accepted, the quality report saves alongside the record, and the command reports all file paths.

### File layout

Records save to `docs/adr/` and are tracked in git, because a decision record is shared history. Each record follows the naming convention `NNNN-<kebab-title>.md`, with the date in the record's own front matter. The blueprint lands at the same base name with a `-blueprint.md` suffix, and the quality report at `-quality.md`.

```
docs/adr/
  0001-replace-polling-with-websocket.md
  0001-replace-polling-with-websocket-blueprint.md
  0001-replace-polling-with-websocket-quality.md
```

The blueprint is self-contained. Run `/playbook:implement docs/adr/0001-...-blueprint.md` to execute it.

### Flags

- `--record-only`: skips the blueprint. Use this when the implementation plan comes in a follow-up session.
- `--list`: lists existing records, then stops.

### What /playbook:adr writes to memory

During investigation, any durable conventions or gotchas discovered get written as project memory facts. After the gate passes, the decision itself and each rejected alternative (with reasoning) land in memory so future `/playbook:scope` and `/playbook:adr` runs don't re-propose them.

## /playbook:learn-project: Building Project Knowledge

`/playbook:learn-project` reads the repo broadly, distils what it finds into atomic facts, and writes them to memory. It's read-only on the project: the only writes are fact files under `~/.claude/memory/`.

Sources it reads (when available):
- Git history: churn hotspots, commit conventions, contributors.
- Code structure: entry points, build/test/lint tooling, CI/CD, migrations.
- Pull requests (via `gh`): recurring themes, review norms, notable decisions.
- JIRA and Confluence (via MCP or `acli` when reachable).

It collects in parallel, then analyses findings into clusters: architecture, conventions, domain glossary, decisions, infrastructure, setup, scripts, database, and data access patterns. Before writing anything, it shows you a table of candidate facts and asks once. It won't write without your confirmation.

The result: one fact file per topic in `~/.claude/memory/<owner>/<repo>/`. The single `~/.claude/memory/memory.graph.json` rebuilds automatically after each fact save. Run with `--refresh` to re-derive and supersede existing facts. Run with `--graph-only` after hand-editing facts to rebuild the graph without re-collecting.

## The Memory Model

One store, two scopes.

**Global**: flat files directly in `~/.claude/memory/`, indexed by `~/.claude/memory/MEMORY.md`. Facts that apply across all repos, like preferences, cross-project conventions, and external pointers.

**Project**: files under `~/.claude/memory/<owner>/<repo>/`, where `<owner>/<repo>` is derived from `git remote get-url origin`. Facts true only inside that repo. The project index gets injected at session start, making those facts available without manual loading.

The whole `~/.claude/memory/` tree is git-ignored, so no facts are ever committed.

Both scopes use the same format: one fact per file, kebab-case filename, with frontmatter and a structured body.

```markdown
---
name: Auth Flow
description: How tokens are issued and validated in this service
type: project
links:
  depends_on: session-model
  relates_to: rate-limiting
anchors:
  - src/auth/
---

Tokens are issued by the `/auth/token` endpoint and validated via middleware.

**Why:** This is a prerequisite for any changes to the auth layer.

**How to apply:** Read `src/auth/` before modifying any route that requires authentication.
```

### Typed edges

Edges live in the `links:` frontmatter block. Values are bare basenames (no path, no extension).

| Edge | Meaning |
|---|---|
| `supersedes` | This fact replaces an older one. Follow the chain head; treat superseded entries as historical. |
| `depends_on` | Load the prerequisite fact for context before acting on this one. |
| `relates_to` | Symmetric neighbor. Pull it in when the topic is related. |
| `contradicts` | Symmetric conflict. Surface the conflict when both facts are live; don't silently pick one. |

Project facts win over global for that repo. Contradictions between the two scopes surface rather than resolve silently.

### Where facts live

Ask: "Is this fact only useful inside this repo?" Yes goes to the project store. No goes to global. In the project store, don't name the repo in the fact text. It's implicit.

## Worked Example: A Trimmed ADR

A decision record for a small, realistic choice.

---

**ADR-0003: Use kebab-case filenames for memory facts**

- **Status:** Accepted
- **Date created:** 2026-01-15

**Context**

Facts under `~/.claude/memory/` are written by multiple commands and referenced by name from `MEMORY.md` index entries. Three naming styles appeared in early practice: `snake_case`, `camelCase`, and `kebab-case`.

**Decision Drivers**

- Index entries reference basenames directly. A consistent style prevents lookup failures.
- Shell tools (`ls`, `grep`, `find`) handle kebab names without quoting.

**Considered Alternatives**

- `snake_case` (effort: S): familiar from Python tooling, but `_` looks noisy in markdown links. Rejected: kebab wins on ergonomics.
- `camelCase` (effort: S): no case-insensitive filesystem issues in practice, but mixes badly with shell globs. Rejected: glob behavior is unpredictable.
- `kebab-case` (effort: S): lowercase, hyphen-separated, shell-safe, and consistent with how slash commands name their output files. **Chosen.**

**Decision**

All fact filenames use kebab-case. Commands enforce this at write time.

**Consequences**

- Existing facts with other styles need a one-time rename.
- New commands inherit the convention automatically.

---

## See also

- [Plan and Implement](01-plan-and-implement.md): implementing a blueprint or plan.
- [The memory system](../concepts/02-memory-system.md): the rationale and full design behind memory.
- [Internals: Model Routing and Memory](../internals/02-model-routing-and-memory.md): the memory graph internals and edge resolution.
- [Docs index](../index.md).
