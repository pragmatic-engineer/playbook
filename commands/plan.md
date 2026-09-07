---
description: Use when an idea needs to become a plan, whether it is still raw and undirected or the direction is already settled, or when the user says let's brainstorm this, let's plan this, let's scope this, explore this idea, or break this down. One continuous, interview-driven session that challenges the premise, weighs 2-3 approaches, then interviews for Work Units and Segments, and produces a verified, self-contained implementation plan ready for /playbook:implement.
allowed-tools: Bash, Read, Grep, Glob, Write, Edit, Agent, Skill, WebFetch
argument-hint: "[idea | PROJ-123 | ./prompt.md] [--ticket <id>] [--depth 0-2] [--adr] [--auto] [--help]"
model: opus
effort: high
---

# Plan: Discovery Through an Implementation Plan

Turn a raw idea, a settled direction, or a ticket into a verified, self-contained implementation plan in one sitting. The session opens divergent: it challenges the premise, explores the codebase and memory, and weighs 2-3 approaches until you approve a design. It then continues, in the same conversation with no `/clear` in between, into a convergent interview for Work Units and Segments, a 3-phase quality gate, and one saved plan file. There's only one artifact and one command: the old two-step handoff between a design doc and a separate planning session is gone.

Invoked as `/playbook:plan`. The remaining arguments are an optional idea seed, ticket id, or file path.

The terminal state is a verified implementation plan at `$(playbook path plans)/<topic-slug>.md`, ready to run with `/playbook:implement`. Do NOT write real code, scaffold anything, or start the implementation here. The one narrow exception is Step 5.5's optional validation spike: throwaway code to check a single uncertain premise, never part of the saved plan's content and never the start of the real build.

## Help

If the arguments contain `--help`, print this and stop:

```
/playbook:plan - One session from a raw idea to a verified implementation plan

USAGE:
  /playbook:plan [idea]              Start an interactive session
  /playbook:plan "offline mode"      Start with an idea seed
  /playbook:plan PROJ-123             Pull a ticket and start from it
  /playbook:plan ./notes.md           Load the idea seed from a file

OPTIONS:
  --ticket <id>  Force ticket mode for <id> (skip seed/file detection).
  --depth <0-2>  How far to crawl ticket links: 0 ticket only, 1 direct
                 links (default), 2 one more hop. Always bounded.
  --adr        Flag this session's decision as ADR-worthy: skip the Step 5
               reversibility test and go straight to recommending
               /playbook:adr at Step 12. The session still continues
               through to a saved implementation plan; --adr adds a
               recommendation, it does not end the session early.
  --auto       Autonomous, but narrower than it sounds: it only self-answers
               the convergent phase (Step 7 onward), taking the recommended
               answer for every Work Unit and Segment decision and recording
               it as an assumption, then runs the quality gate and saves
               without pausing. The divergent phase (approach selection, the
               /playbook:adr route check, design approval) always stops for
               a human, regardless of this flag. See Autonomous Mode below;
               this is a narrower, intentionally different scope than the
               old /playbook:scope --auto, which also auto-picked the
               approach on a raw topic-only invocation.
  --help       Show this help

Asks one question at a time with a recommended answer. Given a ticket id,
pulls the ticket (description, comments, attachments, linked items) via a
connected MCP or a configured provider command, then explores the codebase and
memory in parallel before asking you. Checks for an in-progress checkpoint
from an earlier, interrupted run on the same topic and offers to resume it.
Confirms a problem statement before proposing 2-3 approaches, gets your
approval on the chosen design, then continues, in the same session, into the
Work Unit and Segment interview, the 3-phase quality gate, and one saved plan
file at $(playbook path plans)/<topic-slug>.md.
```

## Core Rules (MUST)

**Autonomous mode (`--auto`) narrows to the convergent phase only.** When `--auto` is set, the divergent phase (Steps 1 through 6) still asks every question it normally would: approach selection, the `/playbook:adr` route check, and the Step 6 design approval are unconditional human gates that no flag skips. From Step 7 onward, `--auto` resolves every decision yourself, taking the answer you would have recommended, recording it in an **Assumptions** list, and skipping the Step 8 and Step 11 confirmation pauses. See **Autonomous Mode (`--auto`)** below for the full boundary and why it changed from `/playbook:scope`'s old behavior. The rules below otherwise describe the default interactive mode.

1. **Ask ONE question at a time.** Not two, not a batch. One question, wait for the answer, then the next. Batch at most 2 into one numbered round only when neither could plausibly depend on exploration or terminology the other's answer might surface, true independence, not just topical proximity (Step 3 covers this in detail). The very first message is always a single question, never a round: there's no established frontier yet to batch from.
2. **Explore before asking.** If the codebase, the memory stores, or an in-progress checkpoint settles a question, resolve it yourself and report what you found. Only ask about intent, constraints, and preferences the code can't answer.
3. **Challenge the premise (divergent phase only).** Don't accept the framing at face value. Ask whether this is the right problem, whether a simpler direction meets the goal, and what "done" actually looks like. Once Step 6 approves a design, stop relitigating it: the convergent phase plans the approved direction, it doesn't reopen it.
4. **Present a design and get unconditional approval before continuing (Step 6).** Hard gate, every time, even for a small idea, even under `--auto`. The design can be a few sentences, but you MUST present it and get a yes before Step 7 starts.
5. **Walk the decision tree in the convergent phase.** Each answer may open new branches. Track which are resolved and which are still open. Don't jump to unrelated topics while a branch has unresolved dependencies.
6. **Do NOT write real code.** The output is a plan file, not implementation. The one narrow exception is Step 5.5's optional validation spike: throwaway code to check a single uncertain premise, never part of the saved plan's content, never the start of the real build.
7. **Do NOT produce Work Units, Segments, or file-level plan detail before Step 6 approves the design, and not until every convergent branch is resolved.** The divergent phase decides direction; the convergent phase (Step 7 onward) turns an approved direction into a plan `/playbook:implement` can run.

## Argument Resolution

Resolve the argument in this order:

1. **Ticket:** if `--ticket <id>` is set, or the argument matches a ticket key (`[A-Z][A-Z0-9]+-\d+`, e.g. `PROJ-123`) or a known tracker URL (Jira, Linear, GitHub issue), treat it as a ticket and go to Step 1.5 to pull it. `--ticket` forces ticket mode even for an ambiguous value.
2. **File path:** if it starts with `./`, `../`, `/`, or `~`, or ends with `.md`, `.txt`, `.yaml`, `.yml`, check whether it exists with the Read tool. If it exists, read it and use it as the idea seed; if not, treat it as a plain-text seed.
3. **Plain text:** otherwise the argument is the idea seed.
4. **No argument:** ask what we're exploring before anything else.

Strip `--ticket <id>`, `--depth <n>`, `--adr`, and `--auto` (like `--help`) before resolving the seed. Don't read `.gitignore`d files even if the seed or ticket mentions them.

## How It Works

### Step 0: Load skills and check for an in-progress checkpoint

**Load skills.** Load `playbook:writing-style` (voice, banned words, no dashes; every question and every written section follows it) and `playbook:grounding-research` (cite `file:line`, tag `[unverified]` when you can't confirm; governs the context digest and any self-answering).

**Derive `<topic-slug>` (MUST, before anything else).** Kebab-case the plain-text idea seed as typed, or, in ticket mode, the ticket id itself (e.g. `PROJ-123` becomes `proj-123`), the same convention `commands/adr.md:21`'s `{kebab-title}` filename uses. Ticket mode derives the slug from the id, not the ticket's title, because Step 1.5 (which pulls the title) runs after this check; using the id keeps the slug available before the ticket is fetched, and it stays the same value for the rest of the session, including Step 12's save. This is the same slug the checkpoint file and the final plan file both use.

**Check for a checkpoint.** This command's own bespoke checkpoint/resume mechanism, not the generic `/playbook:session-handoff` (that one is explicitly invoked rather than automatic, keyed by project-slug plus a suffix rather than this topic's own slug, and produces a free-text summary rather than a structured plan-in-progress; none of that serves an automatic, topic-slug-keyed resume). Resolve the plans directory and look for a matching checkpoint:

```bash
if ! PLANS_DIR=$(playbook path plans 2>&1); then
  echo "error: playbook path plans failed: $PLANS_DIR" >&2
  exit 1
fi
CHECKPOINT="$PLANS_DIR/<topic-slug>.checkpoint.md"
```

- **Found:** read it. Show its `Goal:` line and its last-modified date (not a bare yes/no), then ask once: **"Found an in-progress plan for `<topic>` last touched `<date>`, goal: `<goal-line>`. Resume it? I'd recommend yes because picking up mid-session avoids redoing settled decisions."**
  - **Yes:** load its Decisions Made, Out of Scope, Open Risks, chosen Approach, `Design approved` marker, and the Work Units/Segments table as far as they got. Resume from the first step whose output the checkpoint doesn't yet have: only a Goal and no Decisions Made resumes at Step 1; an Approach with no `Design approved` marker resumes at Step 5 (the route check still needs an answer); a `Design approved` marker with no Work Units resumes at Step 7. Never resume at Step 7 on an Approach alone: Step 6's approval is a hard gate (Core Rules), and a checkpoint that hasn't recorded it hasn't cleared that gate yet, no matter how settled the approach looks.
  - **No:** start fresh. The stale checkpoint is not deleted here: it gets overwritten in place as new decisions are appended through Step 3 onward (see the write shape below), and only Step 12 deletes it, once a completed plan actually replaces it. Silently deleting a stale checkpoint the moment someone declines to resume would destroy a session's progress on a whim, on the chance they meant to resume a different topic under the same seed.
- **Not found:** proceed to Step 1 with nothing to resume.

**Checkpoint content and write shape.** The checkpoint is a single Markdown file: a `Goal:` line, `Decisions Made`, `Out of Scope`, `Open Risks`, `Approach`, a `Design approved` marker (set only once Step 6's gate clears), and the `Work Units / Segments` table as far as they've been settled, the same structured shape the final plan's condensed sections use. Rewrite it after each resolved decision, not only in Step 3 and Step 7: Step 4 (approach chosen), Step 5 (route-check answer), and Step 6 (the `Design approved` marker) each trigger a rewrite too, under the same locked-write discipline `MEMORY.md`/`GLOSSARY.md` use elsewhere in this repo (a mkdir-based advisory lock, matching the Rust hooks' `with_dir_lock` in `src/common/atomic.rs`), adapted here to a full rewrite rather than a one-line append, since the checkpoint's content is a structured document, not an append-only log:

```bash
LOCK="$CHECKPOINT.lock"
ACQUIRED=0
for _ in $(seq 1 20); do
  mkdir "$LOCK" 2>/dev/null && { ACQUIRED=1; break; }
  sleep 0.05
done
if ! cat > "$CHECKPOINT" <<'EOF' 2>/tmp/plan-checkpoint-err
<the full updated checkpoint: Goal, last-touched date, Decisions Made,
Out of Scope, Open Risks, Approach, and the Work Units/Segments table as
far as they got>
EOF
then
  echo "couldn't save checkpoint: $(cat /tmp/plan-checkpoint-err 2>/dev/null), continuing without resume safety this turn"
fi
[ "$ACQUIRED" = 1 ] && rmdir "$LOCK" 2>/dev/null
```

A checkpoint write failure warns inline with that message and the interview keeps going. Never abort a session over a persistence hiccup: a human is present to see the warning, and losing resume safety for one turn is a far smaller cost than losing the whole session.

### Step 1: Frame the idea and check scope

Restate the idea in one or two sentences so we agree on what we're exploring.

**Scope check (MUST).** If the idea is several independent subsystems (e.g. "a platform with chat, billing, and analytics"), stop and flag it. Help decompose into independent pieces, name how they relate and what order to build them, then plan the first piece through the normal flow. Don't design a tangle.

### Step 1.5: Pull the ticket (ticket mode only)

Run this only when Argument Resolution found a ticket. Skip it entirely for a plain idea or file seed.

**Connect (layered).** Find a way to reach the tracker, in order:

1. **MCP:** search for a connected ticket tool (`ToolSearch` for Jira, Linear, Atlassian, or issue tools). If one is connected, use it.
2. **Provider command:** else look for a configured fetch command. Read `.claude/plan.config` (or the repo's existing tracker config) for a per-tracker command with an `{id}` placeholder, for example `jira issue view {id} --raw` or `linear issue {id} --json`, and run it with Bash. A public tracker URL with no auth can be read with `WebFetch`. A page that needs auth or JavaScript rendering that `WebFetch` can't handle can be opened with the `agent-browser` MCP, if it's connected: `open` the url, then `snapshot` for the accessibility tree and `screenshot` for visual content.
3. **Neither:** stop and tell the user how to connect one (an MCP server or a provider command), then offer to continue with the ticket id as a plain-text seed. Do NOT fabricate ticket contents.

**Crawl (bounded, never infinite).** Gather, then stop:

- The ticket itself: title, description, status, and comments.
- Attachments: read images visually and PDFs or docs as pages. For an attachment the tracker exposes only as a web link, open it with the `agent-browser` MCP (if connected) and `snapshot` or `screenshot` it. Note and skip binaries and anything the tracker doesn't expose.
- One hop of direct links: linked issues, sub-tasks, parent epic, and linked PRs. `--depth` controls this (0 = ticket only, 1 = direct links (default), 2 = one more hop). Clamp `--depth` to the range 0 to 2, so the crawl is never unbounded.
- Bounds: cap total related items at about 15, dedup visited tickets by id, and stop early when a hop adds nothing new.

**Discover in parallel.** Fan out discovery agents over the gathered sources (issue the Agent calls in one message so they run at once, per Step 2): the ticket body plus comments, batches of linked items, and the attachments. Each returns a short cited summary (the source id or url, and the facts that bear on the work). These feed the Step 2 digest alongside the codebase exploration. Assign each discovery agent a stable `name` at spawn and `TaskStop` it as soon as it returns. A spawned agent stays idle-alive for `SendMessage` follow-ups and this flow never reuses a finished one, so leaving it unstopped keeps a subagent running in the background.

The ticket's title and description become the idea seed for Step 1's framing. Record the ticket id and link so Step 9 can cite them in the plan.

### Step 2: Explore context in parallel

Fan out `Explore` agents to map what the dialogue needs. **Dispatch them in parallel: issue all the Agent calls in a single message so they run at once.** Read-only exploration has no shared state, so parallel is always the default here.

Scale the fan-out to the idea: one agent for a tiny change, up to about four for a broad feature. Give each a distinct area, for example:

- Existing patterns and prior art for this kind of change.
- Integration points and the consumers a change would touch.
- Constraints: config, conventions, and anything in the code that limits the options.

Alongside the `Explore` agents, dispatch one independent `critic` agent (`subagent_type: playbook:critic`, focus `premise`), prompted to challenge the premise rather than explore code. Its return feeds the Step 2 digest and the Step 4 approach exploration, so premise-challenge isn't only in the orchestrator's head. Close it on return with the others (Step 2 teardown).

Built-in `Explore` agents have been reliable at returning results; the `critic` is structurally read-only and has only the return channel, so it may deliver nothing (`playbook:delegating-subagents`). An area whose agent returned nothing was NOT explored: it does not mean there is nothing there. Say which areas are unexplored rather than treating the digest as complete, and if the premise-challenge came back empty, challenge the premise yourself before moving to Step 3.

**Check memory and prior plans.** Alongside the `Explore` agents, check whether a memory store exists: the global store at `~/.config/playbook/memory/MEMORY.md` and the project store at `~/.config/playbook/memory/<owner>/<repo>/MEMORY.md` (`<owner>/<repo>` from `git remote get-url origin`). Load the relevant fact files from whichever exist. Also scan `$(playbook path plans)/*.md` (excluding `*.checkpoint.md` and `*-quality.md`) for a prior saved plan whose title or topic overlaps this idea, a cheap keyword match, not semantic search. When neither has anything relevant, skip this silently. When either surfaces a plausible match, a decision already made or an idea already rejected, say so in the digest: what was decided, when, and why. Ask directly whether anything has changed before diverging into new approaches, rather than re-litigating a settled call from scratch.

Consolidate into a short cited digest (a few bullets, each with `file:line`). This grounds the questions that follow so you ask about intent, not about facts the code already holds. In ticket mode, fold the Step 1.5 ticket findings into the same digest, citing the source id or url for those. Assign each `Explore` agent a stable `name` at spawn and `TaskStop` it as soon as it returns. A spawned agent stays idle-alive for `SendMessage` follow-ups and this flow never reuses a finished one, so leaving it unstopped keeps a subagent running in the background.

**Verify the load-bearing premises before diverging.** From the digest, list the load-bearing citations: the premises the design will rest on (for example "the code already does X", "there is no existing helper for Y"). Re-read each cited `file:line`. Drop or tag `[unverified]` any that don't hold, and tag each surviving context bullet HIGH / MEDIUM / LOW (the `playbook:grounding-review` skill defines the levels). Spot-check the load-bearing claims only; don't audit every citation, or the divergent phase drags. Dropped or LOW-confidence premises become open items carried into Step 7 (see below), not a separate handoff document.

### Step 3: Interactive discovery

Ask questions one at a time by default, each with a recommended answer and reasoning, each following from the last. Cover:

- **Purpose:** why this, why now? What breaks or stays broken without it?
- **Success criteria:** what does "done" look like, observably?
- **Constraints:** technical, product, or time limits that rule options in or out.
- **Non-goals:** what this explicitly won't do.

**Rounds (narrow exception to one-at-a-time).** Think of the open questions as a frontier: the ones you could ask right now without guessing at an answer you haven't heard yet. The frontier here is almost always one question deep, since these four categories inform each other adaptively (Purpose shapes what Success criteria means, and so on), not as an upfront-resolvable tree. Batch two into one numbered round only on the rare case where they're genuinely independent. Never batch more than 2: a small idea's whole question budget (see Adapting to Complexity) is 2-4, and a bigger round reintroduces the wall-of-text problem the recommended-answer mechanic exists to avoid. Format a round like this:

```
1. **<question title>**: <question body>
   Recommended: <your recommended answer>

2. **<question title>**: <question body>
   Recommended: <your recommended answer>
```

A single question uses the same numbered, recommended-answer shape without the second entry.

Between questions or rounds, explore further if an answer opens a new area, and report what you found before the next question or round. After a round, re-run this check and the domain glossary check (below) against every answer just received, not only the most recent one, before drafting the next round. If either check would change a question the user hasn't answered yet (asked in the same round as an answer that reshapes it), withdraw it: don't let the user answer it against context that's already stale, re-derive it in the next round instead.

**Checkpoint after each resolved decision (MUST).** The moment a question in this step resolves, append it to the checkpoint's Decisions Made list and rewrite the checkpoint file using Step 0's locked write shape. This is what makes a mid-interview interruption resumable: without it, a crash between Step 3 and Step 12 would lose everything settled so far.

**Domain glossary (when a term is genuinely ambiguous or new).** If the conversation turns on a term that's overloaded, vague, or new to this codebase, don't just use it and move on: propose a precise definition and check it with the user. This isn't for every noun in a small idea, only for a term the design actually hinges on. Write it to `GLOSSARY.md` at the target repo's root (create the file only on its first real entry; it's tracked in git, not ignored, since its value is shared vocabulary across future sessions, not scratch). Each entry states what the term IS in one or two sentences, not what it does, plus a short list of synonyms to avoid so the disambiguation is recorded, not just implied. Write it the moment it resolves, don't batch it for later. If `GLOSSARY.md` already has a conflicting entry for the term, surface the conflict to the user instead of overwriting it silently.

Append with the same locked-write shape the checkpoint and `MEMORY.md`'s index use: a mkdir-based advisory lock, matching the Rust hooks' `with_dir_lock` (`src/common/atomic.rs`). Two concurrent sessions can each resolve a term at the same moment; a plain check-then-append can silently drop one of the two lines.

```bash
GLOSSARY_MD="$ROOT/GLOSSARY.md"
LOCK="$GLOSSARY_MD.lock"
ACQUIRED=0
for _ in $(seq 1 20); do
  mkdir "$LOCK" 2>/dev/null && { ACQUIRED=1; break; }
  sleep 0.05
done
printf '%s\n' "<term entry>" >> "$GLOSSARY_MD"
[ "$ACQUIRED" = 1 ] && rmdir "$LOCK" 2>/dev/null
```

Scale the depth: 2-4 questions for a small idea, more for a broad one. Don't over-interview a simple thing (see Adapting to Complexity).

### Step 3.5: Draft and confirm the problem statement

Synthesize the running document's problem section from the Step 3 answers: Purpose and Success criteria become Problem and Goals, Non-goals stays Non-goals, and a new Requirements section states the user-facing capabilities this needs, in behavior terms, not implementation. Present it and ask: **"Does this capture the problem and what it needs to do? Anything to add or change?"** Revise until confirmed. This is the requirements gate: Step 4 designs approaches against a confirmed problem statement, not an implicit one. Keep this section free of scope details, technical approach, or components: those come later, once a direction is chosen. This stays folded into the one running document rather than a separate file: there's no PRD to hand off, because nothing hands off until Step 12.

### Step 4: Propose approaches

Present 2-3 distinct approaches with their trade-offs. Lead with your recommendation and say why. Keep each approach to what matters: what it does, its main cost, and what it rules out. Let the user pick or push back.

**Checkpoint the chosen approach (MUST).** The moment the user picks one, write it to the checkpoint's `Approach` field and rewrite the file using Step 0's locked write shape.

### Step 5: Route check

Look at the chosen direction against a three-part test, all required: the decision is hard to reverse once made, it would be non-obvious to a future reader why it was made this way, and it's the product of a genuine trade-off, not a forced or obvious choice. When all three hold (a data model, a public contract, a cross-cutting dependency are common shapes), flag it and offer `/playbook:adr` for the deep record: **"This carries an architectural call worth a formal record. Route to /playbook:adr for that decision? I'd recommend yes because it's hard to reverse."** `--adr` forces this recommendation without running the three-part test.

**This fires unconditionally regardless of `--auto`.** Even in autonomous mode, the divergent phase, including this route check, always stops for a human: `--auto` only reaches its self-answering behavior from Step 7 onward (see Autonomous Mode below). Record the answer (recommend `/playbook:adr` or not) either way; Step 12 surfaces it. Answering this question does not end the session: whether or not the user wants an ADR, the flow continues into Step 5.5 and onward to a saved implementation plan. An ADR and an implementation plan are not alternatives, they're two different artifacts this decision may need.

**Checkpoint the route-check answer (MUST).** Write the recommendation and the user's answer to the checkpoint's `Decisions Made` list and rewrite the file using Step 0's locked write shape.

### Step 5.5: Offer a validation spike

If the chosen approach rests on a premise Step 2 tagged LOW confidence, offer to check it before writing anything down: **"This approach assumes [premise], which I couldn't verify. Want a quick throwaway spike to check it first?"** Skip this step entirely when nothing is LOW confidence.

On yes:

- Build one small, self-contained, runnable artifact that exercises just the uncertain logic. No real persistence, no polish, no setup beyond running it.
- Walk it through the specific edge cases the premise is actually in doubt about, not just the happy path.
- Report what you learned. Update the Step 2 confidence tag: resolved, or still open and now an explicit open item carried into Step 9's plan.
- Commit the spike to a dedicated throwaway branch (never main, never the working branch), then leave the working tree clean. If it stays worth keeping as a reference, note the branch name as an open item; otherwise it's just there if anyone needs to check the reasoning later.

The spike is disposable and scoped to one premise. It never becomes part of the saved plan's content and never starts the real implementation; that's still `/playbook:implement`'s job.

### Step 6: Present the design

Present the design in sections scaled to complexity: a few sentences where it's straightforward, more where it's nuanced. The problem and requirements are already confirmed (Step 3.5); cover the chosen approach, the key components and their boundaries, and the main risks. Ask after each section whether it looks right. Revise until the user approves. Do NOT continue into Step 7 before approval, even under `--auto`.

**Checkpoint the approval (MUST).** The moment the user approves, set the checkpoint's `Design approved` marker and rewrite the file using Step 0's locked write shape. This is the field Step 0's resume logic checks before it will resume at Step 7: an `Approach` alone never implies approval.

Keep applying the domain glossary discipline from Step 3 here too: a term that turns out ambiguous while presenting the design gets the same treatment, resolved and written to `GLOSSARY.md` immediately, with the same locked append.

**This is the divergent/convergent boundary.** No `/clear` happens here: the session continues directly into Step 7 with the same context, the same digest, the same approved design. Collapsing this boundary is the entire reason this command exists instead of two separate ones: the old handoff between a design doc and a fresh planning session is gone, along with the phrase-matching ambiguity it created at the routing-hook level between "still exploring" and "ready to plan."

### Step 7: Convergent interview for Work Units and Segments

**This is where `--auto` starts applying.** Everything from here through Step 11 can be self-answered under `--auto`; everything before this point cannot (see Autonomous Mode below).

Skip Goal clarification and Scope boundaries here: Step 3.5 already confirmed the problem, goals, and non-goals, and re-asking them would relitigate a decision the divergent phase already settled. Start directly at implementation-detail questions, aimed first at whatever's still open: Step 2's dropped or LOW-confidence premises, and Step 5.5's spike findings if any are still unresolved. Those are exactly the premises the divergent phase couldn't fully verify.

Ask questions one at a time. Each question should follow from the previous answer (don't jump topics), include your recommended answer with reasoning, and resolve a specific branch.

Types of questions, in roughly this order (adapt):

- **Existing patterns:** "I found [pattern] in the codebase. Follow it or break from it? **I'd recommend following it** because [reason]."
- **Architecture decisions:** "Two approaches: A does [X] but [downside]. B does [Y] but [cost]. **I'd recommend A** because [reason]."
- **Dependencies:** "This depends on [system]. How should it interact? **I'd recommend [approach]** because I see [evidence in code]."
- **Edge cases:** "What happens when [failure scenario]? **I'd recommend [handling]** because [reason]."
- **Migration/rollout:** "How do we get from current state to the new state? **I'd recommend [strategy]** because [reason]."

Between questions, explore the codebase if the answer reveals new areas. Report what you found before asking the next question.

**Checkpoint after each resolved decision (MUST), same as Step 3:** append it to the checkpoint's Decisions Made list (or its Work Units/Segments table, once those start taking shape) and rewrite the checkpoint file using Step 0's locked write shape.

**Knowledge capture (memory).** When exploration reveals a durable convention or gotcha about the codebase (true regardless of this plan), and a project store is present at `~/.config/playbook/memory/<owner>/<repo>/`, persist it as a project memory fact right then: a kebab-case file in `~/.config/playbook/memory/<owner>/<repo>/` with `name`/`description`/`type: project`/`links:`/`anchors:` (to the files), plus its `MEMORY.md` index line. When no project store is present, skip this step silently.

**Locked index append (MUST, whenever the index line is written, here or at Step 12).** Two `cc` sessions in the same repo can each persist a fact around the same moment; a plain check-then-append can silently drop one of the two lines. Append with the same mkdir-based advisory lock the Rust hooks use (`src/common/atomic.rs`'s `with_dir_lock`): briefly wait for the lock, append regardless of whether it was acquired (never block indefinitely on a stuck lock), remove the lock directory only if this run created it.

```bash
MEMORY_MD=~/.config/playbook/memory/<owner>/<repo>/MEMORY.md
LOCK="$MEMORY_MD.lock"
ACQUIRED=0
for _ in $(seq 1 20); do
  mkdir "$LOCK" 2>/dev/null && { ACQUIRED=1; break; }
  sleep 0.05
done
printf '%s\n' "- [<kebab-title>](<file>.md): <one-line hook>" >> "$MEMORY_MD"
[ "$ACQUIRED" = 1 ] && rmdir "$LOCK" 2>/dev/null
```

**Work Unit sizing (MUST).** Each WU is one coherent commit: small enough to review on its own, following the `playbook:engineering-standards` size limits and incremental-delivery guidance. Prefer more, smaller WUs over a few large ones; `/playbook:implement` commits each separately. The `Files` column lists production and test files. The `Requires` column is the dependency edge `/playbook:implement` topologically orders and cycle-checks. The `Segment` column names the PR-sized group each WU belongs to (see below).

**Segment sizing (MUST).** A Segment groups Work Units into one PR-sized, reviewable increment. Each Segment becomes one pull request, so:

1. **One concern per Segment.** Data layer, service layer, wire-up, and docs are separate Segments, not one, per `playbook:engineering-standards` "one concern per PR".
2. **Budget (three tiers, from `playbook:engineering-standards`).** Target under 500 changed lines per Segment (soft). A Segment estimated over 1000 (enforced) needs explicit justification in the plan; prefer to split it. Never plan a Segment estimated over 1500 (hard). `/playbook:implement` re-splits at the 1500 hard limit if reality exceeds the estimate.
3. **Ordering respects WU dependencies.** A Segment's WUs may only `Require` WUs in the same or an earlier Segment; no forward cross-Segment dependency. Default to a **linear** Segment chain (`S1 -> S2 -> S3`), which `/playbook:implement` maps to stacked PRs.
4. **Coverage.** Every WU belongs to exactly one Segment; no WU is left out and none appears in two.
5. **Suggestions, not law.** These are `/playbook:implement`'s starting point; note in the plan that it may re-split a Segment whose real diff blows the budget. Mark two Segments as parallel-safe only when their file sets are disjoint and neither `Requires` the other.

**Parallel-safety (MUST mark explicitly).** Assign a shared `Parallel group` label only when every member of the group:

1. has no dependency on another member (none appears in another's `Requires`),
2. touches a disjoint set of files (no file in two members), and
3. shares no mutable runtime state and no ordering-sensitive step (e.g. sequential DB migrations).

If any condition fails, leave the WUs ungrouped (they run sequentially). When unsure, leave sequential: a wrong parallel flag makes `/playbook:implement` run concurrent agents over the same files and corrupt the working tree. `/playbook:implement` re-verifies the flags before dispatching, but the plan should not assert parallelism it can't justify.

### Step 8: Confirm understanding

When every convergent branch is resolved, summarize:

```
## Design Summary

**Goal:** [one sentence]

### Decisions Made
1. [Decision]: [what was chosen] (because [reason])
2. ...

### Out of Scope
- [what was explicitly excluded]

### Open Risks
- [anything flagged but accepted]
```

This folds together decisions from both phases: the direction chosen at Steps 4 and 6, and the implementation detail settled in Step 7. Ask: **"Does this capture everything? Anything to change?"** Do NOT proceed until confirmed. **In `--auto`, skip this gate:** fold the summary and the Assumptions list into the plan and proceed.

### Step 9: Generate the implementation plan

Produce one self-contained document that `/playbook:implement` can consume directly. A condensed problem/goals/non-goals/approaches/decision section (from the divergent phase) comes first, followed directly by the Work Units/Segments plan (from the convergent phase). There's no separate PRD or design-doc file: approval already happened in-session at Steps 3.5 and 6, so the saved file only needs to record what was decided, not re-seek approval for it.

```
## Implementation Plan

**Status:** Proposed | **Date:** <YYYY-MM-DD>

### Background
[Why this, why now. One or two sentences from Step 1/Step 3's Purpose.]

### Problem
[From Step 3.5's confirmed problem statement.]

### Goals
[From Step 3.5.]

### Non-goals
[From Step 3.5.]

### Approaches considered
[2-3, trade-offs, which was chosen (Step 4). Rejection notes for the others.]

### Decision
[The chosen approach and why (Step 6). Ticket id and link, if ticket mode.]

### Architecture
[High-level design. Reference specific files and functions found during research.]

### Files to Create/Modify
| File | Action | Purpose |
|------|--------|---------|
| path/to/file.ts | modify | [what changes and why] |
| path/to/new.ts  | create | [what it does] |

### Segments (suggested PRs)
Ordered, PR-sized groups of Work Units. One concern each; each Segment becomes one pull request.
`/playbook:implement` honors these but may re-split a Segment whose real diff exceeds the 1500-line hard limit.
| Seg | Title | Work Units | Requires | Concern | Est. lines |
|-----|-------|-----------|----------|---------|-----------|
| S1 | [title] | WU-0, WU-1, WU-2 | none | [schema + types] | ~180 |
| S2 | [title] | WU-3 | S1 | [wire-up] | ~90 |

### Deliverables (Work Units)
Smallest independently-committable units, in dependency order. One WU = one small commit. Each WU
belongs to exactly one Segment.
| WU | Title | Files | Requires | Segment | Parallel group | Done When |
|----|-------|-------|----------|---------|----------------|-----------|
| WU-0 | [title] | path/to/types.ts | none | S1 | none | [observable acceptance] |
| WU-1 | [title] | path/to/a.ts, path/to/a.test.ts | WU-0 | S1 | P1 | ... |
| WU-2 | [title] | path/to/b.ts, path/to/b.test.ts | WU-0 | S1 | P1 | ... |
| WU-3 | [title] | path/to/index.ts | WU-1, WU-2 | S2 | none | ... |

### Parallel Groups
- **P1** (after WU-0): WU-1 and WU-2. Disjoint files, no shared state, safe to run concurrently by separate agents.
- **Sequential:** WU-0 first; WU-3 last (needs WU-1 and WU-2).

### Per-Work-Unit Detail
For each WU, in dependency order:

#### WU-N: [title]
- **Requires:** [WU-x, WU-y | nothing]
- **Files:** [exact paths, production + test]
- **Changes:** [concrete what-to-do]
- **Test scenarios:** [Gherkin Given/When/Then or TDD cycles this WU satisfies]
- **Done When:**
  - [ ] [observable acceptance criterion]

### Testing Strategy
- [What to test, which test files, what assertions]

### Routing note
[Recorded from Step 5: recommend /playbook:adr for the chosen decision, or none.]

## Confidence + open items

- Confidence: HIGH | MEDIUM | LOW, <one line on what makes it that>
- Open items (verify downstream): each one MUST be stated precisely enough that whoever picks it up next knows exactly what to check or decide. If it can't be phrased that precisely yet, say so plainly instead of listing a vague placeholder that only looks actionable.
  - <blind spot or LOW-confidence premise>, <who verifies: /playbook:plan interview, /playbook:implement watch>
```

The Testing Strategy MUST follow the `playbook:engineering-standards` skill (test types, isolation, TDD red/green/refactor, no coverage decrease).

### Step 10: Quality Gate (MUST)

After producing the plan, run the three-phase gate. Do NOT skip it. Do NOT ask the user to verify it. Do NOT proceed to Step 11 until it passes. The criteria are inline below. Run Phase 1 first (Phases 2 and 3 read its report), then dispatch Phase 2 and Phase 3 in parallel: issue both Agent calls in a single message so they run at once. The 1-before-(2,3) order is the only real dependency here; Phase 2 and Phase 3 are independent, so they never run one at a time. Consolidate all three at the Quality Gate Result.

**All three phase agents are structurally read-only, so the return value is their only channel and it is unreliable** (`playbook:delegating-subagents`; invoke it before dispatching). `fact-checker`, `critic` and `test-reviewer` hold Read, Grep, Glob and Skill only, and cannot write a report file: `playbook agents check` forbids `Write` and `Bash` for that tier by design. **Run Phase 1 inline**, since its checks are mechanical and finish in about two minutes with re-runnable output. A phase that returned nothing is **INCONCLUSIVE, never PASS**, and INCONCLUSIVE blocks exactly like FAIL: redo it inline or record the gap and name the phase that did not run. A gate that checked nothing must never read as a pass.

#### Phase 1: Fact-Check

Spawn a `fact-checker` agent (`subagent_type: playbook:fact-checker`) with the full plan and these criteria:

- Every referenced file path exists.
- Function/type signatures and imports in the plan match the real code.
- The plan is consistent with existing patterns and conventions.
- Downstream consumers of changed code are identified.
- The test infrastructure the plan assumes actually exists.
- The Work Unit dependency graph is acyclic, and each Parallel group's WUs have disjoint files with no dependency on each other (the parallel-safe flags are accurate).
- **Segments are well-formed:** every WU maps to exactly one Segment (full coverage, no WU in two); Segment order respects WU `Requires` (no forward cross-Segment dependency); each Segment's estimate is within budget (FAIL if a planned Segment exceeds the 1500 hard limit, WARN if it exceeds 1000 without justification or exceeds the 500 soft limit); any parallel-marked Segments have disjoint files and no mutual `Requires`.

Returns a structured PASS / FAIL / WARN report. Phase 1 folds a Verification Summary into the report, reusing the `playbook:grounding-review` table shape:

```markdown
## Verification Summary

| Referenced path | Confirmed? | Where used |
|---|---|---|
| <path> | Yes (Read) / No (not found) | WU-N |

Confidence: HIGH | MEDIUM | LOW
```

Spawn it with a stable `name`; the moment it returns, `TaskStop` it: a spawned agent stays idle-alive for `SendMessage` follow-ups and this flow never reuses a finished one, so leaving it unstopped keeps it running in the background. **Record the gate:** write its full raw return text to a file, e.g. `/tmp/<repo>/plan-<topic-slug>-fact-check.txt`, then run `playbook gate record <topic-slug> plan fact-check <that file>`. Do this every time Phase 1 returns, including every retry iteration below, not just the final one: `gate record` upserts on `(plan_slug, phase)`, so a stale FAIL from an earlier iteration is overwritten once a later iteration passes, and only the last recording before Step 11's `gate check` matters. **After it returns**, if a project store is present at `~/.config/playbook/memory/<owner>/<repo>/`, persist any durable gotcha it found as a memory fact; otherwise skip. **If any FAILs:** revise the plan and re-run Phase 1 (max 3 iterations). Don't proceed until it passes.

#### Phase 2: Adversarial Review

Spawn a `critic` agent (`subagent_type: playbook:critic`, focus `plan`) with the full plan and the Phase 1 report. It challenges the design:

- Is there a simpler alternative that meets the goal?
- Scope creep or over-engineering?
- Missing error paths or failure handling?
- Blast radius: what could this break?
- Contradictions with the fact-check report.

Returns a structured report. Spawn it with a stable `name`; the moment it returns, `TaskStop` it: a spawned agent stays idle-alive for `SendMessage` follow-ups and this flow never reuses a finished one, so leaving it unstopped keeps it running in the background. **Record the gate:** write its full raw return text to a file, e.g. `/tmp/<repo>/plan-<topic-slug>-adversarial.txt`, then run `playbook gate record <topic-slug> plan adversarial <that file>`. Do this every time Phase 2 returns, including every retry iteration below, not just the final one: `gate record` upserts on `(plan_slug, phase)`, so only the last recording before Step 11's `gate check` matters. **After it returns**, record any rejected simpler alternative (with reasoning) in the plan's Risks section. **If any FAILs:** revise and re-run Phase 2 (max 3 iterations).

#### Phase 3: Test Review

Spawn a `test-reviewer` agent (`subagent_type: playbook:test-reviewer`) with the plan's Testing Strategy and the Phase 1 report (it runs in parallel with Phase 2, so it doesn't wait on the adversarial findings). It evaluates the proposed tests against `playbook:engineering-standards`: regression-pinning, flakiness, boundary coverage, test independence, mock quality, assertion strength.

Returns a structured report. Spawn it with a stable `name`; the moment it returns, `TaskStop` it: a spawned agent stays idle-alive for `SendMessage` follow-ups and this flow never reuses a finished one, so leaving it unstopped keeps it running in the background. **Record the gate:** write its full raw return text to a file, e.g. `/tmp/<repo>/plan-<topic-slug>-test-review.txt`, then run `playbook gate record <topic-slug> plan test-review <that file>`. Do this every time Phase 3 returns, including every retry iteration below, not just the final one: `gate record` upserts on `(plan_slug, phase)`, so only the last recording before Step 11's `gate check` matters. **After it returns**, if a project store is present at `~/.config/playbook/memory/<owner>/<repo>/`, persist any durable test-quality pattern as a memory fact; otherwise skip. **If any FAILs:** revise the test plan and re-run Phase 3 (max 3 iterations).

#### Quality Gate Result

Present all three reports:

```
## Quality Gate Result

**Fact-Check:**        PASS (N/N checks passed)
**Adversarial Review:** PASS (N/N challenges passed)
**Test Review:**       PASS (N/N checks passed)
[or: BLOCKED: N FAILs, M WARNs]
```

WARNs are shown for awareness but do not block.

- **INCONCLUSIVE**: a phase returns INCONCLUSIVE, not PASS, when it couldn't actually perform its check: the agent failed to run or returned nothing, the target files were unreadable, or its confidence is LOW and blind spots dominate so a PASS would be unsupported. INCONCLUSIVE blocks the save exactly like FAIL and re-runs on the same max-3 loop; it's labeled distinctly so the cause reads as "couldn't verify," not "found a problem." A gate that checked nothing MUST NOT read PASS.

**Gate check (MUST, before Step 11):** run `playbook gate check <topic-slug> plan fact-check adversarial test-review`. Everything above (the phase reports, the Quality Gate Result presentation, the INCONCLUSIVE rule) is for the human reading it; this command's exit code is what actually decides whether Step 11 may run. Exit 0: proceed to Step 11. Non-zero exit: do NOT proceed to Step 11, and do NOT write any "gate passed" language; instead report the command's own output verbatim, since it already names exactly which phase is MISSING, FAIL, or INCONCLUSIVE, never re-narrate it in your own words, then revise and re-run the failing phase(s) on that phase's own retry loop above.

### Step 11: User Approval

**In `--auto`, skip this step:** there is no approval pause. After the gate passes, go straight to Step 12; a gate FAIL stops the run (Step 10) instead of prompting for an override. The rest of this step is the default interactive flow.

After the gate passes, present the full plan and the gate reports. Ask:

**"Quality gate passed. Does this plan look right? Anything to change before I save it?"**

If there are WARNs, highlight them: **"Note: N warnings flagged (see report). None are blocking."**

Do NOT save until the user explicitly approves. If they request changes, revise the affected parts, re-run the quality gate (Step 10), and present again.

**Override:** if the gate has FAILs and the user says to proceed anyway, record the override in the quality report file: `Quality gate override: proceeding despite FAIL on <check> because <user's reason>`.

### Step 12: Save & Next Steps

Only after the user approves. The plans directory lives outside this repo checkout (`$HOME/.config/playbook/repos/<owner>/<repo>/<worktree-id>/plans/`), resolved via the CLI, not gitignored: there is nothing repo-local left to ignore.

1. Save the plan to `$PLANS_DIR/<topic-slug>.md` and the gate reports to `$PLANS_DIR/<topic-slug>-quality.md`.
2. Delete the checkpoint file now that the final plan replaces it: `rm -f "$PLANS_DIR/<topic-slug>.checkpoint.md"`. The checkpoint's only job was resuming an interrupted session; once the finished plan is saved, keeping it around would leave a stale, superseded draft next to the real answer.
3. If a project store is present at `~/.config/playbook/memory/<owner>/<repo>/`, persist the plan's accepted key decisions (from both the divergent and convergent phases) as project memory facts (`type: project`, `anchors:` to the files they touch), and update `~/.config/playbook/memory/<owner>/<repo>/MEMORY.md` with the same locked append shown earlier. The graph rebuilds automatically on fact save via the PostToolUse hook. Otherwise skip.
4. Tell the user:
   - "Saved to `<plans-dir>/<topic-slug>.md`" (`<plans-dir>` is `playbook path plans`'s resolved path).
   - "Run `/clear`, then implement it with a clean context: `/playbook:implement <plans-dir>/<topic-slug>.md`." This session's back-and-forth is exactly what a fresh execution phase shouldn't carry forward; there's no way to clear it from inside this session, so say so instead of leaving it implicit.
   - If Step 5 recommended it or `--adr` was set: "Want to record the architectural call too? Run `/clear`, then `/playbook:adr` for that decision." Note it's a separate command producing a separate artifact, not a replacement for the saved plan.
   - **In `--auto`:** also list the **Assumptions** made (especially any `OPEN` ones) so the user can audit the autonomous choices before running `/playbook:implement`.

### Teardown (MUST run, even on failure or abort)

`TaskStop` every subagent spawned in this flow that is still alive (the Step 1.5 discovery agents, Step 2's `Explore` and `critic` agents, and the Step 10 Phase 1, 2, and 3 agents). Confirm via `TaskList` that none from this run remain before finishing.

## Autonomous Mode (`--auto`)

`--auto` is scoped narrower than the old `/playbook:scope --auto`: it automates only the convergent phase, Step 7 onward. The divergent phase, Steps 1 through 6, always stops for a human, regardless of flags: approach selection (Step 4), the `/playbook:adr` route check (Step 5), and the design approval (Step 6) are unconditional gates. This is an intentional, stated breaking change from today's `/playbook:scope --auto`, which, on a raw topic-only invocation with no prior design doc, auto-answered approach selection too. Now that the divergent phase and the convergent phase are one command instead of two, letting `--auto` skip the divergent phase's gates would mean nobody ever looks at the chosen direction before the plan gets written, which is a materially bigger risk than skipping the old `/playbook:scope --auto`'s implementation-detail assumptions.

Enable when `--auto` appears in the arguments; it's stripped (like `--help`) before resolving the topic seed. From Step 7 onward:

- **No questions.** For every decision Step 7 would ask, take the answer you would have recommended ("I'd recommend X because Y") and proceed. Still do the Step 2 research first: explore the codebase and, if a memory store is present, read it too, since a preference or convention there may override your default choice. When no memory store exists, skip that step silently.
- **Record assumptions.** Every self-made decision from Step 7 onward goes into an **Assumptions** list with its rationale, so the user can audit what was chosen for them. When you're genuinely split on a decision, record it as an `OPEN` assumption (with the leading option and why) rather than silently picking.
- **Skip the confirmation gates.** Do not pause at Step 8 ("Does this capture everything?") or Step 11 ("Does this plan look right?"). Fold the Design Summary and the Assumptions list into the saved plan instead.
- **Quality gate still runs (Step 10).** It needs no user input. If a phase still FAILs after its 3 iterations, STOP: do not save; report the failing checks and the assumptions made. No user is present to override a FAIL in `--auto`.
- **Save and report (Step 12).** On a passing gate, save the plan and quality report, then tell the user the paths, the assumptions made (flag any `OPEN` ones), and to run `/playbook:implement` when ready. If a project store is present at `~/.config/playbook/memory/<owner>/<repo>/`, persist the accepted decisions there; otherwise skip.

## Adapting to Complexity

- **Simple change (1-2 files):** 2-4 questions in Step 3, 3-5 in Step 7. Don't over-interview a trivial change.
- **Medium feature (3-10 files):** more in Step 3 as the idea needs; 8-15 in Step 7, focused on integration points and edge cases.
- **Large feature (new module, architecture):** the fullest Step 3 discovery; 15-25 in Step 7, covering dependencies, rollout, migration.
- **Bug fix:** replace design questions with diagnosis: reproduction steps, error messages, root-cause hypotheses. Still provide recommended answers.
