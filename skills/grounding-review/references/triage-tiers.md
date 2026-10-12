# Triage tiers

How `/playbook:deep-review` and `/playbook:implement` (Step 9) act on the tier map that `review-triage` returns and `playbook review triage-merge` completes. The two commands point here for the rules below and keep their own lens lists, prompts and commands.

## Completing the map

A dispatch that failed or returned nothing, a lens the map left out, and a `tier` other than `skip`, `cheap-check` or `full-lens` all become `full-lens`; lenses triage did classify keep their tier. A silent or malformed triage never narrows coverage (per `playbook:delegating-subagents`, `review-triage` is structurally read-only so its return value can fail silently).

## cheap-check

Dispatch a `cheap-checker` agent (`subagent_type: playbook:cheap-checker`, already pinned to low effort, so no tier variant applies) instead of `reviewer`. Its prompt names the lens's narrow concern, taken from the tier map's `reason` field for that lens (already a short, grounded justification from `review-triage`, so it doubles as the concern statement), the diff the command reviews and its conventions for where the tree lives, and ONE reference file path to read for criteria.

Resolve that file with one call, `playbook review ref <lens>`, which prints its absolute path: the lens's grounding-review reference file, or the full `SKILL.md` path for a lens with no matching file (or when the file is missing). Hand `cheap-checker` that ABSOLUTE path, never a bare repo-relative string: it has no `Bash`, and a repo-relative path never resolves against the diff's own target repo. The narrow concern text, not the reference file, is what scopes the check, so a `SKILL.md` fallback still returns a finding scoped to just that lens's concern.

## skip

Dispatch nothing for that lens. Track it explicitly as skipped in the one-line `Triage:` summary (for example "Triage: security=full-lens, docs=cheap-check, perf=skip"). A skipped lens is never conflated with a dispatched lens that returned nothing: it was never dispatched, so it has no return value to lose.

## Trust gate

This tiered dispatch ships and functions as soon as it lands: a `full-lens` tier still gets the exact reviewer it always did, a `cheap-check` tier gets a real narrow-scope pass from `cheap-checker`, and a `skip` tier is a real, tracked decision to run nothing. But a `skip` or `cheap-check` decision should not be treated as validated judgment yet: `playbook eval review-triage` has not yet recorded a pass verdict against a real fixture set. Until it has, treat triage's tier choices as best-effort, not proven: a `skip` verdict is not yet evidence a lens truly had nothing to find, and a `cheap-check` narrow pass is not yet guaranteed to have caught everything the full lens would have.
