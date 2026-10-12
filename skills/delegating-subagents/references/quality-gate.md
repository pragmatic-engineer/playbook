# Quality gate rules shared by plan and adr

`/playbook:plan` Step 10 and `/playbook:adr` Stage 3 run the same three phases (Fact-Check, Adversarial Review, Test Review). The rules below are the same in both, so the two commands point here and keep only their own slugs, paths and next step. The repeat loop after a FAIL lives in `gate-loop.md`.

## Recording

After every return of a phase, including every retry and not just the last, pipe the agent's full raw return text to `playbook gate record` on stdin (a quoted heredoc, no scratch file). `gate record` upserts on `(plan_slug, phase)`, so a stale FAIL from an earlier iteration is overwritten once a later iteration passes, and only the last recording before the final `gate check` matters.

## Verification Summary

Phase 1 folds a Verification Summary into its report, reusing the `playbook:grounding-review` table shape:

```markdown
## Verification Summary

| Referenced path | Confirmed? | Where used |
|---|---|---|
| <path> | Yes (Read) / No (not found) | WU-N |

Confidence: HIGH | MEDIUM | LOW
```

## Test Review criteria

The `test-reviewer` evaluates the proposed tests against `playbook:engineering-standards`: regression-pinning, flakiness, boundary coverage, test independence, mock quality, assertion strength.

## INCONCLUSIVE

A phase returns INCONCLUSIVE, not PASS, when it could not actually perform its check: the agent failed to run or returned nothing, the target files were unreadable, or its confidence is LOW and blind spots dominate so a PASS would be unsupported. INCONCLUSIVE blocks the command's next step exactly like FAIL and re-runs under the gate loop. It is labeled distinctly so the cause reads as "could not verify," not "found a problem." A gate that checked nothing MUST NOT read PASS.

## Reading the gate check result

The `gate check` exit code decides whether the command may continue. The phase reports and the result block are for the human reading them. On a non-zero exit, do NOT write any "gate passed" language and do NOT continue: report the command's own output verbatim, since it already names which phase is MISSING, FAIL, STALE or INCONCLUSIVE, and never re-narrate it in your own words. For a FAIL or INCONCLUSIVE phase, revise and re-run it on that phase's own retry loop. For a STALE phase, re-run that same phase against the current source without revising anything: STALE only means its recorded verdict's hash no longer matches the current draft, not that the phase's own review was wrong, and revising content in response would itself cascade staleness onto sibling phases.
