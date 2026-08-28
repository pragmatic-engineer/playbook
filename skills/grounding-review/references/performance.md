# Performance

- N+1 query patterns or unbounded result sets.
- Large object copies or allocations in hot paths.
- Blocking I/O on async threads or event loops.
- Polling without backoff or jitter.
- Unnecessary serialisation round-trips.
- Bypassing existing caches, indexes, or helper functions that already solve the problem.
