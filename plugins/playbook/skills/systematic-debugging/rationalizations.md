# Common rationalizations

Read this when tempted to skip a phase of systematic debugging.

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

## More red flags

- "Skip the test, I will verify by hand."
- "I do not fully understand this, but this might work."
- "The pattern says X, but I will adapt it differently."
- Each fix reveals a new problem somewhere else.

## User redirections and what they mean

- "Is that not happening?" You assumed without verifying.
- "Will it show us...?" You should have added evidence gathering.
- "Stop guessing." You are proposing fixes without understanding.
- "Think harder about this." Question the fundamentals, not the symptom.
- "Are we stuck?" Your approach is not working.

All of these mean: stop, return to Phase 1.
