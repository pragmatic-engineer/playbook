---
name: engineering-standards
description: Use when working on pull requests, planning a testing approach, or thinking about deployment under the team engineering standards.
---

# Engineering Standards

Team engineering standards for pull requests, testing, design, and deployment. RFC 2119 keywords (MUST, SHOULD, etc.) carry their standard meanings.

## Commits

### Self-review before committing

Read the staged diff before you commit it. `/playbook:commit-and-push` runs in a forked
context on the `git` agent, so it stages, writes a message, and pushes without
ever judging the change. You are the only reader who sees it first.

Run `git diff --cached` and check:

- Every staged file belongs in this commit. Nothing swept in by `-A`.
- One concern. If the message needs an "and", split the commit.
- No leftover scaffolding: debug output, commented-out code, a stray test file.
- Nothing secret: keys, tokens, `.env` files, real customer data.
- The change does what you set out to do, and you can say why in one line.

`hooks/precommit-check.sh` covers the mechanical half of this (secret-shaped
filenames, debug leftovers in added lines, oversized commits) and warns without
blocking. It cannot tell whether the change is correct or whether it belongs in
one commit. That part is yours.

## Pull Requests

### Readiness

A PR MUST meet these criteria before requesting review:

- CI is fully passing.
- Automated tests are included for the change.
- The author has self-reviewed the diff.
- The description explains the "why", not just the "what".
- Any intentional technical debt is documented in the description with justification.

### Size

- **Soft limit (500):** PRs SHOULD be under 500 changed lines (additions + deletions).
- **Enforced limit (1000):** a PR over 1000 changed lines MUST carry explicit justification; without it, split before requesting review.
- **Hard limit (1500):** PRs MUST NOT exceed 1500 changed lines. There is no override; split the work.
- Large changes SHOULD be split into logical units (e.g., one PR for the data layer, another for the service layer).
- One concern per PR. A refactor, a feature, and its docs are separate PRs, not one. Unrelated changes in a single diff force the reviewer to track several things at once.
- Treat the diff as an interface the reviewer reads (adapted from Krug's "Don't Make Me Think"): the smaller and more focused it is, the less they have to figure out. When work is large, ship a sequence of small PRs.

### Review Comments

- Review comments SHOULD use Conventional Comments format: a bare label per comment (`blocking`, `issue`, `suggestion`, `nitpick`, `question`). `blocking` replaces `issue` for a finding that must be fixed before merge.
- Blocking comments are for issues that MUST be resolved before merge: quality gaps, undocumented tech debt, security or data integrity concerns, missing tests, or architectural issues affecting future maintenance.
- Blocking comments SHOULD be treated as opportunities for discussion, not hard stops. Valid resolutions: fix immediately, create a follow-up ticket, document the limitation, agree the concern is out of scope, or escalate to a design discussion.
- Non-blocking feedback SHOULD be framed as suggestions: "one option here..." or "worth considering...".
- Reviewers SHOULD prioritise re-reviews and PRs closest to completion over new PRs (pull work to the right).

### Review Turnaround

- Initial reviews SHOULD be completed within 24 hours.
- Re-reviews (after author addresses feedback) SHOULD be completed within 4 hours.

## Automated Testing

### Test Types

| Type | What it tests | External dependencies |
|---|---|---|
| Unit tests | Business logic of a specific function or class | None |
| Integration tests | Service and data layer behaviour | Database |
| API/container tests | API behaviour with real database, stubbed externals | API + database |
| E2E tests | Full user flows | Full environment |

### Requirements

- All code changes MUST include appropriate automated tests.
- Unit tests SHOULD focus on error handling and edge cases that are difficult to exercise in higher-level tests.
- Integration tests MUST be isolated: each test creates its own data and MUST NOT rely on shared seed data.
- Code SHOULD be structured so it could achieve high unit test coverage without requiring refactoring, even if 100% coverage is not required.
- When implementing new functionality, TDD (red/green/refactor) SHOULD be used: write a failing test first, then minimal implementation to pass, then refactor. This prevents tests that are tautologically coupled to the implementation.
- Coverage thresholds MUST NOT be decreased. If changes increase coverage, thresholds SHOULD be raised.
- CI MUST pass before a PR is eligible for review.

### Mocking

- Mock only where necessary, as close to the application boundary as possible.
- Prefer dependency injection over global mocking for easier-to-understand tests.
- For domain services: spy on the interface and verify call parameters; do NOT mock the database tables owned by another service.
- Inject an external client as a constructor or function parameter rather than constructing it inside the function under test. A function that builds its own client can't be mocked without reaching into its internals.
- Prefer a narrow, per-operation interface (one method per operation) over one generic call that branches on an argument. A mock over a narrow interface has a fixed shape, and a test visibly declares which operations it touches.

## Manual Testing

- Changes MUST be manually verified before merge, as a complement to automated testing.
- If a scenario can be covered by an automated test, it SHOULD be.
- Database migration PRs: the query SHOULD be verified by converting it to a SELECT and running it against production data first.

## Incremental Delivery

- Work MUST be delivered incrementally, not in a single large release.
- Feature flags MUST be used (where available) to enable safe, incremental rollout.
- Each work unit SHOULD be independently deliverable and testable.

## Deployment

- Deployment path: merge to main -> automated post-merge checks -> manual approval -> deploy.
- Engineers MUST monitor their changes after deployment for errors, performance regressions, and unexpected behaviour.
