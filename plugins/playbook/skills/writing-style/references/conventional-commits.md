# Conventional Commits

Read this before writing a commit message or a PR title. Squash merge turns the PR title into the commit message, so a title follows every rule here. The voice, banned-word and dash rules in `SKILL.md` still apply.

## Contents

- Format
- Types
- Scope
- Breaking changes
- The description
- When a change fits two types
- Common mistakes
- Good and bad
- Credit

## Format

```
<type>[optional scope][!]: <description>

[optional body]

[optional footers]
```

- The first line is the header. Keep it to 72 characters at most, prefix included.
- A blank line separates the header, the body and the footers.
- The body explains why, only when the header does not. One or two sentences, or up to 3 short bullets.
- Footers are `Token: value` or `Token #value` lines, such as `Refs: #42`, `Closes #42` or `BREAKING CHANGE: ...`.
- Never type `Signed-off-by` by hand. Commit with `git commit --signoff` (or the commit tool) and git adds it.

## Types

Pick the type from what the change does for the user, and from what the files are.

| Type | Use it for | Example |
| --- | --- | --- |
| `feat` | New capability a user or caller can see | `feat(PROJ-123): add retry to the sync job` |
| `fix` | A bug fixed | `fix(auth): reject an expired refresh token` |
| `docs` | Documentation files only | `docs(readme): explain the install step` |
| `style` | Formatting only, no change in meaning | `style(api): sort the imports` |
| `refactor` | Restructure code and keep behavior | `refactor(cache): split the loader into two functions` |
| `perf` | Same behavior, faster or lighter | `perf(search): cache the parsed index` |
| `test` | Test files only | `test(billing): cover the proration edge case` |
| `build` | Build system or dependency bump | `build(deps): bump serde to 1.0.210` |
| `ci` | CI configuration files only | `ci: run the lint job on pull requests` |
| `chore` | Upkeep that fits no other type | `chore: remove the unused fixtures` |
| `revert` | Undo an earlier commit | `revert: undo the retry change in the sync job` |

The binary accepts exactly these 11 lowercase types in a commit message and a PR title, and refuses any other. When a new commit is made, a second check refuses a type that contradicts the staged files: only docs files cannot be `feat`, `fix`, `refactor` or `perf`; only test files cannot be `feat`, `refactor`, `perf` or `docs`; only CI files cannot be `feat` or `docs`; only lock files, or manifests next to a lock file, cannot be `feat`, `fix`, `refactor` or `docs`. That second check does not run on a PR title or an amend, so apply the same rule by hand there.

## Scope

- Use at most one scope, in parentheses, right after the type.
- With no ticket, the scope is a lowercase module or area name, such as `auth` or `cache`. Leave it out when no single area fits.
- When the work belongs to a Jira or Linear ticket, the scope is the ticket key: `feat(PROJ-123): add retry to the sync job`. This MUST hold for every commit and the PR title on a stacked PR set. The commit and PR tools refuse a stacked commit or PR that lacks a key they found, and nothing checks it without them. Do not invent a key. With none known, use a module name or no scope.
- A GitHub issue is not a scope. Put it in the body or a footer: `Closes #42` when the change finishes it, `Refs #42` when it only relates.

## Breaking changes

A change that breaks existing callers MUST say so in two places:

- A `!` right before the colon: `feat(api)!: drop the v1 endpoints`.
- A footer that names the migration step:

```
BREAKING CHANGE: the v1 endpoints are gone. Move clients to /v2/orders.
```

## The description

These rules come from the binary checks and the writing rules:

- Imperative mood: "add", not "added" or "adds".
- Start in lowercase and end with no period.
- State the effect of the change, not a list of files.
- No em dashes or en dashes anywhere in the message.
- No AI or tool attribution, such as a `Co-Authored-By` for an AI or a "Generated with" line.
- The whole header is 72 characters at most. The PR tooling refuses a longer title.

## When a change fits two types

The effect a user sees wins.

- `fix` over `refactor`: if the restructure also stops a bug, it is `fix`.
- `feat` over `fix`: if the change adds a capability, even while fixing something, it is `feat`.
- `refactor` is only for a change with no visible effect.
- Tests added next to a feature belong in the `feat` commit. A commit with only tests is `test`.
- If a change needs two types, it is two commits.

## Common mistakes

- `feat` for a docs, test or CI change. The type check refuses it.
- `fix` for a dependency bump or a cleanup. A bump is `build` or `chore`, a cleanup is `chore` or `refactor`.
- A vague description such as `update stuff` or `fix things`.
- Several unrelated changes in one commit. Split them.
- A ticket-bound change with no ticket scope.
- Past tense: `added retry` instead of `add retry`.
- A GitHub issue number in the scope.
- A trailing period, or a capital first letter in the description.

## Good and bad

| Bad | Good |
| --- | --- |
| `fixed the login bug.` | `fix(auth): reject an expired refresh token` |
| `feat: update docs` | `docs(readme): explain the install step` |
| `fix: bump serde` | `build(deps): bump serde to 1.0.210` |
| `feat(#42): add export` | `feat(export): add CSV export` with `Closes #42` in the footer |
| `feat: Added retry.` | `feat(PROJ-123): add retry to the sync job` |
| `chore: stuff` | `chore: remove the unused fixtures` |
| `refactor: stop the crash on empty input` | `fix(parser): stop the crash on empty input` |
| `feat: drop v1 endpoints` | `feat(api)!: drop the v1 endpoints`, with a `BREAKING CHANGE:` footer |

## Credit

Adapted from Conventional Commits 1.0.0 (https://www.conventionalcommits.org/en/v1.0.0/), licensed CC BY 3.0 (https://creativecommons.org/licenses/by/3.0/). The ticket scope, the two breaking-change markers and the description casing are house rules, not part of the spec.
