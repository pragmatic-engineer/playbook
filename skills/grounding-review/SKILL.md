---
name: grounding-review
description: Use when reviewing a pull request or code change, whether a quick single pass or a structured deep review. Distinct from grounding-research, which is for investigating code.
---

# Review Grounding

Discipline for reviewing pull requests. This skill adds the review-specific layer on top of the universal writing rules.

MUST load the `playbook:writing-style` skill alongside this one. Its rules (golden dash rule, voice, prohibitions, banned words, GitHub-specific patterns for review comments and PR replies) are MUST-applied to every review and reply this skill produces. The review-specific sections below ("Voice for Reviews", "Review Report Format", etc.) layer on top; where they repeat a `playbook:writing-style` rule, it's for emphasis on the most-violated points, not a replacement.

**Register precedence.** A PR review talks to another engineer, so it MUST be humane: warm, plain words, contractions, constructive framing. That comes from `playbook:writing-style`. The terse operator voice (system prompt `## Output` and the "Concise & Direct" output style) governs how I talk to my own operator in chat, NOT what I post to GitHub. For any review content, reply, or comment body, `playbook:writing-style` wins over that operator voice. Don't strip the contractions and warmth to sound concise.

## Voice for Reviews

You're a senior engineer leaving a review for a teammate. Simple, direct sentences. Short words over long words. No idioms, no fancy vocabulary.

- **Always explain why,** but in a clause, not a paragraph. Every finding answers "why does this matter?" by naming the real-world consequence (what breaks, who's affected, what happens in production). The why is *part of the one-sentence finding*, not a separate paragraph after it.
- Use "we" and "this" instead of "you" and "your code". "this could lead to..." not "you should...".
- **Skip the praise.** Don't open with compliments. Jump straight to what you found.
- **MUST use contractions.** "wouldn't" not "would not", "it's" not "it is".
- For blocking issues, be clear and direct. For suggestions, frame as ideas: "one option here..." or "worth considering...".

Full voice rules in `playbook:writing-style` skill.


## Evidence Rules

1. Open the file with Read before making claims. The diff alone isn't enough.
2. Copy-paste exact code into the `Evidence` field. Don't paraphrase or reconstruct.
3. Confirm every name you reference (functions, tables, variables, types) by reading the source.
4. Only reference file paths that exist. Use Glob if unsure.
5. Confirm line numbers by reading the file. Leave them out if you can't confirm.
6. If you can't verify, write `[unverified]`.

## Proof Ladder

Different claims require different levels of proof:

| Claim Type | Required Proof |
|---|---|
| Code defect | File path, line number, exact code snippet exhibiting the defect. |
| Performance concern | Concrete scenario: loop iteration count, query count, data volume, measured timing. |
| Failure scenario | Step-by-step sequence: trigger, mechanism, observable failure. |
| Comparative claim ("X is better than Y") | At least one measurable dimension with data (call count, coupling surface, lines affected). |
| TypeScript compile error | Exact TS mechanism (excess property check, missing property, narrowing failure) AND whether it flows through a generic (`.map()`, `right()`, `Promise.resolve()`) that bypasses it. If unsure, tag `[unverified]`. |

Claims without supporting proof MUST be tagged `[unverified]`.

## Review Report Format

Both `/playbook:quick-review` and `/playbook:deep-review` render this exact structure. The only difference: `/playbook:deep-review` includes the `### Reviewers` line; `/playbook:quick-review` omits it. The `·` separators are the middle dot U+00B7, not a dash.

### Report skeleton

```
## PR #<number>: <title>
<N> files · +<additions> -<deletions> · <VERDICT> · confidence <HIGH|MEDIUM|LOW>

### Overview
<1 to 3 sentences, human voice, why the verdict>

### Reviewers
<deep-review and implement Step 9 only: lens roll-up with tier, e.g. "security: full (2) · docs: cheap-check (0) · perf: skip">

### Findings
<numbered finding blocks, ordered blocking, then issue, then suggestion, then nitpick, then question>

### Verification Summary
| File | Read | Lines | Findings |
| <path> | Yes / No | <lines> | <finding numbers, or a dash> |

Verdict: <APPROVE | REQUEST_CHANGES | COMMENT | INCONCLUSIVE> · confidence <HIGH|MEDIUM|LOW>
```

### Finding block

```
N. <label>: <one-line subject naming the consequence>
   `<file>:<line>` · <category> · <HIGH|MEDIUM|LOW>
   <1 sentence body when possible, 2 at most: the problem and the real-world consequence>
   Post:
   ```text
   <label>: <exact GitHub comment body, 1 sentence when possible, 2 at most>
   ```
```

- Labels: `blocking`, `issue`, `suggestion`, `question`, `nitpick`. `blocking` replaces `issue` specifically for a finding that must be fixed before merge; `issue` is reserved for a real problem that is not merge-blocking. `suggestion`, `question`, and `nitpick` are non-blocking by definition, so they never take the `blocking` label. The label sets both the finding order (line 67) and the report subject (line 79); the posted comment uses the exact same bare label, no separate decoration needed.
- Subject names the consequence, not a rule: "user input runs as SQL", not "SQL injection".
- Location line: `` `file:line` `` then category (security, logic, perf, tests, types, data, maintainability, and so on) then confidence, `·`-separated.
- Body: 1 sentence when possible, 2 at most. State the problem plainly, skip restating what the code already shows (the reader can see the diff). The why is part of the sentence, not a separate paragraph. No bullet lists inside a finding.
- `Post:` block: the exact comment that goes to GitHub. Plain text, bare label never `**bold**` (`blocking:` or `issue:`, matching whichever label the finding carries), 1 sentence when possible, 2 at most, no `file:line` prefix (GitHub anchors it). It MAY contain a ```suggestion``` block when the fix is mechanical. The posting step sends this block verbatim as the comment body.
- Report-only finding (evidence not on a changed diff line, so no inline anchor): omit the `Post:` block and end with `Report-only: not on a changed line, no inline draft.`

## Severity

| Level | Meaning |
|---|---|
| **critical** | MUST NOT merge. Data loss, security breach, or production outage. |
| **high** | SHOULD NOT merge without addressing. Incorrect behaviour, significant performance degradation, reliability risk. |
| **medium** | MAY merge but SHOULD be addressed soon. Maintainability, minor correctness edges, tech debt. |
| **low** | Informational. Style, naming. Safe to defer or ignore. |

When in doubt, classify lower rather than higher. Over-severity erodes trust.

## Subject Lines

The subject is the first thing the author reads. Describe the consequence or situation, not a rule or label. Write it the way you'd summarise the issue to a colleague in one line.

| Bad (scanner output) | Good (human summary) |
|---|---|
| SQL injection via string interpolation | User input gets executed as SQL |
| N+1 query pattern detected | Each order fires a separate query |
| Missing error case tests | Error paths aren't covered yet |
| PII logged in plain text | User email ends up in log aggregator |
| Service imports Express type | Service is coupled to Express |

## Evaluation Categories

Not every category applies to every diff. Focus on what's relevant; skip categories where the change has no meaningful impact.

- **Security:** references/security.md
- **Performance:** references/performance.md
- **Reliability:** references/reliability.md
- **Maintainability:** references/maintainability.md
- **Functionality / Correctness:** references/correctness.md
- **Architecture:** references/architecture.md
- **Scope Control:** references/scope-control.md

Each path above is relative to THIS skill's own directory, the path the Skill tool reports as "Base directory for this skill" when `playbook:grounding-review` is invoked, not relative to the caller's working directory or any target repo being reviewed.

A lens-scoped reviewer dispatched by `/playbook:deep-review` or `/playbook:implement` never invokes this skill directly for that dispatch. Its orchestrator resolves the matching file to an absolute path instead, via `$CLAUDE_PLUGIN_ROOT`, the same env var `hooks/hooks.json`, `commands/doctor.md`, and `commands/setup.md` already use for plugin-bundled files, and hands that agent the resolved path. This is necessary because a reviewer with no `Bash` cannot expand `$CLAUDE_PLUGIN_ROOT` itself, and `Read` requires an absolute path either way.

An unscoped reviewer (`/playbook:quick-review`'s single pass) invokes this skill directly, reads the base directory the Skill tool reports, and combines it with each relative path above to read all 7.

## Known Rationalizations (Review)

| Rationalization | Reality |
|---|---|
| "The file probably exists at..." | Verify with Glob or Read. Probably is not evidence. |
| "Based on the pattern, it should be..." | Pattern matching is not proof. Check the actual file. |
| "This is a common anti-pattern" | Common to whom? Show the specific code that exhibits it. |
| "The function likely does X" | Read the function. Likely is not verified. |
| "I can see from the diff that..." | The diff shows changes, not the full file. Read the source. |
| "This is obviously a bug" | Show the failure scenario: trigger, mechanism, observable failure. |
| "Based on my experience..." | Your experience is not evidence in this codebase. Cite the file. |

## Verification Summary

Every review MUST end with a Verification Summary table.

```
## Verification Summary

| File | Read | Lines | Findings |
|---|---|---|---|
| src/dao/UserDao.ts | Yes | 12, 15, 42 | #1, #3 |
| src/auth/login.ts | Yes | 4, 8 | #2 |
| src/utils/hash.ts | No | - | - |

Confidence: HIGH | All findings verified against source files.
```

**Confidence levels:**

- **HIGH**: every finding has tool-verified evidence. All file paths confirmed via Read/Glob.
- **MEDIUM**: most findings verified, but 1-2 rely on diff-only evidence (tagged `[unverified]`).
- **LOW**: multiple findings lack verification. MUST tag each as `[unverified]`.

A LOW confidence review is better than a fabricated HIGH confidence review.

## Boundaries

- Stay in your lane: only report findings within your area of expertise.
- Don't duplicate findings other specialist reviewers would cover.
- Don't invent file paths, function names, table names, or variable names.
- Don't cite abstract rules without explaining the real-world consequence.
- When reviewing dependency injection: verify constructor parameters use the service interface, not extracted raw functions. Flag raw function injection where an existing service or interface provides the method.
- Skip git commit hashes or SHAs in any output.
