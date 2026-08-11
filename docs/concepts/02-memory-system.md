# The Memory System

Each new Claude Code session starts cold: no memory of the project, your conventions, or past decisions. The memory system carries durable facts forward so you don't re-teach the same things session after session. It's a typed graph stored in plain markdown files in one local store on your machine, split into global and project scopes.

## The Problem It Solves

Without persistent memory, every session rediscovers what it needs: your coding style, the team's architectural decisions, the quirks in a particular codebase. That works for one-off tasks. For ongoing work across many sessions, the cost compounds: corrections given once need to be given again, decisions get relitigated, patterns re-explained.

Memory breaks that loop. Facts get written once and loaded on demand. The system knows which facts belong to one repo and which apply everywhere.

## Two Scopes

Both scopes use the same file format and the same index structure. The only differences are scope and when the index is loaded.

**Global** at `~/.claude/memory/`: cross-project facts. Your preferences, corrections, and pointers to external resources. These apply in every repo. The index is read on demand, not at session start.

**Project** under `~/.claude/memory/<owner>/<repo>/`: facts true only inside one repo, namespaced by the repo's git remote (`<owner>/<repo>` from `git remote get-url origin`). Each project subfolder has its own `MEMORY.md`, which loads automatically at session start. The whole `~/.claude/memory/` store is git-ignored at the `.claude` level, so these files stay on your machine and never get committed.

The split exists because the two categories are genuinely different. A preference for a coding style applies everywhere. The auth layer's token flow is meaningless outside one service. Namespacing project facts by repo also keeps them from polluting the global root, so the global index stays small enough to load efficiently.

## File Format and Index

Each fact is one file, kebab-case name, with YAML frontmatter:

```yaml
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
```

`type` is one of `user`, `feedback`, `project`, or `reference`. `description` is a one-line trigger hint: "use when..." For `feedback` and `project` type facts, the body follows a fixed structure: the rule first, then a `**Why:**` section and a `**How to apply:**` section. The optional `anchors:` field lists repo-relative code locations the fact describes: a directory, a file, or a `file#symbol`. It maps a fact to concrete code so the graph knows where a fact lives.

The global root and each project subfolder have a `MEMORY.md` index. One line per fact:

```
- [Title](file.md): one-line hook
```

The index is a navigation aid, not the source of truth for edges. See [Internals: Model Routing and Memory](../internals/02-model-routing-and-memory.md) for the full format mechanics.

## Typed Edges

Facts link to each other via `links:` in frontmatter. Values are bare basenames (no path, no extension). Four edge types:

| Edge | Direction | What it means |
|---|---|---|
| `supersedes` | new → old | This fact replaces the target. Act on the chain head; treat superseded facts as historical. |
| `depends_on` | authoring → prerequisite | Load the prerequisite before acting on this fact. |
| `relates_to` | symmetric | Pull the neighbor for related context. |
| `contradicts` | symmetric | Both facts are live but conflict. Surface the conflict; don't silently pick one. |

Edges are typed because each type carries a different action. `supersedes` says "ignore the old one." `depends_on` says "load this first." `relates_to` says "pull in the neighbor." `contradicts` says "surface the conflict." An untyped link would be ambiguous: should the system load the neighbor, replace it, or warn about it? The type resolves the ambiguity.

Each edge is stored once on the authoring fact. Reverse links are inferred at load by scanning frontmatter, not stored explicitly. Traversal depth is 1 for all types except `supersedes`, which the system follows fully to the chain head. Resolution checks the source's own scope first, then falls back to global, so a project fact can link to a global one and still resolve; a project fact shadows a global fact of the same basename. In contradictions between levels, the project fact wins for its repo. A basename missing from both scopes is dangling: it surfaces rather than fails silently.

## How Facts Are Created

Facts get written two ways.

**Ad-hoc during work.** When `/scope`, `/adr`, or `/implement` encounters a durable convention, a decision, a rejected alternative, or an error fix, it writes a fact immediately. `/deep-review` writes findings from a review pass. These are narrow, targeted writes tied to the work at hand.

**Bulk analysis via `/learn-project`.** This command reads the repo broadly (git history, code structure, PRs, and JIRA/Confluence when reachable), clusters what it finds into topics, and writes one fact file per topic. Before writing anything, it shows you a candidate table and asks once. It won't write without your confirmation.

Both paths produce the same file format and land in the right scope of the store.

## How Facts Are Loaded

At session start, `session-init.py` derives the repo's `<owner>/<repo>` from its git remote and reads that project's `MEMORY.md` index at `~/.claude/memory/<owner>/<repo>/` (capped at 16KB), injecting it as context. Fact bodies are not injected upfront; they're read on demand when you or a command needs them. The global index is also read on demand, not at session start. This keeps the initial context lean.

The planning and execution commands (`/scope`, `/adr`, `/implement`) read both scopes before planning or executing. Project facts override global for that repo. Conflicts between the two scopes surface rather than resolve silently. The commit and review commands (`/commit-and-push`, `/quick-review`, `/address-pr-comments`) don't touch memory.

At session end, a `SessionEnd` hook fires a last-chance prompt: if any durable facts from the session haven't been written yet, persist them now. This pairs with the auto-learn nudge on the next session start to close the loop.

## graph.json

A single `graph.json` lives at `~/.claude/memory/graph.json` and covers every fact, global and project. Nodes are facts and referenced code locations. Edges are `links:` between facts, plus `anchors:` pointing facts to code. Each node carries a `scope` (`global` or `project`) and, for project facts, the `project` (`owner/repo`). It's a navigation aid: commands read it for orientation; nothing parses it automatically at session start. The graph is becoming the primary retrieval path rather than just a navigation aid; see [ADR 0004: Graph-first memory retrieval and triggered capture](../adr/0004-graph-first-memory.md) for the decision.

The graph rebuilds automatically. A PostToolUse hook (`rebuild-memory-graph.py`) fires whenever a file under `~/.claude/memory/` is saved, so writing or editing any fact keeps the graph current without a manual step.

## The Auto-Learn Loop

When a session ends after making at least five edits in a repo, the `session-clean-exit.py` hook drops a flag in `~/.claude/runtime/to-learn/`. The next time you open a session in that repo, `session-init.py` reads the flag and surfaces a nudge: consider running `/learn-project` to refresh project memory, or `/learn-project --stage` to queue candidate facts for review.

`--stage` collects candidates into `~/.claude/memory/<owner>/<repo>/staging/` without touching the live store. `--from-staged` reviews them and promotes confirmed facts through the normal write flow.

This loop is nudge-and-approve by design. Memory is durable. A bad fact, a duplicate, or an outdated convention in the store gets loaded and acted on in future sessions. Automatic writes let errors compound unattended. The confirmation gate keeps the human in the loop, every time.

## Design Rationale

A few principles shaped the system.

**Durable over re-learned.** The cost of rediscovery is real. Write once, load on demand.

**Two scopes.** Global preferences and project facts are different things. They live in different parts of the store for good reason.

**Typed edges.** Each relationship between facts carries a specific action. The type tells the system what to do. A bare, untyped link says nothing about the action to take.

**Human-approve.** Memory writes are permanent. The confirmation gate exists to catch duplicates, errors, and outdated facts before they propagate.

**Local-only.** The whole memory store is git-ignored by design. It lives on your machine, and project facts never surface in a repo's commit history.

## See also

- [Decisions and Memory](../guides/03-decisions-and-memory.md)
- [Internals: Model Routing and Memory](../internals/02-model-routing-and-memory.md)
- [The system prompt](01-system-prompt.md)
- [Docs index](../index.md)
