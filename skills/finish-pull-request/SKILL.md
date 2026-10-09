---
name: finish-pull-request
description: Finishes a freshly opened PR by running the configured self-review, fixes findings, promotes the draft, and merges when allowed. Use right after /playbook:create-pull-request returns a PR URL.
---

# Finish Pull Request

Run this in the orchestrating session right after `/playbook:create-pull-request` returns a PR URL. That command forks into the `git` agent, which has no `Agent` tool and cannot spawn review swarms, so the self-review, fix, promote and merge work lives here, in the session that can.

Inputs you carry over from the create-pull-request report: the PR number or URL, its branch, and whether `--ready` was passed. If any is missing, read it with `gh pr view`.

## Run mode

Run `playbook mode status --json` (add `--flag auto` or `--flag ask` if the session was started with `--auto` or `--ask`). The `mode` key is `ask` or `auto`. In auto mode, skip the `/clear` self-review below when `/playbook:implement` opened this PR, because its review already covered the code. Any other caller gets no review here, and the final report says that no self-review ran. If `--ready` was passed, it still promotes the draft. The `autoReview.fix` and `autoMerge.enabled` settings still apply in auto mode: the steps below read and follow them, except that this skill never merges a PR whose code no review covered.

## Step 5: Self-review before ready

1. Decide whether a review runs, and which one, by reading the repo's auto-review config before touching the PR further:
   - Run `playbook config get autoReview.enabled` and read its stdout. On success it prints a line of the shape `autoReview.enabled: <value> (source: <tier>)`, where `<value>` is `true` or `false`. If the command exits non-zero, or the line does not contain either `true` or `false`, that is a config read error: **do not** treat it as `false`, go straight to the fallback bullet below instead.
   - If the printed value is `false`: skip the review entirely. Say so explicitly in the final report ("auto-review is disabled for this repo, review skipped") so the user knows it was intentional, not forgotten.
   - If the printed value is `true` (or the key is unset, which defaults to `true`): run `playbook config get autoReview.type` and read its stdout, printed the same way (`autoReview.type: <value> (source: <tier>)`, with `<value>` an unquoted `auto`, `deep` or `quick`).
     - `auto` (the default when unset): run `playbook pr review-triage --pr <n>` for the PR just created and read its first line, `review=quick` or `review=deep`. It asks a small model three times and prints `quick` only when all three agree; the lines after it are signals and notes for your own reading, and the `/clear` below drops them. Then follow the matching bullet below. If the command exits non-zero, prints anything else, or you cannot read the line, use `deep`: a triage failure never skips or lightens the review.
     - `deep` (or the triage answered `deep`): run `/clear`, then run `/playbook:deep-review --self` against the PR just created, exactly as before. A draft PR is a real PR, so this works; reviewing before any PR exists does not (`deep-review` needs `gh pr view`/`gh pr diff`).
     - `quick` (or the triage answered `quick`): run `/playbook:quick-review --self` instead. Run `/clear` first too: `quick-review --self` is report-only and asks no follow-up questions, so a fresh context is not strictly needed here, but clearing costs nothing and keeps the same safety margin as the `deep` path.
   - Fallback, config read error: if either `playbook config get` call above itself errors (a non-zero exit, or a line that does not contain the expected token; this happens when a config file at some tier is malformed JSON or is not an object), **do not** block PR creation and **do not** silently skip the review. Fall back to today's default: run `/clear`, then run `/playbook:deep-review --self`. Name what `playbook config get` printed to stderr in the final report, so a broken config file degrades to the known-safe default instead of a silent skip or a hard failure.
2. Read the two opt-in PR settings, after the review (a `/clear` drops what was read before it). Run `playbook config get autoReview.fix` and `playbook config get autoMerge.enabled`, each printed like the review settings (`<key>: <value> (source: <tier>)`). Both default to `false`. If either call errors or prints neither `true` nor `false`, treat that key as `false` and name what `playbook config get` printed to stderr in the final report: these settings opt in to extra actions, so a broken config must never turn one on. They apply in ask mode and in auto mode alike, and because the user opted in through config, neither needs a question.
3. Fix the findings. A push updates the draft automatically, no new PR needed.
   - `autoReview.fix` is `false`: fix any findings the review surfaced, as you judge best.
   - `autoReview.fix` is `true`: fix every finding that survived the review's own verification sweep (the fact-check in `deep-review`, the grounding pass in `quick-review`), not only the ones that look easy. Commit the fixes through `/playbook:commit-and-push`, which pushes them to the PR branch, then re-run the scoped checks for the changed area (the repo's test, lint and format commands for those files) and fix what they report. If no review ran (disabled, or auto mode with no review), there are no findings to fix: say so.
4. If the PR is already ready (it printed `Created ready PR`, because `pr.draft` is `false`), skip `gh pr ready`, since there is nothing to promote. Otherwise, if `--ready` was passed (the caller wanted this published, not left as a draft), or `autoMerge.enabled` is `true` (a PR cannot merge while it is a draft): run `gh pr ready <branch>` now, after step 1 decided whether a review runs (and after fixing any findings when it did), not before. `--ready` means "ready once step 1 has run," whether that ran a review or explicitly skipped one for a disabled repo; it never means "skip step 1."
5. If `autoMerge.enabled` is `true`, merge the PR only when its code was reviewed and every check is green. Take these in order, and stop at the first one that fails:
   - **Reviewed:** in auto mode, when no review ran (any caller other than `/playbook:implement`), do not merge, even with `autoMerge.enabled` on. The PR is already marked ready from step 4: say in the final report that auto-merge was skipped because no review covered the code, and stop.
   - **Stacked PR:** if the PR's base is another open PR's branch rather than the repo's default branch, do not merge it before that base merges. Check with `gh pr view <n> --json baseRefName` and `gh pr list --state open --head <baseRefName> --json number`. Say it is waiting on its base and stop, leaving the PR ready.
   - **Wait for green:** record the head commit with `gh pr view <n> --json headRefOid -q .headRefOid`. Then wait two minutes after `gh pr ready` before trusting any result: checks register late, and a run skipped while the PR was a draft can make an early list look green. After that, poll `gh pr checks <n> --json name,bucket` every 30 seconds until a deadline of 45 minutes from the first poll. Each poll is its own short Bash call (or a loop that stays well inside the Bash tool's time limit, a few minutes at most): never one call that waits out the whole deadline. Wait on all checks, not only the required ones: a repo may require none, and a merge queue would merge at once. Decide on each check's `bucket`:
     - `pass` or `skipping`: fine.
     - `fail` or `cancel`: do not merge. Report which check and stop.
     - `pending`: keep waiting.
     - Every check `pass` or `skipping`: go on, once the head commit still equals the recorded one. A head commit that changed means do not merge: report it and stop.
     - Deadline reached with a check still pending: report which checks were unfinished and stop without merging.
     - No checks at all once the two minutes are up: the PR has none. Go on and say so in the final report.
   - **Merge:** run `gh pr merge <n> --auto --match-head-commit <recorded headRefOid>` with no method flag first, so a merge queue picks how it lands. If `gh` refuses because a merge method is required, run `gh repo view --json squashMergeAllowed,mergeCommitAllowed,rebaseMergeAllowed` and retry the same command with the one allowed method flag (`--squash`, `--merge` or `--rebase`), preferring squash when several are allowed. On any other refusal, report the message and stop. Never `--admin`, never `--delete-branch`, never a method flag to get around any other refusal.
   - **Confirm:** do not trust the exit code. Read `gh pr view <n> --json state,autoMergeRequest` every 15 seconds, up to 10 minutes, each read its own short Bash call or a short loop. Report which case it is, and never call a PR merged unless `state` reads `MERGED`:
     - `state` is `MERGED`: merged.
     - `state` is `OPEN` and `autoMergeRequest` is set: queued, or auto-merge armed and waiting. Keep reading until the limit, then report it as still waiting.
     - `state` is `OPEN` and `autoMergeRequest` is null: the merge was rejected or auto-merge was cleared. Report it and stop.
     - `state` is `CLOSED`: closed without merging.
6. If neither `--ready` nor `autoMerge.enabled` applies: stop after step 3. Leave the PR in the state it opened in (a draft by default).

Never skip straight to `gh pr ready` on a fresh draft without running step 1 first, even when step 1 concludes with "review skipped" rather than an actual review.
