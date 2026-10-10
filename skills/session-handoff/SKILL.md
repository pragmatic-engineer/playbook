---
name: session-handoff
description: Writes a four-part handoff (where we are, decisions, next steps, open questions) for the next session. Use when wrapping up, before /clear or compaction, or when asked to summarize for next time.
---

# Session Handoff

Use at the end of a session, before `/clear` or compaction, or when the user asks to "wrap up" or "summarize for next time". Write for a reader who has not seen this conversation: a future session, a teammate, or the user in a week. A handoff that needs the source conversation to be understood has failed.

## Sections

Four sections in this order. Skip one with nothing real in it; do not pad.

1. **Where we are now.** Observable facts only: branch, current commit (short SHA and subject), working tree state, last-known lint, typecheck, test and build results, anything running in the background, last verified behavior. If no repo was touched, give the equivalent current-state facts.
2. **Decisions made.** One line per surviving decision, with its WHY (mandatory; a decision without a reason rots into folklore). Record the chosen path, not the alternatives. Name any forcing constraint inline. Deferrals and reversals are decisions and go here, not in next steps.
3. **Next steps.** Only concrete remaining work, in order, each with why-now or why-next. At a clean stopping point write the single line `Clean stop. No pending work.` Never list speculative enhancements that were not discussed.
4. **Open questions.** Ambiguous, deferred or risky items, each with the decision or task that surfaced it. This is where "we did not verify X" belongs. Omit the section if nothing is open.

## Style

- Decisions and facts only. No tool narration, exploration recap, chronology, praise, or speculation about earlier sessions.
- Concrete identifiers: short SHAs, `path:line`, exact versions, absolute ISO dates. "TS 5.5.0 -> 5.9.3 because 5.5.0 was never released stable", not "we bumped TypeScript".
- Tight: each section under about 10 lines unless the session spanned several subsystems.
- No emojis, no em or en dashes, no preamble, no closing offer.
- No memory pointers or ledger paths unless a next step depends on them.

## Where to write it

Print the handoff as plain Markdown, then save it in one Bash call by piping the same document to `playbook handoff save`:

```bash
playbook handoff save <<'HANDOFF_EOF'
<the handoff document>
HANDOFF_EOF
```

The file is keyed by the current directory, so a later session there loads it after `/clear`. The command prints the saved path and refuses empty input. `--dir <path>` saves for another directory.

If the user passes a path argument (e.g. `/playbook:session-handoff docs/handoffs/2026-06-22.md`), also write the document there, creating parent directories, and confirm the path in one line. A bare filename goes in the current working directory. Never write to a path the user did not provide; this governs the user-facing path, not `playbook handoff save`.

End with one line: run `/clear` and the next session loads the handoff automatically, or run `/playbook:session-start` to load it on demand.

## Template

```
# Session Handoff - <one-line topic>

## Where we are now
- <fact>

## Decisions made
- <chosen path>. Why: <reason>.

## Next steps
1. <action>. Why now: <reason>.
(or: "Clean stop. No pending work.")

## Open questions
- <gap>. Surfaced by: <decision or task>.
```

The topic line is specific ("v0.2.0 monorepo restructure complete"), never generic ("Coding session").
