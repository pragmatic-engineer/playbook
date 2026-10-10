---
name: finish-pull-request
description: Finishes a freshly opened PR by running the configured self-review, fixes findings, promotes the draft, and merges when allowed. Use right after /playbook:create-pull-request returns a PR URL.
---

# Finish Pull Request

Run this in the orchestrating session right after `/playbook:create-pull-request` returns a PR URL. That command forks into the `git` agent, which has no `Agent` tool and cannot spawn review swarms, so the self-review, fix, promote and merge work lives here, in the session that can.

Inputs from the create-pull-request report: the PR number or URL, its branch, and whether `--ready` was passed. If any is missing, read it with `gh pr view`.

## Run mode

Run `playbook mode status --json` (add `--flag auto` or `--flag ask` if the session was started with `--auto` or `--ask`). The `mode` key is `ask` or `auto`. In auto mode, skip the `/clear` self-review below when `/playbook:implement` opened this PR, because its review already covered the code. Any other caller gets no review here, and the final report says that no self-review ran. If `--ready` was passed, it still promotes the draft. The `autoReview.fix` and `autoMerge.enabled` settings still apply in auto mode, except that this skill never merges a PR whose code no review covered.

## Reading a setting

`playbook config get <key>` prints `<key>: <value> (source: <tier>)`. A config read error is a non-zero exit, or a line without the expected value. Never treat an error as `false` for a review setting: use the review fallback in Step 5.1. For the two opt-in settings (`autoReview.fix`, `autoMerge.enabled`) an error means `false`, because a broken config must never turn on an extra action. Name what `playbook config get` printed to stderr in the final report either way.

## Step 5: Self-review before ready

1. Decide whether a review runs, and which one, before touching the PR further.
   - `playbook config get autoReview.enabled`. If it prints `false`, skip the review and say so in the final report ("auto-review is disabled for this repo, review skipped"). If it prints `true`, or the key is unset (it defaults to `true`), read `playbook config get autoReview.type`, which is `auto` (the default), `deep` or `quick`.
   - `auto`: run `playbook pr review-triage --pr <n>` and read its first line, `review=quick` or `review=deep`. It prints `quick` only when three small-model reads agree. If the command exits non-zero or prints anything else, use `deep`: a triage failure never skips or lightens the review.
   - `deep`: run `/clear`, then `/playbook:deep-review --self` against the PR just created (a draft is a real PR, so this works).
   - `quick`: run `/clear`, then `/playbook:quick-review --self`.
   - Config read error on either call: do not block and do not skip. Run `/clear`, then `/playbook:deep-review --self`.
2. After the review (a `/clear` drops what was read before it), read `playbook config get autoReview.fix` and `playbook config get autoMerge.enabled`. Both default to `false`, apply in ask and auto mode alike, and need no question.
3. Fix the findings. A push updates the draft, no new PR needed.
   - `autoReview.fix` is `false`: fix any findings the review surfaced, as you judge best.
   - `autoReview.fix` is `true`: fix every finding that survived the review's sweep. Commit through `/playbook:commit-and-push`, then re-run the scoped checks for the changed area and fix what they report. If no review ran, there are no findings to fix: say so.
4. If the PR is already ready (it printed `Created ready PR`), skip `gh pr ready`. Otherwise, if `--ready` was passed or `autoMerge.enabled` is `true` (a PR cannot merge while it is a draft), run `gh pr ready <branch>` now, after Step 5.1 decided whether a review runs and after fixing any findings, not before.
5. If `autoMerge.enabled` is `true`, merge only when the code was reviewed and every check is green. Take these in order and stop at the first that fails:
   - **Reviewed:** in auto mode, when no review ran (any caller other than `/playbook:implement`), do not merge, even with `autoMerge.enabled` on. The PR is already ready from step 4. Say in the final report that auto-merge was skipped because no review covered the code, and stop.
   - **Stacked PR:** if the PR's base is another open PR's branch rather than the default branch, do not merge it before that base merges. Check with `gh pr view <n> --json baseRefName` and `gh pr list --state open --head <baseRefName> --json number`. Say it is waiting on its base and stop, leaving the PR ready.
   - **Wait for green:** record the head with `gh pr view <n> --json headRefOid -q .headRefOid`. Then run `playbook pr ci-wait <n> --all --settle 120 --timeout 300 --interval 30` with the Bash `timeout` set to 600000. `--all` waits on every check, not only the required ones, since a repo may require none. `--settle` lets checks register after `gh pr ready`. On `CI_VERDICT=TIMEOUT` run it again with `--timeout 540` and no `--settle`, up to five more times, about 45 minutes in all. Each run is its own Bash call. Read the verdict:
     - `PASS`: go on, once the head still equals the recorded one. A changed head means do not merge: report it and stop.
     - `NONE`: the PR has no checks. Go on and say so in the final report.
     - `FAIL` or `CANCELLED`: do not merge. Report the printed counts, name the failing checks with `gh pr checks <n>`, and stop.
     - `TIMEOUT` after the last run: report that checks were unfinished and stop without merging.
   - **Merge:** run `playbook pr merge <n> --match-head <recorded headRefOid>`. It arms auto-merge with no method flag, so a merge queue picks how it lands, and `gh` refuses if the head moved. It prints `merge_rc=<code>` and what `gh` said. If the text says a merge method is required, read `gh repo view --json squashMergeAllowed,mergeCommitAllowed,rebaseMergeAllowed` and run `gh pr merge <n> --auto --match-head-commit <recorded headRefOid>` with the one allowed method flag (`--squash`, `--merge` or `--rebase`), preferring squash. On any other refusal, report the message and stop. Never `--admin`, never `--delete-branch`, never a method flag to get around any other refusal.
   - **Confirm:** do not trust the exit code. Run `playbook pr land-wait <n> --timeout 540 --interval 15`. It prints `<state> <mergeState> <review> <armed>` per read, then `LAND_VERDICT=`. Report which case it is, and never call a PR merged unless the verdict is `MERGED`:
     - `MERGED`: merged.
     - `TIMEOUT`, last line `OPEN` with `armed`: queued, or auto-merge armed and waiting. Report it as still waiting.
     - `TIMEOUT`, last line `OPEN` with `-` in the last column: the merge was rejected or auto-merge was cleared. Report it and stop.
     - `TIMEOUT`, last line `CLOSED`: closed without merging.
     - `REVIEW_GATE` or `CHANGES_REQUESTED`: a person has to act. Report it and stop.
     - `RESTATE`: the PR is behind or in conflict. Report it and stop.
6. If neither `--ready` nor `autoMerge.enabled` applies: stop after step 3. Leave the PR in the state it opened in (a draft by default).

Never skip straight to `gh pr ready` on a fresh draft without running Step 5.1 first, even when it concludes with "review skipped" rather than an actual review.
