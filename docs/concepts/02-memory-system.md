# The Memory System

Each new Claude Code session starts cold: no memory of the project, your conventions, or past decisions. The memory system carries durable facts forward so you don't re-teach the same things session after session. It's a typed graph stored in plain markdown files in one local store on your machine, split into global, org, and project scopes.

## The Problem It Solves

Without persistent memory, every session rediscovers what it needs: your coding style, the team's architectural decisions, the quirks in a particular codebase. That works for one-off tasks. For ongoing work across many sessions, the cost compounds: corrections given once need to be given again, decisions get relitigated, patterns re-explained.

Memory breaks that loop. Facts get written once and loaded on demand. The system knows which facts belong to one repo, which apply across every repo under one owner, and which apply everywhere.

## Three Scopes

All three scopes use the same file format and the same index structure. The only differences are scope and when the index is loaded.

**Global** at `~/.config/playbook/memory/`: cross-project facts. Your preferences, corrections, and pointers to external resources. These apply in every repo. The index is read on demand, not at session start.

**Org** under `~/.config/playbook/memory/<owner>/`: facts shared across every repo under one owner (e.g. every `acme/*` repo), but not universal. Namespaced by the first segment of the repo's git remote. Each owner subfolder has its own `MEMORY.md` index, sibling to that owner's project subfolders.

**Project** under `~/.config/playbook/memory/<owner>/<repo>/`: facts true only inside one repo, namespaced by the repo's git remote (`<owner>/<repo>` from `git remote get-url origin`). Each project subfolder has its own `MEMORY.md` index. The whole `~/.config/playbook/memory/` store lives outside any repo checkout, so these files stay on your machine and never get committed.

The split exists because the three categories are genuinely different. A preference for a coding style applies everywhere. A team convention applies across every repo your org owns but says nothing about a different org's codebase. The auth layer's token flow is meaningless outside one service. Namespacing org and project facts by owner (and repo) also keeps them from polluting the global root, so the global index stays small enough to load efficiently.

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

Each edge is stored once on the authoring fact. Reverse links are inferred at load by scanning frontmatter, not stored explicitly. Traversal depth is 1 for all types except `supersedes`, which the system follows fully to the chain head. Resolution checks the source's own scope first, then falls back to global: this applies independently to org and project sources, so a project fact does not resolve through its org tier, only through its own project scope or global. A project or org fact can still link to a global target and resolve; a fact shadows a same-basename fact in a less specific scope reachable this way. A basename missing from both scopes tried is dangling: it surfaces rather than fails silently.

## How Facts Are Created

Facts get written two ways.

**Ad-hoc during work.** When `/playbook:scope`, `/playbook:adr`, or `/playbook:implement` encounters a durable convention, a decision, a rejected alternative, or an error fix, it writes a fact immediately. `/playbook:deep-review` writes findings from a review pass. These are narrow, targeted writes tied to the work at hand.

**Bulk analysis via `/playbook:learn-project`.** This command reads the repo broadly (git history, code structure, PRs, and JIRA/Confluence when reachable), clusters what it finds into topics, and writes one fact file per topic. Before writing anything, it shows you a candidate table and asks once. It won't write without your confirmation.

Both paths produce the same file format and land in the right scope of the store.

## How Facts Are Loaded

Facts reach context three ways, described in full in [ADR 0004](../adr/0004-graph-first-memory.md) and [ADR 0008](../adr/0008-bounded-memory-injection-with-prompt-recall-and-handoff-continuity.md).

**At session start**, the `session-init` hook injects a slice of `memory.graph.json`: every global fact plus every fact scoped to the current repo, as names and one-line descriptions, not full bodies. The slice is capped at 16000 characters, so it cannot grow without bound as the store grows. If the graph is unavailable, `session-init` falls back to the legacy `MEMORY.md` index (capped the same way); if neither exists, it injects nothing.

**Editing or writing a file** surfaces the facts anchored to it: the `memory-anchors` hook matches the edited path against the graph's anchor index and injects the matching facts' names, descriptions, and `depends_on`/`contradicts` neighbours.

**Asking about a file or topic**, a prompt rather than an edit, surfaces matching facts' full bodies, not just their descriptions. `memory-anchors` also runs on every prompt, matching prompt text and this session's already-touched files against the same index, deduped so a fact injects at most once per session.

The anchor index above builds once per session and is not rebuilt within it, so a fact written mid-session will not surface via anchor or prompt matching until the next session starts.

The planning and execution commands (`/playbook:scope`, `/playbook:adr`, `/playbook:implement`) read all three scopes before planning or executing. Project facts override org, which overrides global, for that repo. Conflicts between scopes surface rather than resolve silently. The commit and review commands (`/playbook:commit-and-push`, `/playbook:quick-review`, `/playbook:address-pr-comments`) don't touch memory.

**Across a session boundary**, `/playbook:session-handoff` persists a handoff document to disk; `session-init` reloads and deletes it automatically at the next `SessionStart`, including after `/clear`, so `/clear` no longer discards what the session had figured out. The `memory-capture` hook also fires once per context-usage threshold crossing during a session (a `Stop` hook, not a `SessionEnd` one), prompting you to write down durable facts and, since ADR 0008, to run a handoff too, in case the session runs long enough to approach compaction.

## memory.graph.json

A single `memory.graph.json` lives at `~/.config/playbook/memory/memory.graph.json` and covers every fact, global, org, and project. Nodes are facts and referenced code locations. Edges are `links:` between facts, plus `anchors:` pointing facts to code. Each node carries a `scope` (`global`, `org`, or `project`) and, for org facts, the `project` (`owner`), or for project facts, the `project` (`owner/repo`). It is the primary retrieval path, not just a navigation aid: every mechanism in "How Facts Are Loaded" above, the SessionStart slice, the edit-time anchor lookup, and prompt-time recall, reads this file. See [ADR 0004: Graph-first memory retrieval and triggered capture](../adr/0004-graph-first-memory.md) for the decision, and [ADR 0008](../adr/0008-bounded-memory-injection-with-prompt-recall-and-handoff-continuity.md) for what was added on top of it.

The graph rebuilds automatically. The `rebuild-memory-graph` hook fires whenever a file under `~/.config/playbook/memory/` is saved, so writing or editing any fact keeps the graph current without a manual step.

## The Auto-Learn Loop

When a session ends after making at least five edits in a repo, the `session-clean-exit` hook drops a flag in `~/.config/playbook/runtime/to-learn/`. The next time you open a session in that repo, `session-init` reads the flag and surfaces a nudge: consider running `/playbook:learn-project` to refresh project memory, or `/playbook:learn-project --stage` to queue candidate facts for review.

`--stage` collects candidates into `~/.config/playbook/memory/<owner>/<repo>/staging/` without touching the live store. `--from-staged` reviews them and promotes confirmed facts through the normal write flow.

This loop is nudge-and-approve by design. Memory is durable. A bad fact, a duplicate, or an outdated convention in the store gets loaded and acted on in future sessions. Automatic writes let errors compound unattended. The confirmation gate keeps the human in the loop, every time.

## Design Rationale

A few principles shaped the system.

**Durable over re-learned.** The cost of rediscovery is real. Write once, load on demand.

**Three scopes.** Global preferences, org conventions, and project facts are different things. They live in different parts of the store for good reason.

**Typed edges.** Each relationship between facts carries a specific action. The type tells the system what to do. A bare, untyped link says nothing about the action to take.

**Human-approve.** Memory writes are permanent. The confirmation gate exists to catch duplicates, errors, and outdated facts before they propagate.

**Local-only.** The whole memory store is git-ignored by design. It lives on your machine, and project facts never surface in a repo's commit history.

## See also

- [Decisions and Memory](../guides/03-decisions-and-memory.md)
- [Internals: Model Routing and Memory](../internals/02-model-routing-and-memory.md)
- [The system prompt](01-system-prompt.md)
- [Docs index](../index.md)
