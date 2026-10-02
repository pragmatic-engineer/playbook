---
name: session-handoff
description: Use at the end of a working session, before /clear or context compaction, or when the user asks to wrap up or summarize for next time.
---

# Session Handoff

## When to Use

Invoke at the end of a working session, before `/clear`, before context compaction, or any time the user asks to "wrap up" or "summarize for next time". The output is written for a reader who has not seen this conversation: a future Claude session, a teammate, or the user themselves a week from now.

## What to Produce

A single document with four sections in this exact order. Skip a section that has nothing real to put in it; do not pad.

### 1. Where we are now

Observable facts only. No interpretation, no narration.

- Branch, current commit (short SHA + subject), working tree state (clean or dirty file list)
- Pipeline status: lint / typecheck / test counts / build (one line each, last-known result)
- Anything running in the background, any process the user needs to know is live
- Last verified behavior (e.g. "binary builds at 0.2.0-dev, prints version cleanly")

If the session did not touch a repo, replace this with the equivalent "current state" facts for the work that was happening.

### 2. Decisions made and WHY

The choices that survived this session and the reason each one was made. One line per decision. The WHY is mandatory: a decision without its reason rots into folklore.

Rules:
- Capture the chosen path, not the alternatives explored. Future readers do not need the deliberation; they need the chosen path and the reasoning so they can revisit it later if the assumption breaks.
- If a decision was forced by an external constraint (a library limitation, a registry reality, a plan mandate, a user directive), name the constraint inline.
- If a decision was a deferral ("we punted X to v0.3 because Y"), record it here, not in next steps. Deferrals are decisions.
- A reversal counts: if you changed direction mid-session, the final decision is what gets recorded.

### 3. Next steps

Only if there is concrete remaining work. Ordered by what should happen first. Each item: the action plus why-now or why-next.

If the session ended at a clean stopping point, write the single line `Clean stop. No pending work.` and skip the list. Do not invent next steps to look thorough.

Do not list speculative future enhancements that were not actually discussed.

### 4. Open questions / known gaps

Anything ambiguous, deferred, or risky that future work has to resolve. Each item: the gap plus which decision or task surfaced it. If nothing is genuinely open, omit the section entirely.

This is the section where "we did not verify X" or "Y depends on Z which is not yet checked" gets recorded honestly.

## Style

- Decisions and facts only. No tool narration ("I read", "I dispatched", "I searched"). No exploration recap. The reader does not care how you investigated, only what is true now.
- Concrete identifiers. Name commits with short SHAs, files with `path:line` references, versions as exact strings, dates as absolute ISO. "We bumped TypeScript" is wrong; "TS 5.5.0 -> 5.9.3 because 5.5.0 was never released stable" is right.
- Tight. Each section under ~10 lines unless the session genuinely covered multiple subsystems. The whole document should fit on one screen for a single-thread session.
- No emojis. No em dashes or en dashes: use commas, colons, or periods.
- No preamble ("Here is the handoff..."). No closing summary or trailing "Let me know if you need anything else". The document ends where it ends.
- A handoff that requires the source conversation to be understood is a failure of the skill. If you cannot make a point stand alone, drop it.

## Where to Write It

Print the handoff to the conversation as plain Markdown, then save it for the next session in one Bash call by piping the same document to `playbook handoff save`:

```bash
playbook handoff save <<'HANDOFF_EOF'
<the handoff document>
HANDOFF_EOF
```

The command keys the file by the current directory, so a later session started in that directory loads it on its own after `/clear`. It prints the saved path. It refuses empty input. Pass `--dir <path>` to save for a directory other than the current one.

If the user passes a path as an argument (e.g. `/session-handoff docs/handoffs/2026-06-22.md`), additionally write the document to that file. Create parent directories as needed. Confirm the written path in one line after the document. A bare filename with no directory goes in the current working directory. Never write to a path that was not explicitly provided; this rule governs the user-facing path, not `playbook handoff save`.

End with one line: run `/clear`, and the next session loads the handoff automatically, or run `/playbook:session-start` to load it on demand.

## What NOT to Include

- The chronological flow of the session ("First we did A, then B, then C").
- Subagent dispatches, tool calls, files read, searches run.
- Praise, sentiment, or framing language about the session.
- Speculation about why an earlier session behaved the way it did.
- Anything that requires reading the prior conversation to make sense.
- Memory pointers, ledger paths, or other artifacts unless a next-step depends on them.

## Output Template

```
# Session Handoff - <one-line topic>

## Where we are now
- <fact>
- <fact>

## Decisions made
- <chosen path>. Why: <reason>.
- <chosen path>. Why: <reason>.

## Next steps
1. <action>. Why now: <reason>.
2. <action>. Why next: <reason>.

(or: "Clean stop. No pending work.")

## Open questions
- <gap>. Surfaced by: <decision or task>.
```

The topic line is one short phrase summarizing what the session was about, not a generic "Session handoff for 2026-06-22". Specific over generic: "v0.2.0 monorepo restructure complete" beats "Coding session".
