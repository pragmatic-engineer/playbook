# Root Cause Tracing

Bugs often show up deep in the call stack: a file written to the wrong path, a database opened with a bad handle, a git command run in the wrong directory. The instinct is to fix where the error appears. That fixes a symptom.

**Core principle: trace backward through the call chain to the original trigger, then fix at the source.**

## When to use

- The error happens deep in execution, not at the entry point.
- The stack trace shows a long call chain.
- It is unclear where the bad value came from.
- You need to find which test or code path triggers the problem.

## The tracing process

1. **Observe the symptom.** The exact error and where it surfaces (`git init failed in packages/core`).
2. **Find the immediate cause.** What line directly triggers it? (`exec('git', ['init'], { cwd: projectDir })`.)
3. **Ask what called it.** Walk up one frame: what function passed this value in?
4. **Keep tracing up.** What value was passed at each level? (`projectDir` was an empty string; an empty `cwd` resolves to the process working directory, which is the source tree.)
5. **Find the original trigger.** Where did the bad value first appear? (A test read a temp path before setup populated it, so it was empty.)

Fix at that source. Then consider `defense-in-depth.md` so a bad value cannot travel that far again.

## When you cannot trace by reading

Add temporary instrumentation just before the dangerous operation, log the value and the call stack, run once, and read it back:

```js
// Log BEFORE the operation, not after it fails.
function gitInit(directory) {
  console.error('DEBUG git init:', {
    directory,
    cwd: process.cwd(),
    env: process.env.NODE_ENV,
    stack: new Error().stack,
  });
  return exec('git', ['init'], { cwd: directory });
}
```

```bash
npm test 2>&1 | grep 'DEBUG git init'
```

Tips that apply in any stack:

- Log to stderr (or the equivalent). A test framework may suppress the normal logger.
- Log before the risky call, not after the failure.
- Include the value, the working directory, relevant environment variables, and a captured stack (`new Error().stack` in JS, `traceback.format_stack()` in Python, `runtime.Stack` in Go, `RUST_BACKTRACE=1` in Rust).
- Look for the test file name and line in the stack, and for a repeated parameter across failures.

## Finding which test pollutes shared state

If something appears during a test run but you do not know which test caused it, bisect: run the suite in halves (or one file at a time) until the smallest set that reproduces it is left. Most runners support running a single file or a name filter; use that to narrow down, then trace inside the offending test.

## Key principle

Never fix only where the error appears. Trace back to the original trigger and fix there. A symptom fix leaves the real cause free to break something else.
