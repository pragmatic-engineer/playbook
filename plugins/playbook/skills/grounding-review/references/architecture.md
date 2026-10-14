# Architecture

- Acknowledged future risks without a concrete mitigation plan (no follow-up ticket, no retention strategy, no capacity estimate).
- Unbounded growth patterns: tables, queues, caches, or logs that grow without expiry, partitioning, or archival.
- Missing capacity or scaling considerations.
- Trade-offs accepted without tracking the deferred work.
- Schema designs that preclude future requirements mentioned in the PR.
- Dependencies on concretions where established service interfaces exist for the same capability.
