# Examples

**Bad** (robotic, paraphrases reviewer, trailing hedge):

> Agreed, there is no ordering guarantee in the schema. For now the heuristic is the best we can do without a timestamp field. If we see incorrect results in practice, we can revisit with the client to add ordering metadata to their export.

**Good** (human: straight to substance):

> No ordering guarantee from the schema, yeah. Heuristic's the best option without a timestamp.

**Bad** (over-explains, filler closing):

> Valid edge case. In practice '' and null are distinct in the DB (Postgres stores them differently), treating them as different is the safer default. If we normalised with (v || null), we'd also swallow legitimate changes from '' to a real notes value when the user intentionally sets notes to empty string first. Leaving as-is for now, but good to be aware of.

**Good** (short, honest):

> Done, added (v || null) normalisation for notes and orderNotes.

**Bad** (praise opener + restate finding + describe fix):

> You're right, that branch was unreachable as written. Confirmed against the prod handler: the upstream API returns the failure flag with the result detail populated, not the success flag. Fixed the wrapper to thread the detail through the failure path and updated the unit test fixture to match the real shape.

**Good** (terse, lets the diff speak):

> Yeah, thanks for that. Fixed.

**Review finding** (bad: teaches, recaps the diff, three parts):

> issue: This loads all rows into memory before filtering. As you know, with a large result set that holds the whole table in the heap, and since Node caps the heap, big tenants will OOM. Consider streaming or filtering in the query.

**Review finding** (good: the problem, then the failure):

> issue: this loads every row before filtering, so a large tenant OOMs the process. Push the filter into the query.

**Review finding, blocking label** (bad: argues priority the label already carries):

> blocking: the retry loop has no max attempt count, and since a stuck dependency means this runs forever, it's worth capping now while the change is small.

**Review finding, blocking label** (good: the label already says urgent, the sentence only needs the defect):

> blocking: the retry loop has no max attempt count, so a stuck dependency retries forever.

**Review finding, complex fix** (bad: hands over a full working implementation):

> blocking: this retries without a backoff, so a flaky dependency gets hammered. Here's the fix:
> ```js
> async function withRetry(fn, { attempts = 3, baseMs = 200 } = {}) {
>   for (let i = 0; i < attempts; i++) {
>     try { return await fn(); }
>     catch (e) { if (i === attempts - 1) throw e; await sleep(baseMs * 2 ** i); }
>   }
> }
> ```

**Review finding, complex fix** (good: states the problem, sketches the idea in pseudocode, leaves the implementation to the author):

> blocking: this retries without a backoff, so a flaky dependency gets hammered. Something like: retry N times, sleep `baseDelay * 2^attempt` between each, give up after the last one.

**More good examples:**

- "Behind a feature flag for now. If other tenants hit it we can broaden the scope."
- "Done, added the null check."
- "Intentional. They're distinct in Postgres so this covers both cases."
- "Fixed."
- "Sorted."
