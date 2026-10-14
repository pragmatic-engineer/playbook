# Defense in Depth

When you fix a bug caused by a bad value, adding one check feels like enough. But a single check gets bypassed by a different code path, a later refactor, or a mock.

**Core principle: validate at every layer the data passes through, so the bug becomes structurally impossible.**

One check says "we fixed the bug." Checks at each layer say "we made the bug impossible." Different layers catch different cases: entry validation catches most, business logic catches edge cases, environment guards catch context-specific danger, and logging helps when the others miss.

## The layers

1. **Entry-point validation.** Reject obviously bad input at the boundary (empty, missing, wrong type, does not exist). Fail with a clear message naming the bad value.
2. **Business-logic validation.** Confirm the data makes sense for this specific operation, not only that it is present.
3. **Environment guards.** Refuse dangerous operations in the wrong context. Example: during tests, refuse to run `git init` outside a temp directory, so a bad path can never touch the real tree.
4. **Debug instrumentation.** Log the value and a stack before the risky operation, so if all else fails you have the forensic trail.

## Applying it

1. **Trace the data flow.** Where does the bad value start, and where is it used? (See `root-cause-tracing.md`.)
2. **Map the checkpoints.** List every point the data passes through.
3. **Add a check at each layer.** Entry, business logic, environment, logging.
4. **Test each layer.** Try to bypass layer 1 and confirm layer 2 still catches it. Each layer should hold on its own.

## Why all the layers

In real sessions each layer catches something the others miss: a different code path skips entry validation, a mock skips the business-logic check, a platform edge case needs the environment guard, and the logging is what reveals structural misuse. Do not stop at one validation point.

Balance this against `KISS`/`YAGNI`: add layers where a bad value would cause real damage (data loss, writes outside a sandbox, corrupted state), not on every trivial helper.
