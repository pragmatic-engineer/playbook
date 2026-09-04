---
name: systematic-debugging
description: Use when encountering any bug, test failure, or unexpected behaviour, before proposing a fix.
---

# Systematic Debugging

Random fixes waste time and add new bugs. A quick patch hides the real problem, and it comes back.

**Core rule: find the root cause before you touch a fix.** A change that makes the symptom go away without explaining why it happened is not a fix.

## The Iron Law

```
NO FIX WITHOUT A ROOT-CAUSE INVESTIGATION FIRST
```

If you have not finished Phase 1, you do not get to propose a fix. This holds even when the bug looks trivial, even under time pressure, even when someone wants it fixed right now. Systematic is faster than guess-and-check.

## When to Use

Any technical issue: test failures, production bugs, unexpected behaviour, performance problems, build failures, flaky tests, integration issues.

Reach for it hardest when the pull toward guessing is strongest:

- Under time pressure (an emergency makes guessing tempting).
- "One quick fix" looks obvious.
- You have already tried a fix or two and they did not hold.
- A previous fix did not work.
- You do not fully understand what is going on.

Do not skip it because the issue "seems simple" (simple bugs have root causes too) or because you are in a hurry (rushing guarantees rework).

## The Four Phases

Finish each phase before you move to the next.

### Phase 1: Find the root cause

Before attempting any fix:

1. **Read the error carefully.** Do not skip past errors or warnings; they often hold the answer. Read the whole stack trace. Note line numbers, file paths, error codes.
2. **Reproduce it.** Can you trigger it reliably? What are the exact steps? Every time, or intermittent? If you cannot reproduce it, gather more data instead of guessing.
3. **Check recent changes.** What changed that could cause this? Look at `git diff` and recent commits, new dependencies, config changes, environment differences.
4. **Gather evidence at component boundaries.** When the system has more than one component (CI to build to signing, API to service to database), add temporary logging at each boundary: what data enters, what data exits, whether config and environment propagate. Run once to see WHERE it breaks, then investigate that component. Do not propose fixes before you know which layer fails.
5. **Trace the data flow backward.** When the error is deep in the call stack, trace back to where the bad value started. Fix at the source, not at the symptom. See `root-cause-tracing.md` in this directory.

### Phase 2: Analyse the pattern

Find the pattern before fixing:

1. **Find working examples.** Locate similar code in the same codebase that works. What is different about the broken path?
2. **Read the reference completely.** If you are following a pattern or library, read the reference implementation end to end. Do not skim; partial understanding causes bugs.
3. **List every difference.** Between working and broken, note every difference, however small. Do not assume "that cannot matter."
4. **Understand the dependencies.** What else does this need: other components, settings, config, environment? What does it assume?

### Phase 3: Hypothesis and test

Use the scientific method:

1. **State one hypothesis, in writing.** "I think X is the root cause because Y." Be specific, not vague.
2. **Test it with the smallest change.** One variable at a time. Do not fix several things at once.
3. **Verify before continuing.** Worked? Go to Phase 4. Did not work? Form a NEW hypothesis; do not stack another fix on top.
4. **When you do not know, say so.** "I do not understand X." Do not pretend. Ask the user, or research more.

### Phase 4: Fix the root cause

Fix the cause, not the symptom:

1. **Write a failing test first.** The simplest reproduction, automated where a framework exists (a one-off script if not). You must have it before the fix. Follow `playbook:engineering-standards` (red/green/refactor); for JS/TS use `playbook:engineering-standards-javascript`.
2. **Make one fix.** Address the root cause. One change. No "while I am here" improvements, no bundled refactor.
3. **Verify.** The new test passes, no other test broke, and the issue is actually gone.
4. **If the fix does not work, stop and count.** How many fixes have you tried? Under 3: return to Phase 1 with the new information. **3 or more: stop and question the architecture (below).** Do not attempt fix number 4 without that discussion.
5. **After 3 failed fixes, question the design.** If each fix reveals new shared state or coupling somewhere else, or each fix needs "massive refactoring," or each fix creates a new symptom, the pattern itself may be wrong. Ask the user before more fixes: is this pattern sound, or are we sticking with it out of inertia? This is a wrong-architecture signal, not a failed hypothesis.

Consider adding validation at several layers so the bug becomes impossible, not merely fixed. See `defense-in-depth.md`.

## Red Flags: stop and return to Phase 1

If you catch yourself thinking any of these, stop:

- "Quick fix now, investigate later."
- "Try changing X and see if it works."
- "Add several changes, then run the tests."
- "Skip the test, I will verify by hand."
- "It is probably X, let me fix that."
- "I do not fully understand this, but this might work."
- "The pattern says X, but I will adapt it differently."
- Listing fixes before tracing the data flow.
- "One more fix attempt" after two or more failures.
- Each fix reveals a new problem somewhere else.

All of these mean: stop, return to Phase 1. Three or more failed fixes means question the architecture.

## Signals from the user that you are doing it wrong

Watch for these redirections:

- "Is that not happening?" You assumed without verifying.
- "Will it show us...?" You should have added evidence gathering.
- "Stop guessing." You are proposing fixes without understanding.
- "Think harder about this." Question the fundamentals, not the symptom.
- "Are we stuck?" (frustrated). Your approach is not working.

When you see these: stop, return to Phase 1.

## Common Rationalizations

| Excuse | Reality |
|--------|---------|
| "Issue is simple, no need for process" | Simple issues have root causes too. The process is fast for simple bugs. |
| "Emergency, no time for process" | Systematic debugging is faster than guess-and-check thrashing. |
| "Try this first, investigate later" | The first fix sets the pattern. Do it right from the start. |
| "I will write the test after the fix works" | Untested fixes do not stick. A test first proves it. |
| "Several fixes at once saves time" | You cannot tell what worked, and it causes new bugs. |
| "The reference is long, I will adapt the gist" | Partial understanding guarantees bugs. Read it fully. |
| "I see the problem, let me fix it" | Seeing the symptom is not understanding the cause. |
| "One more fix attempt" (after 2+) | 3+ failures means an architecture problem. Question the pattern, do not fix again. |

## When Investigation Finds No Root Cause

If the investigation genuinely shows the issue is environmental, timing-dependent, or external:

1. You have completed the process.
2. Document what you investigated.
3. Add appropriate handling (retry, timeout, clear error message).
4. Add logging or monitoring for next time.

But 95% of "no root cause" cases are incomplete investigation. Be honest about which one this is.

## Quick Reference

| Phase | Do | Done when |
|-------|-----|-----------|
| 1. Root cause | Read errors, reproduce, check changes, gather evidence, trace backward | You understand what and why |
| 2. Pattern | Find working examples, read the reference, list differences | You can name the difference |
| 3. Hypothesis | State one theory, test minimally | Confirmed, or a new hypothesis |
| 4. Fix | Failing test, one fix, verify | Bug gone, tests green |

## Supporting Techniques (this directory)

- `root-cause-tracing.md`: trace a bug backward through the call chain to the original trigger.
- `defense-in-depth.md`: after finding the cause, add validation at each layer so the bug cannot recur.
- `condition-based-waiting.md`: replace arbitrary timeouts in tests with polling on the real condition.

## Related Disciplines

- `playbook:engineering-standards`: the failing test in Phase 4 (red/green/refactor) and the testing rules.
- `playbook:engineering-standards-javascript`: JS/TS specifics (test structure, mocking, the `waitFor` helper).
- `playbook:grounding-research`: verify claims against the real code before you assert them; cite `file:line`.

## Memory

When the investigation surfaces a durable root cause or gotcha (true regardless of this one bug), and a project memory store exists at `~/.claude/memory/<owner>/<repo>/` (derive `<owner>/<repo>` from `git remote get-url origin`), save it as a project memory fact with `anchors:` to the files involved. This turns a hard debugging session into a fact the next run reads first. If there is no project store, skip it.
