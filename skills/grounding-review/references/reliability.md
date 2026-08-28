# Reliability

- Missing error handling on I/O, network calls, or external service interactions.
- Silent catch blocks that swallow errors without logging or re-raising.
- Unsafe retry logic (no backoff, no idempotency, no circuit breaker).
- Unverified assumptions about external API behaviour. Claims about how a third-party API behaves MUST be verified against its documentation. "They likely send a Retry-After header" is not acceptable without a doc reference.
- Logging gaps that would make production incidents harder to diagnose.
- Resource leaks (connections, file handles, timers not cleaned up).
- Don't stop at the function that skips error handling: trace up the call chain for a boundary that catches it. A repository or DB-access call is a boundary too, same weight as an API route taking external input; it's fine if it doesn't handle an error itself as long as the layer above it (the service) does. Flag it only when no layer in the chain handles it.
- In TypeScript, external or untyped data (request bodies, config, third-party responses, DB rows) cast with `as` instead of parsed with a runtime schema validator (Zod, Valibot, or whatever the project already uses). A cast is a compile-time assertion only; nothing checks it at runtime, so malformed data flows straight through.
