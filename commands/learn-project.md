---
description: Deeply learn the current project (git history, PRs, JIRA, Confluence), store distilled topics in the memory system (routed per-project vs global), and export a navigable graph.json of the memory graph.
allowed-tools: Bash, Read, Grep, Glob, Write, Agent, WebFetch
argument-hint: "[--refresh] [--graph-only] [--stage] [--from-staged] [--max-prs N] [--max-commits N]"
model: opus
effort: high
---

# Learn Project

Build a durable mental model of the repo you're in and persist it as memory facts. Read broadly (code, git history, PRs, and JIRA/Confluence when reachable), distill into topics, classify each fact as repo-specific or cross-project, and write it in the memory format from the system prompt's **Memory** section. Read-only on the project: the only writes are under `~/.claude/memory/` (fact files, plus `~/.claude/memory/graph.json` when the graph rebuilds).

## Argument parsing

Parse `$ARGUMENTS`:

- `--refresh` → re-derive and supersede existing learned facts instead of skipping them.
- `--graph-only` → skip Phases 1-3; rebuild the single `~/.claude/memory/graph.json` from current memory (Phase 4.5), then report. Use after hand-editing facts.
- `--stage` → run collection and analysis (Phases 0-2) but don't write to the live store or ask for confirmation. Write candidate facts to `~/.claude/memory/<owner>/<repo>/staging/` for later review, then stop. See **Staging mode**. Use for unattended or session-end runs.
- `--from-staged` → skip collection; load candidates from `~/.claude/memory/<owner>/<repo>/staging/`, run the normal confirm-and-write flow (Phases 3-4.5), then clear the staging area.
- `--max-prs N` (default 200) and `--max-commits N` (default: all, summarized) → bound scope on large repos.
- Anything else → ignore with a one-line warning; don't abort.

## Execution rules

1. Run every bash block for real. Don't simulate.
2. Read files before asserting facts about them (grounding).
3. Combine independent bash calls into a single tool call.
4. Never edit project code or config. Writes are limited to `~/.claude/memory/` files.
5. Dispatch subagents for collection and analysis with the Agent tool, `collector` for Phase 1 and `analyst` for Phase 2: issue the independent Agent calls in a single message so they run in parallel. Subagents produce distilled structured findings, never raw dumps.
6. **Delivery differs per agent tier, per `playbook:delegating-subagents` (invoke it before dispatching).** `collector` holds `Bash`, so it MUST write its findings to a named absolute path under `/tmp/learn-project/<owner>-<repo>/` (`mkdir -p` it first) and return only a one-line count; read those files after each collector finishes, goes idle, or is given up on, because an Agent-tool spawn often completes and returns nothing. `analyst` is structurally read-only and cannot write a file, so its candidate facts come back only by return value, which may not arrive. Either way, an agent that delivered nothing did NOT run: name it as missing rather than proceeding with a partial picture, since a fact written from a half-collected repo is worse than a missing one and much harder to notice later.
7. No silent truncation. If you cap commits/PRs or skip a source, the final report says so.
8. Never persist secrets. Tokens, keys, or credentials seen in configs/CI must never enter a memory fact.

## Phase 0: Preflight and scope

```bash
set -e
ROOT=$(git rev-parse --show-toplevel 2>/dev/null) || { echo "error: not in a git repo" >&2; exit 1; }
cd "$ROOT"
REPO=$(gh repo view --json nameWithOwner -q .nameWithOwner 2>/dev/null) || { u=$(git remote get-url origin 2>/dev/null); u=${u%.git}; REPO="$(basename "$(dirname "$u")")/$(basename "$u")"; }
STORE="$HOME/.claude/memory/$REPO"
COMMITS=$(git rev-list --count HEAD 2>/dev/null || echo 0)
echo "Repo:    $REPO"
echo "Root:    $ROOT"
echo "Store:   $STORE"
echo "Commits: $COMMITS"

# capability probes
command -v gh   >/dev/null && gh auth status >/dev/null 2>&1 && echo "gh:   ok" || echo "gh:   UNAVAILABLE (PRs skipped)"
if command -v acli >/dev/null; then
  echo "acli: present ($(acli --version 2>/dev/null | head -1))"
  acli jira auth status       >/dev/null 2>&1 && echo "acli jira:       authed" || echo "acli jira:       NOT authed"
  acli confluence auth status >/dev/null 2>&1 && echo "acli confluence: authed" || echo "acli confluence: NOT authed"
else
  echo "acli: absent"
fi

# JIRA project keys referenced in history (histogram)
echo "JIRA keys in history:"
git log --oneline -500 2>/dev/null | grep -oE '[A-Z][A-Z0-9]+-[0-9]+' | sed -E 's/-[0-9]+$//' | sort | uniq -c | sort -rn | head || true
```

Then, before collecting:

- **Atlassian access:** if `mcp__atlassian__*` tools are available in this session, use them. Else if `acli` is present and authenticated, **use it, and load the `playbook:atlassian-cli` skill first**: it carries the real command surface and the Confluence page-discovery workaround. Jira and Confluence authenticate separately, so treat the two probes above independently: Jira reachable and Confluence not is a normal state, not an error. Else mark JIRA/Confluence **unavailable** and record it for the report.
- **Targets:** resolve the JIRA project key(s) from the histogram and the Confluence space from README/links. If ambiguous, ask the user once.
- **Capture:** `REPO`, `ROOT`, scope caps, and which sources are reachable. You need these in every later phase.

## Phase 1: Collect (parallel subagents)

Dispatch these collectors in parallel with `subagent_type: collector`. `collector` pins Haiku, the cost win this phase is built for. Each returns a compact structured summary (tight JSON or markdown) that cites paths/refs, NOT raw command output. Spawn each collector with a stable `name`; the moment it returns its result, call `TaskStop` on it. A spawned agent stays idle-alive for `SendMessage` follow-ups and this flow never reuses a finished collector, so leaving it unstopped keeps it running in the background.

- **git-history**: contributors and ownership, churn hotspots (`git log --format= --name-only | sort | uniq -c | sort -rn`), commit-message and branch conventions, tags/releases, cadence.
- **code-structure**: top-level tree, entry points, languages, build/test/lint tooling, Dockerfiles / CI-CD configs, IaC, migration dirs and ORM models, `scripts/` and Makefile targets.
- **pull-requests** (if `gh` ok): `gh pr list --state all --limit <MAX_PRS> --json number,title,labels,body,author`: recurring themes, review norms, linked JIRA keys, notable decisions.
- **jira** (if reachable): epics, active sprints/boards, components, common labels for the project key(s).
- **confluence** (if reachable): pages on setup/onboarding, architecture, runbooks, and decisions in the project space; capture titles, URLs, and key points.

## Phase 2: Analyze into topics (parallel subagents)

Feed the Phase 1 findings to one analyst per cluster, spawned with `subagent_type: analyst`. Spawn each analyst with a stable `name` and `TaskStop` it as soon as it returns. A finished agent stays idle-alive for `SendMessage` follow-ups; this flow never reuses one, so stopping it immediately prevents lingering background processes. Each emits **candidate facts**, where each fact has: `title`, `body` (the fact, then Why, then How to apply), proposed `type` (`project` for repo knowledge, `reference` for external pointers), `scope` (`repo` | `global`), proposed `links` edges, and `anchors` (repo-relative code locations the fact describes: dirs, files, or `file#symbol`).

Clusters:

- architecture & module map
- conventions & patterns (design patterns adopted, build/test/lint, branching, commit/PR)
- domain glossary
- decisions & active work (ADRs from PRs/commits + JIRA epics)
- infrastructure (CI/CD, deploy, cloud, IaC)
- setup (local dev and onboarding)
- scripts & tooling
- database schemas & models
- data access patterns

Keep facts atomic: one concept per fact. Drop low-signal or self-evident facts.

## Phase 3: Classify, dedupe, plan

- **Scope routing:** default `repo`. Mark `global` only when the fact is org/account-wide and not tied to this repo (company tooling, the Atlassian instance, standards seen across repos). A repo fact that contradicts a global one wins for this repo; note it with a `contradicts` edge.
- **Dedupe:** read the existing indexes (`$STORE/MEMORY.md` and `~/.claude/memory/MEMORY.md`) and the relevant fact files. If a fact already exists: skip it, unless `--refresh`, in which case update the file or write a successor carrying a `supersedes` edge. Never blind-duplicate.
- **Plan:** show the user a concise table of candidate facts (title · scope · type · new/update/supersede). Ask once: "Write these to memory?" Proceed only on yes; honor a subset selection.

## Phase 4: Write memory

Project store, first time in this repo only:

```bash
mkdir -p "$STORE"
```

Then write each approved fact:

- One fact per file, kebab-case name, in the chosen store (`$STORE/` or `~/.claude/memory/`).
- Frontmatter: `name`, `description` (one-line when-to-use), `type`, `links:` with bare-basename edges (`supersedes`, `depends_on`, `relates_to`, `contradicts`), and `anchors:` listing the repo-relative code locations the fact describes (`src/auth/`, `src/auth/login.py`, or `src/auth/login.py#authenticate`).
- Body: the fact, then **Why:** and **How to apply:**. Use absolute dates for anything time-bound (`date +%F`).
- In the project store, do NOT name the repo in the fact text; it's implicit.
- Add or refresh the `- [Title](file.md): one-line hook` line in the right `MEMORY.md`. Mark superseded index entries `(superseded)`.
- Write a `project-overview` fact as the entry point, linked via `relates_to` to the main topic facts.

## Phase 4.5: Rebuild the navigation graph

`~/.claude/memory/graph.json` is a single graph covering every fact, global and project. It rebuilds automatically: the `rebuild-memory-graph.py` PostToolUse hook fires whenever a fact file under `~/.claude/memory/` is saved, so once Phase 4 has written the facts the graph is already current. Normally you skip this phase.

`--graph-only` forces a rebuild without re-collecting, for use after hand-editing fact files:

```bash
playbook memory rebuild
```

**Why a dedicated subcommand rather than invoking the hook.** The hook skips unless the write it was told about is under `~/.claude/memory/`, which is right for a PostToolUse hook and leaves no way to force a full rebuild. This used to run `python3 hooks/rebuild-memory-graph.py < /dev/null`, because that script treated empty stdin as "rebuild everything". The Rust port dropped that branch deliberately (see `should_skip` in `src/hooks/rebuild_memory_graph.rs`), judging it unexercised by the hook's test suite. It was exercised, by this command. Faking a `tool_input` payload that names a path inside the memory dir would also work and is what the port's own doc calls the more fragile option, since it breaks silently the next time the skip logic changes.

It walks every fact under `~/.claude/memory/`, derives each fact's scope (`global`, or `project` with its `owner/repo`), and writes `~/.claude/memory/graph.json` atomically. Nodes are facts plus their `anchors:` code locations; edges are the `links:` between facts and the fact→code anchors. Report the node and edge counts, and flag any dangling edge.

## Staging mode (`--stage` and `--from-staged`)

These split collection from the write decision, so a run can happen unattended (for example, nudged at session end) and the human approves later. The staging area is `$STORE/staging/`, inside the central memory store.

**`--stage`** (collect now, decide later):

1. Run Phases 0-2 as normal to produce candidate facts.
2. Skip Phase 3's confirmation and Phase 4's live writes. Create `$STORE/staging/`, then write each candidate to `$STORE/staging/<kebab>.md` in the normal fact format, plus two extra frontmatter fields: `status: pending` and `staged: <date +%F>`, a `scope:` (`repo` | `global`), and, when it would update an existing fact, a `supersedes:` note.
3. Write or refresh `$STORE/staging/STAGED.md` with one `- [Title](file.md): one-line hook` line per candidate.
4. Do NOT touch the live `MEMORY.md` or `graph.json`.
5. Report the count staged, the staging path, and: "Review with `/playbook:learn-project --from-staged`."

**`--from-staged`** (review and promote):

1. Skip Phases 0-2. Read every candidate in `$STORE/staging/`.
2. Run Phase 3 against them: show the candidate table, dedupe against the live stores, and ask once "Write these to memory?" (honor a subset).
3. For approved candidates, run Phase 4 (write to the live store, dropping the `status`/`staged` staging fields; apply `supersedes`/updates; refresh `MEMORY.md`) and Phase 4.5 (rebuild `graph.json`).
4. Remove promoted candidates from staging. Leave any the user skipped; delete any the user rejects.
5. Report as in Phase 5.

## Phase 5: Report

One tight summary:

- Facts written / updated / superseded, per cluster and per store.
- Sources used, and **sources skipped with the reason** (e.g. "Confluence: no MCP and acli absent").
- The path to each store's `MEMORY.md`.
- The `graph.json` path, node and edge counts, and any dangling anchors or edges flagged during the build.

## Teardown (MUST run, even on failure or abort)

`TaskStop` every subagent spawned in this flow that is still alive. Confirm via `TaskList` that none from this run remain before finishing.

## Anti-patterns to refuse

1. Dumping raw `git log` / PR / JIRA output into memory. Facts are distilled, atomic, and actionable.
2. Silent skips. An unreachable source or applied cap must appear in the report.
3. Duplicating an existing fact instead of superseding or updating it.
4. Writing repo-specific detail into the global store, or cross-project facts into the project store.
5. Editing project code or config. Memory files under `~/.claude/memory/` are the only writes.
6. Persisting secrets or tokens pulled from configs or CI.
7. Leaving `graph.json` stale or non-deterministic. Rebuild it whenever facts change, and sort nodes/edges so reruns produce clean diffs.
