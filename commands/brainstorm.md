---
description: Divergent discovery session that explores a raw idea, weighs approaches, and produces an approved PRD plus a design doc that hands off to /playbook:scope.
allowed-tools: Agent, Read, Bash, Grep, Glob, Skill, Write, Edit, WebFetch
argument-hint: "[idea | PROJ-123 | ./prompt.md] [--ticket <id>] [--depth 0-2] [--adr] [--no-chain] [--help]"
model: opus
effort: high
---

# Brainstorm: Divergent Discovery

Turn a raw idea into an approved PRD and design doc. Explore the problem, challenge the premise, weigh 2-3 approaches, and capture the "why" before any planning starts. This is the divergent counterpart to `/playbook:scope`: `/playbook:scope` converges a settled direction into a plan, `/playbook:brainstorm` finds the direction first.

Invoked as `/playbook:brainstorm`. The remaining arguments are an optional idea seed, ticket id, or file path.

The terminal state is a PRD and a design doc plus an offer to run `/playbook:scope`. Do NOT write code, scaffold anything, or produce an implementation plan here (the one narrow exception is Core Rule 1's validation spike). That's `/playbook:scope` and `/playbook:implement`.

## Help

If the arguments contain `--help`, print this and stop:

```
/playbook:brainstorm - Divergent discovery that produces a PRD and a design doc

USAGE:
  /playbook:brainstorm [idea]              Start an interactive discovery session
  /playbook:brainstorm "offline mode"      Start with an idea seed
  /playbook:brainstorm PROJ-123            Pull a ticket and discover from it
  /playbook:brainstorm ./notes.md          Load the idea seed from a file

OPTIONS:
  --ticket <id>  Force ticket mode for <id> (skip seed/file detection).
  --depth <0-2>  How far to crawl ticket links: 0 ticket only, 1 direct
                 links (default), 2 one more hop. Always bounded.
  --adr        Route to /playbook:adr at the end instead of /playbook:scope (the direction
               carries a weighty architectural decision worth a formal record).
  --no-chain   Write the design doc and stop. Skip the /clear-and-continue suggestion too.
  --help       Show this help

Asks one question at a time with a recommended answer. Given a ticket id, pulls the
ticket (description, comments, attachments, linked items) via a connected MCP or a
configured provider command, then explores the codebase in parallel before asking
you. Checks memory and past design docs for a similar idea already rejected. Confirms
a PRD before proposing 2-3 approaches, then captures both in
.claude/designs/<date>-<slug>-prd.md and .claude/designs/<date>-<slug>.md,
then offers to chain into /playbook:scope (which reads the docs and skips what they already settled).
```

## Core Rules (MUST)

1. **Do NOT write code or an implementation plan.** The output is a design doc. Detailed file lists, Work Units, and test strategy belong to `/playbook:scope`. The one narrow exception is Step 5.5's optional validation spike: throwaway code to check a single uncertain premise, never part of the design doc's content, never the start of the real implementation.
2. **Ask ONE question at a time.** One question, a recommended answer, wait, then the next. The only exception is the first message, where you present context and the first question together.
3. **Explore before asking.** If the codebase settles a question, resolve it yourself and report what you found. Only ask about intent, constraints, and preferences the code can't answer.
4. **Challenge the premise.** Don't accept the framing at face value. Ask whether this is the right problem, whether a simpler direction meets the goal, and what "done" actually looks like.
5. **Present a design and get approval before writing the doc.** Hard gate, every time, even for a small idea. The design can be a few sentences, but you MUST present it and get a yes.

## Argument Resolution

Resolve the argument in this order:

1. **Ticket:** if `--ticket <id>` is set, or the argument matches a ticket key (`[A-Z][A-Z0-9]+-\d+`, e.g. `PROJ-123`) or a known tracker URL (Jira, Linear, GitHub issue), treat it as a ticket and go to Step 1.5 to pull it. `--ticket` forces ticket mode even for an ambiguous value.
2. **File path:** if it starts with `./`, `../`, `/`, or `~`, or ends with `.md`, `.txt`, `.yaml`, `.yml`, check whether it exists with the Read tool. If it exists, read it and use it as the idea seed; if not, treat it as a plain-text seed.
3. **Plain text:** otherwise the argument is the idea seed.
4. **No argument:** ask what we're exploring before anything else.

Strip `--ticket <id>`, `--depth <n>`, `--adr`, and `--no-chain` (like `--help`) before resolving the seed. Don't read `.gitignore`d files even if the seed or ticket mentions them.

## How It Works

### Step 0: Load skills

Load `playbook:writing-style` (voice, banned words, no dashes; the design doc and every question follow it) and `playbook:grounding-research` (cite `file:line`, tag `[unverified]` when you can't confirm; governs the context digest and any self-answering).

### Step 1: Frame the idea and check scope

Restate the idea in one or two sentences so we agree on what we're exploring.

**Scope check (MUST).** If the idea is several independent subsystems (e.g. "a platform with chat, billing, and analytics"), stop and flag it. Help decompose into independent pieces, name how they relate and what order to build them, then brainstorm the first piece through the normal flow. Don't design a tangle.

### Step 1.5: Pull the ticket (ticket mode only)

Run this only when Argument Resolution found a ticket. Skip it entirely for a plain idea or file seed.

**Connect (layered).** Find a way to reach the tracker, in order:

1. **MCP:** search for a connected ticket tool (`ToolSearch` for Jira, Linear, Atlassian, or issue tools). If one is connected, use it.
2. **Provider command:** else look for a configured fetch command. Read `.claude/brainstorm.config` (or the repo's existing tracker config) for a per-tracker command with an `{id}` placeholder, for example `jira issue view {id} --raw` or `linear issue {id} --json`, and run it with Bash. A public tracker URL with no auth can be read with `WebFetch`. A page that needs auth or JavaScript rendering that `WebFetch` can't handle can be opened with the `agent-browser` MCP, if it's connected: `open` the url, then `snapshot` for the accessibility tree and `screenshot` for visual content.
3. **Neither:** stop and tell the user how to connect one (an MCP server or a provider command), then offer to continue with the ticket id as a plain-text seed. Do NOT fabricate ticket contents.

**Crawl (bounded, never infinite).** Gather, then stop:

- The ticket itself: title, description, status, and comments.
- Attachments: read images visually and PDFs or docs as pages. For an attachment the tracker exposes only as a web link, open it with the `agent-browser` MCP (if connected) and `snapshot` or `screenshot` it. Note and skip binaries and anything the tracker doesn't expose.
- One hop of direct links: linked issues, sub-tasks, parent epic, and linked PRs. `--depth` controls this (0 = ticket only, 1 = direct links (default), 2 = one more hop). Clamp `--depth` to the range 0 to 2, so the crawl is never unbounded.
- Bounds: cap total related items at about 15, dedup visited tickets by id, and stop early when a hop adds nothing new.

**Discover in parallel.** Fan out discovery agents over the gathered sources (issue the Agent calls in one message so they run at once, per Step 2): the ticket body plus comments, batches of linked items, and the attachments. Each returns a short cited summary (the source id or url, and the facts that bear on the work). These feed the Step 2 digest alongside the codebase exploration. Assign each discovery agent a stable `name` at spawn and `TaskStop` it as soon as it returns. A spawned agent stays idle-alive for `SendMessage` follow-ups and this flow never reuses a finished one, so leaving it unstopped keeps a subagent running in the background.

The ticket's title and description become the idea seed for Step 1's framing. Record the ticket id and link so Step 7 can note them in the design doc.

### Step 2: Explore context in parallel

Fan out `Explore` agents to map what the dialogue needs. **Dispatch them in parallel: issue all the Agent calls in a single message so they run at once.** Read-only exploration has no shared state, so parallel is always the default here.

Scale the fan-out to the idea: one agent for a tiny change, up to about four for a broad feature. Give each a distinct area, for example:

- Existing patterns and prior art for this kind of change.
- Integration points and the consumers a change would touch.
- Constraints: config, conventions, and anything in the code that limits the options.

Alongside the `Explore` agents, dispatch one independent `critic` agent (`subagent_type: critic`, focus `premise`), prompted to challenge the premise rather than explore code. Its return feeds the Step 2 digest and the Step 4 approach exploration, so premise-challenge isn't only in the orchestrator's head. Close it on return with the others (Step 2 teardown).

Built-in `Explore` agents have been reliable at returning results; the `critic` is structurally read-only and has only the return channel, so it may deliver nothing (`playbook:delegating-subagents`). An area whose agent returned nothing was NOT explored: it does not mean there is nothing there. Say which areas are unexplored rather than treating the digest as complete, and if the premise-challenge came back empty, challenge the premise yourself before moving to Step 3.

**Check memory and prior designs.** Alongside the `Explore` agents, check whether a memory store exists: the global store at `~/.claude/memory/MEMORY.md` and the project store at `~/.claude/memory/<owner>/<repo>/MEMORY.md` (`<owner>/<repo>` from `git remote get-url origin`). Load the relevant fact files from whichever exist. Also scan `.claude/designs/*.md` for a prior design doc whose title or topic overlaps this idea (a cheap keyword match, not semantic search). When neither has anything relevant, skip this silently. When either surfaces a plausible match, a decision already made or an idea already rejected, say so in the digest: what was decided, when, and why. Ask directly whether anything has changed before diverging into new approaches, rather than re-litigating a settled call from scratch.

Consolidate into a short cited digest (a few bullets, each with `file:line`). This grounds the questions that follow so you ask about intent, not about facts the code already holds. In ticket mode, fold the Step 1.5 ticket findings into the same digest, citing the source id or url for those. Assign each `Explore` agent a stable `name` at spawn and `TaskStop` it as soon as it returns. A spawned agent stays idle-alive for `SendMessage` follow-ups and this flow never reuses a finished one, so leaving it unstopped keeps a subagent running in the background.

**Verify the load-bearing premises before diverging.** From the digest, list the load-bearing citations: the premises the design will rest on (for example "the code already does X", "there is no existing helper for Y"). Re-read each cited `file:line`. Drop or tag `[unverified]` any that don't hold, and tag each surviving context bullet HIGH / MEDIUM / LOW (the `playbook:grounding-review` skill defines the levels). Spot-check the load-bearing claims only; don't audit every citation, or the divergent phase drags. Dropped or LOW-confidence premises become open items in the Step 7 handoff trailer.

### Step 3: Interactive discovery

Ask questions one at a time, each with a recommended answer and reasoning, each following from the last. Cover:

- **Purpose:** why this, why now? What breaks or stays broken without it?
- **Success criteria:** what does "done" look like, observably?
- **Constraints:** technical, product, or time limits that rule options in or out.
- **Non-goals:** what this explicitly won't do.

Between questions, explore further if an answer opens a new area, and report what you found before the next question.

Scale the depth: 2-4 questions for a small idea, more for a broad one. Don't over-interview a simple thing.

**Domain glossary (when a term is genuinely ambiguous or new).** If the conversation turns on a term that's overloaded, vague, or new to this codebase, don't just use it and move on: propose a precise definition and check it with the user. This isn't for every noun in a small idea, only for a term the design actually hinges on. Write it to `GLOSSARY.md` at the target repo's root (create the file only on its first real entry; it's tracked in git, not ignored, since its value is shared vocabulary across future sessions, not scratch). Each entry states what the term IS in one or two sentences, not what it does, plus a short list of synonyms to avoid so the disambiguation is recorded, not just implied. Write it the moment it resolves, don't batch it for later. If `GLOSSARY.md` already has a conflicting entry for the term, surface the conflict to the user instead of overwriting it silently.

### Step 3.5: Draft and confirm the PRD

Synthesize a PRD from the Step 3 answers: Purpose and Success criteria become Problem and Goals, Non-goals stays Non-goals, and a new Requirements section states the user-facing capabilities this needs, in behavior terms, not implementation. Present it and ask: **"Does this capture the problem and what it needs to do? Anything to add or change?"** Revise until confirmed. This is the requirements gate: Step 4 designs approaches against a confirmed PRD, not an implicit one. Keep the PRD itself out of scope details, technical approach, or components: those are the design doc's job, not this one's.

### Step 4: Propose approaches

Present 2-3 distinct approaches with their trade-offs. Lead with your recommendation and say why. Keep each approach to what matters: what it does, its main cost, and what it rules out. Let the user pick or push back.

### Step 5: Route check

Look at the chosen direction against a three-part test, all required: the decision is hard to reverse once made, it would be non-obvious to a future reader why it was made this way, and it's the product of a genuine trade-off, not a forced or obvious choice. When all three hold (a data model, a public contract, a cross-cutting dependency are common shapes), flag it and offer `/playbook:adr` for the deep record: **"This carries an architectural call worth a formal record. Route to /playbook:adr for that decision? I'd recommend yes because it's hard to reverse."** When any leg is missing, the handoff target is `/playbook:scope`. `--adr` forces the `/playbook:adr` route.

### Step 5.5: Offer a validation spike

If the chosen approach rests on a premise Step 2 tagged LOW confidence, offer to check it before writing anything down: **"This approach assumes [premise], which I couldn't verify. Want a quick throwaway spike to check it first?"** Skip this step entirely when nothing is LOW confidence.

On yes:

- Build one small, self-contained, runnable artifact that exercises just the uncertain logic. No real persistence, no polish, no setup beyond running it.
- Walk it through the specific edge cases the premise is actually in doubt about, not just the happy path.
- Report what you learned. Update the Step 2 confidence tag: resolved, or still open and now an explicit open item in the Step 7 trailer.
- Commit the spike to a dedicated throwaway branch (never main, never the working branch), then leave the working tree clean. If it stays worth keeping as a reference, note the branch name in the design doc's open items; otherwise it's just there if anyone needs to check the reasoning later.

The spike is disposable and scoped to one premise. It never becomes part of the design doc's content and never starts the real implementation; that's still `/playbook:scope` and `/playbook:implement`'s job.

### Step 6: Present the design

Present the design in sections scaled to complexity: a few sentences where it's straightforward, more where it's nuanced. The problem and requirements are already confirmed (Step 3.5); cover the chosen approach, the key components and their boundaries, and the main risks. Ask after each section whether it looks right. Revise until the user approves. Do NOT write the doc before approval.

Keep applying the domain glossary discipline from Step 3 here too: a term that turns out ambiguous while presenting the design gets the same treatment, resolved and written to `GLOSSARY.md` immediately.

### Step 7: Write the PRD and the design doc

On approval, save both to `.claude/designs/`. First time in this repo, create the dir and ignore it (same pattern as `/playbook:scope`'s plans):

```bash
ROOT=$(git rev-parse --show-toplevel)
mkdir -p "$ROOT/.claude/designs"
grep -qxF '.claude/designs/' "$ROOT/.gitignore" 2>/dev/null || printf '.claude/designs/\n' >> "$ROOT/.gitignore"
```

**PRD** (from Step 3.5), `.claude/designs/<YYYY-MM-DD>-<slug>-prd.md`, product-facing:

```markdown
# PRD: <title>

Date: <YYYY-MM-DD>
Status: Approved
Design doc: <path to the paired design doc below>

## Problem
<why, what's broken or missing today>

## Goals
<what this must achieve, observably>

## Non-goals
<what this explicitly won't do>

## Requirements
<user-facing capability statements, not implementation>

## Success metrics
<how we'll know it worked>
```

**Design doc**, `.claude/designs/<YYYY-MM-DD>-<slug>.md`, engineering-facing:

```markdown
# <title>

Date: <YYYY-MM-DD>
Status: Approved (design), pending planning
Ticket: <id and link, if ticket mode; omit otherwise>
PRD: <path to the PRD above>

## Problem
See `<prd-path>` for the full problem statement, goals, and requirements.

## Context
<cited digest: code file:line from Step 2, and ticket sources from Step 1.5 if any>

## Approaches considered
<2-3, trade-offs, which was chosen>

## Decision
<the chosen approach and why, with rejection notes for the others>

## Components and boundaries
<the units and their interfaces, kept light; /playbook:scope does the detailed Work Units>

## Risks and open questions
<what's flagged but accepted, and what /playbook:scope still needs to decide>

## Routing note
<route to /playbook:adr for a decision, or to /playbook:scope>

## Confidence + open items

- Confidence: HIGH | MEDIUM | LOW, <one line on what makes it that>
- Open items (verify downstream): each one MUST be stated precisely enough that whoever picks it up next knows exactly what to check or decide. If it can't be phrased that precisely yet, say so plainly instead of listing a vague placeholder that only looks actionable.
  - <blind spot or LOW-confidence premise>, <who verifies: /playbook:scope interview, /playbook:implement watch>
```

Save both files. Don't auto-commit.

**Knowledge capture.** If a project store is present at `~/.claude/memory/<owner>/<repo>/`, persist the chosen approach and each rejected approach (with the reasoning from Step 4's trade-offs) as project memory facts (`type: project`, `anchors:` to any files discussed), and update the project's `MEMORY.md` index. This is what Step 2's rejected-idea check reads on a future run; skipping it here means that check finds nothing. If no project store is present, skip silently.

### Step 8: Self-review

Look at both docs with fresh eyes and fix inline:

- **Placeholders:** any TBD, TODO, or vague requirement? Fill it.
- **Consistency:** do the sections agree? Does the design doc's Decision actually satisfy the PRD's Goals and Requirements?
- **Scope:** is this focused enough for one plan, or does it need decomposition?
- **Ambiguity:** could a requirement read two ways? Pick one and make it explicit.
- The "Confidence + open items" trailer is present and filled with the real open items from Step 2 (dropped or LOW-confidence premises), each stated precisely enough to act on, not left as the template placeholder.

### Step 9: Human review gate

Tell the user: **"PRD and design doc written to `<prd-path>` and `<design-path>`. Give them a read and tell me if you want changes before we plan."** Wait. If they request changes, make them and re-run Step 8.

### Teardown (MUST run, even on failure or abort)

`TaskStop` every subagent spawned in this flow that is still alive. Confirm via `TaskList` that none from this run remain before proceeding to the handoff.

### Step 10: Handoff

Never invoke the next command from inside this session: brainstorm's exploration, the interview, and the digest are exactly the accumulated context a fresh planning phase shouldn't inherit. There's no way to clear it from within a running turn (`/clear` is a user action, no hook or tool can trigger it), so the fix is not auto-chaining in the first place, only telling the user what to run next.

Once approved, unless `--no-chain`, tell the user:

- **`/playbook:scope` route:** **"Run `/clear`, then run `/playbook:scope <design-doc-path>` to continue with a clean context. It reads the PRD and design doc and skips what we already settled."**
- **`/playbook:adr` route** (from Step 5 or `--adr`): **"Run `/clear`, then run `/playbook:adr <design-doc-path>` to record that decision with a clean context."**

With `--no-chain`, print both doc paths and stop; skip the next-step suggestion entirely.
