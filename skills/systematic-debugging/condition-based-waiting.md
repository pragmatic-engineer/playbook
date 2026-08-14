# Condition-Based Waiting

Flaky tests often guess at timing with a fixed delay. That creates a race: the test passes on a fast machine and fails under load or in CI.

**Core principle: wait for the actual condition you care about, not a guess about how long it takes.**

## When to use

- A test has an arbitrary delay (`setTimeout`, `sleep`, `time.sleep`).
- A test is flaky: it passes sometimes and fails under load.
- A test times out when run in parallel.
- You are waiting for an async operation to finish.

Do not use it when you are testing timing behaviour itself (debounce, throttle intervals). There, always document why the delay is needed.

## The pattern

Replace "wait a fixed time, then check" with "poll the condition until it is true, up to a timeout":

```js
// Guessing at timing (flaky):
await new Promise(r => setTimeout(r, 50));
expect(getResult()).toBeDefined();

// Waiting for the condition (stable):
await waitFor(() => getResult() !== undefined);
expect(getResult()).toBeDefined();
```

A generic poller (adapt to your stack; JS/TS teams should prefer the framework helper, e.g. Testing Library `waitFor` or Vitest `vi.waitFor`, per `playbook:engineering-standards-javascript`):

```js
async function waitFor(condition, description, timeoutMs = 5000) {
  const start = Date.now();
  while (true) {
    const result = condition();
    if (result) return result;
    if (Date.now() - start > timeoutMs) {
      throw new Error(`Timeout waiting for ${description} after ${timeoutMs}ms`);
    }
    await new Promise(r => setTimeout(r, 10)); // poll every 10ms
  }
}
```

Common targets: wait for an event to arrive, a state to reach a value, a count to reach N, a file to exist, or a compound condition.

## Common mistakes

| Mistake | Fix |
|---------|-----|
| Polling every 1ms | Poll every ~10ms; tight loops waste CPU |
| No timeout (loops forever) | Always set a timeout with a clear error |
| Caching state before the loop | Read the value inside the loop so it stays fresh |

## When a fixed delay is correct

Sometimes you genuinely need to wait for timed behaviour (a tool that ticks every 100ms, and you need two ticks). Then: first wait for the triggering condition, then wait a delay based on the known interval, with a comment explaining the number. A fixed delay is fine when it is grounded in a known timing, not a guess.
