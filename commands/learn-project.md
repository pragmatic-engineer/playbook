---
description: Use when starting on a repo the memory store knows nothing about, or to refresh what it knows. Reads git history, PRs, JIRA and Confluence, stores distilled facts in memory, and exports memory.graph.json.
allowed-tools: Bash, Read, Grep, Glob, Write, Agent, Skill, WebFetch
argument-hint: "[--refresh] [--graph-only] [--stage] [--from-staged] [--max-prs N] [--max-commits N] [--auto] [--ask]"
model: opus
effort: high
---

# Learn Project

Build a durable mental model of the repo you're in and persist it as memory facts. Read broadly (code, git history, PRs, and JIRA/Confluence when reachable), distill into topics, classify each fact as repo-specific or cross-project, and write it in the memory format from the system prompt's **Memory** section. Read-only on the project: the only writes are under `~/.config/playbook/memory/` (fact files, plus `~/.config/playbook/memory/memory.graph.json` when the graph rebuilds).

## Argument parsing

Parse `$ARGUMENTS`:

- `--refresh` → re-derive and supersede existing learned facts instead of skipping them.
- `--graph-only` → skip Phases 1-3; rebuild the single `~/.config/playbook/memory/memory.graph.json` from current memory (Phase 4.5), then report. Use after hand-editing facts.
- `--stage` → run collection and analysis (Phases 0-2) but don't write to the live store or ask for confirmation. Write candidate facts to `~/.config/playbook/memory/<owner>/<repo>/staging/` for later review, then stop. See **Staging mode**. Use for unattended or session-end runs.
- `--from-staged` → skip collection; load candidates from `~/.config/playbook/memory/<owner>/<repo>/staging/`, run the normal confirm-and-write flow (Phases 3-4.5), then clear the staging area.
- `--max-prs N` (default 200) and `--max-commits N` (default: all, summarized) → bound scope on large repos.
- `--auto` or `--ask` → set the run mode for this run. Step 0 reads it.
- Anything else → ignore with a one-line warning; don't abort.

## Execution rules

1. Run every bash block for real with the `Bash` tool (capital B, tool names are case-sensitive). Don't simulate.
2. Read files before asserting facts about them (grounding).
3. Combine independent `Bash` calls into a single tool call.
4. Never edit project code or config. Writes are limited to `~/.config/playbook/memory/` files.
5. Dispatch subagents for collection and analysis with the Agent tool, `collector` for Phase 1 and `analyst` for Phase 2: issue the independent Agent calls in a single message so they run in parallel. Subagents produce distilled structured findings, never raw dumps.
6. **Delivery differs per agent tier, per `playbook:delegating-subagents` (invoke it before dispatching).** `collector` holds `Bash`, so it MUST write its findings to a named absolute path under `/tmp/learn-project/<owner>-<repo>/` (`mkdir -p` it first) and return only a one-line count; read those files after each collector finishes, goes idle, or is given up on, because an Agent-tool spawn often completes and returns nothing. `analyst` is structurally read-only and cannot write a file, so its candidate facts come back only by return value, which may not arrive. Either way, an agent that delivered nothing did NOT run: name it as missing rather than proceeding with a partial picture, since a fact written from a half-collected repo is worse than a missing one and much harder to notice later.
7. No silent truncation. If you cap commits/PRs or skip a source, the final report says so.
8. Never persist secrets. Tokens, keys, or credentials seen in configs/CI must never enter a memory fact.

## Step 0: Read the run mode

Do this first, before Phase 0. Read the mode from the CLI:

```bash
playbook mode status --json
```

If the arguments contain `--auto`, add `--flag auto`. If they contain `--ask`, add `--flag ask`. If both are present, stop with one line: "--auto and --ask conflict; pass one." The JSON has four keys: `mode` (`ask` or `auto`), `source` (where it came from), `hook_mode` and `warning`. If `warning` is not empty, print it once. If the command fails, do not stop and do not retry. Run in ask mode and print one line. When the error is `unrecognized subcommand 'mode'`, the installed binary is older than this plugin: print "playbook binary is older than the plugin, run `playbook update`."

**Effort ceiling.** Also run `playbook effort resolve commands learn-project --json`. Its `ceiling` is the highest effort the user allows for this run (`null` means no limit). Hold your own work to it, and before you spawn an agent follow the `delegating-subagents` skill, which runs `playbook effort resolve agents <agent>` and uses the `subagentType` it returns. If the command fails, carry on with no ceiling.

**Routing.** Where you choose what to spawn, also run `playbook route mechanical --json` and follow its `action` under the `routing.escalate` gate: on `ask` stop and ask the user, on `downgraded` (`deny`) use the lower tier it names, and in auto mode (`auto`) proceed and log the assumption. If the command fails, carry on.

- **`ask` mode:** behave exactly as this file describes.
- **`auto` mode:** follow the Auto path below.

### Auto path

In auto mode behave as `--stage`: collect and analyse, then write the candidates to the staging area and stop. The Phase 3 write question ("Write these to memory?") is skipped in that path, and nothing reaches the live store. Where Phase 0 would ask which JIRA key or Confluence space to use, take the most likely one and name it in the report. If `--from-staged` was also passed, stop with one line: "Auto mode does not promote staged facts; run /playbook:learn-project --from-staged --ask."

## Phase 0: Preflight and scope

```bash
playbook learn preflight
```

It prints `Repo:`, `Root:`, `Store:` and `Commits:` lines, then `gh:` and `acli:` probe lines (with `acli jira:` and `acli confluence:` auth lines when `acli` is present), then the JIRA project keys found in the last 500 commits as a count histogram. It exits 1 with `error: not in a git repo` outside a repository.

Then, before collecting:

- **Atlassian access:** if `mcp__atlassian__*` tools are available in this session, use them. Else if `acli` is present and authenticated, **use it, and load the `playbook:atlassian-cli` skill first**: it carries the real command surface and the Confluence page-discovery workaround. Jira and Confluence authenticate separately, so treat the two probes above independently: Jira reachable and Confluence not is a normal state, not an error. Else mark JIRA/Confluence **unavailable** and record it for the report.
- **Targets:** resolve the JIRA project key(s) from the histogram and the Confluence space from README/links. If ambiguous, ask the user once.
- **Capture:** `REPO`, `ROOT`, scope caps, and which sources are reachable. You need these in every later phase.

## Phase 1: Collect (parallel subagents)

On `--refresh` only, before dispatching, run `playbook memory context --repo $REPO`: its facts block (`name: description` lines), not the full fact bodies. If the command produces no output (empty store, or the `playbook` binary unavailable), fall back to reading `~/.config/playbook/memory/memory.graph.json` directly with the `Read` tool and picking out nodes whose `scope` is `global`, or whose `project` matches `$REPO` (or its owner, for `org` scope). Include whichever result you got in each collector's prompt so it can flag what's already documented instead of silently re-discovering it, and note anything that looks stale against what it finds. Note in the report which source produced it: command output, direct graph read, or nothing found. Skip this on a fresh (non-`--refresh`) run: there's rarely anything in the store yet, and this command's job is building it, not consuming it.

Dispatch these collectors in parallel with `subagent_type: playbook:collector`. `collector` pins Haiku, the cost win this phase is built for. Each returns a compact structured summary (tight JSON or markdown) that cites paths/refs, NOT raw command output. Spawn each collector with a stable `name`; the moment it returns its result, call `TaskStop` on it. A spawned agent stays idle-alive for `SendMessage` follow-ups and this flow never reuses a finished collector, so leaving it unstopped keeps it running in the background.

- **git-history**: contributors and ownership, churn hotspots (`git log --format= --name-only | sort | uniq -c | sort -rn`), commit-message and branch conventions, tags/releases, cadence.
- **code-structure**: top-level tree, entry points, languages, build/test/lint tooling, Dockerfiles / CI-CD configs, IaC, migration dirs and ORM models, `scripts/` and Makefile targets.
- **pull-requests** (if `gh` ok): `gh pr list --state all --limit <MAX_PRS> --json number,title,labels,body,author`: recurring themes, review norms, linked JIRA keys, notable decisions.
- **jira** (if reachable): epics, active sprints/boards, components, common labels for the project key(s).
- **confluence** (if reachable): pages on setup/onboarding, architecture, runbooks, and decisions in the project space; capture titles, URLs, and key points.

## Phase 2: Analyze into topics (parallel subagents)

Feed the Phase 1 findings to one analyst per cluster, spawned with `subagent_type: playbook:analyst`. Spawn each analyst with a stable `name` and `TaskStop` it as soon as it returns. A finished agent stays idle-alive for `SendMessage` follow-ups; this flow never reuses one, so stopping it immediately prevents lingering background processes. Each emits **candidate facts**, where each fact has: `title`, `body` (the fact, then Why, then How to apply), proposed `type` (`project` for repo knowledge, `reference` for external pointers), `scope` (`repo` | `global`), proposed `links` edges, and `anchors` (repo-relative code locations the fact describes: dirs, files, or `file#symbol`).

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
- **Dedupe:** run `playbook memory context --repo $REPO` once: its facts block already covers both project-scoped and global-scoped facts for this repo (global-scoped facts are always included), so one call replaces both index reads. Load the relevant fact files it names. If the command produces nothing, fall back to reading `~/.config/playbook/memory/memory.graph.json` directly the same way as Phase 1's priming; this fallback is not optional polish here, since a broken dedupe read risks writing a duplicate fact. Note which source produced the result. If a fact already exists: skip it, unless `--refresh`, in which case update the file or write a successor carrying a `supersedes` edge. Never blind-duplicate.
- **Plan:** show the user a concise table of candidate facts (title · scope · type · new/update/supersede). Ask once: "Write these to memory?" Proceed only on yes; honor a subset selection.

## Phase 4: Write memory

Project store, first time in this repo only:

```bash
mkdir -p "$STORE"
```

Then write each approved fact:

- One fact per file, kebab-case name, in the chosen store (`$STORE/` or `~/.config/playbook/memory/`).
- Frontmatter: `name`, `description` (one-line when-to-use), `type`, `links:` with bare-basename edges (`supersedes`, `depends_on`, `relates_to`, `contradicts`), and `anchors:` listing the repo-relative code locations the fact describes (`src/auth/`, `src/auth/login.py`, or `src/auth/login.py#authenticate`).
- Body: the fact, then **Why:** and **How to apply:**. Use absolute dates for anything time-bound (`date +%F`).
- In the project store, do NOT name the repo in the fact text; it's implicit.
- Write a `project-overview` fact as the entry point, linked via `relates_to` to the main topic facts.

## Phase 4.5: Rebuild the navigation graph

`~/.config/playbook/memory/memory.graph.json` is a single graph covering every fact, global, org, and project. It rebuilds automatically: the `rebuild-memory-graph` PostToolUse hook fires whenever a fact file under `~/.config/playbook/memory/` is saved, so once Phase 4 has written the facts the graph is already current. Normally you skip this phase.

`--graph-only` forces a rebuild without re-collecting, for use after hand-editing fact files:

```bash
playbook memory rebuild
```

**Why a dedicated subcommand rather than invoking the hook.** The hook skips unless the write it was told about is under `~/.config/playbook/memory/`, which is right for a PostToolUse hook and leaves no way to force a full rebuild. This used to run `python3 hooks/rebuild-memory-graph.py < /dev/null`, because that script treated empty stdin as "rebuild everything". The Rust port dropped that branch deliberately (see `should_skip` in `src/hooks/rebuild_memory_graph.rs`), judging it unexercised by the hook's test suite. It was exercised, by this command. Faking a `tool_input` payload that names a path inside the memory dir would also work and is what the port's own doc calls the more fragile option, since it breaks silently the next time the skip logic changes.

It walks every fact under `~/.config/playbook/memory/`, derives each fact's scope (`global`, or `project` with its `owner/repo`), and writes `~/.config/playbook/memory/memory.graph.json` atomically. Nodes are facts plus their `anchors:` code locations; edges are the `links:` between facts and the fact→code anchors. Report the node and edge counts, and flag any dangling edge.

## Staging mode (`--stage` and `--from-staged`)

These split collection from the write decision, so a run can happen unattended (for example, nudged at session end) and the human approves later. The staging area is `$STORE/staging/`, inside the central memory store.

**`--stage`** (collect now, decide later):

1. Run Phases 0-2 as normal to produce candidate facts.
2. Skip Phase 3's confirmation and Phase 4's live writes. Create `$STORE/staging/`, then write each candidate to `$STORE/staging/<kebab>.md` in the normal fact format, plus two extra frontmatter fields: `status: pending` and `staged: <date +%F>`, a `scope:` (`repo` | `global`), and, when it would update an existing fact, a `supersedes:` note.
3. Write or refresh `$STORE/staging/STAGED.md` with one `- [Title](file.md): one-line hook` line per candidate.
4. Do NOT touch the live `memory.graph.json`.
5. Report the count staged, the staging path, and: "Review with `/playbook:learn-project --from-staged`."

**`--from-staged`** (review and promote):

1. Skip Phases 0-2. Read every candidate in `$STORE/staging/`.
2. Run Phase 3 against them: show the candidate table, dedupe against the live stores, and ask once "Write these to memory?" (honor a subset).
3. For approved candidates, run Phase 4 (write to the live store, dropping the `status`/`staged` staging fields; apply `supersedes`/updates) and Phase 4.5 (rebuild `memory.graph.json`).
4. Remove promoted candidates from staging. Leave any the user skipped; delete any the user rejects.
5. Report as in Phase 5.

## Phase 5: Report

One tight summary:

- Facts written / updated / superseded, per cluster and per store.
- Sources used, and **sources skipped with the reason** (e.g. "Confluence: no MCP and acli absent").
- The `memory.graph.json` path, node and edge counts, and any dangling anchors or edges flagged during the build.

## Teardown (MUST run, even on failure or abort)

`TaskStop` every subagent spawned in this flow that is still alive. Confirm via `TaskList` that none from this run remain before finishing.

## Anti-patterns to refuse

1. Dumping raw `git log` / PR / JIRA output into memory. Facts are distilled, atomic, and actionable.
2. Silent skips. An unreachable source or applied cap must appear in the report.
3. Duplicating an existing fact instead of superseding or updating it.
4. Writing repo-specific detail into the global store, or cross-project facts into the project store.
5. Editing project code or config. Memory files under `~/.config/playbook/memory/` are the only writes.
6. Persisting secrets or tokens pulled from configs or CI.
7. Leaving `memory.graph.json` stale or non-deterministic. Rebuild it whenever facts change, and sort nodes/edges so reruns produce clean diffs.
