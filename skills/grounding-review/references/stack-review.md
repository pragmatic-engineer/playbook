# Reviewing a pull request stack

Read this after `playbook review prepare` has resolved `PR_NUMBER`. A stack is a chain of PRs where each one targets the branch of the PR below it. The design is in `docs/adr/0019-pr-stack-review.md`.

## 1. Detect

```bash
playbook config get review.stackMaxPrs
playbook config get review.stackMaxLines
playbook pr stack <PR_NUMBER> --json --max-prs <stackMaxPrs> --max-lines <stackMaxLines>
```

If a command fails, or the JSON has `in_stack: false`, the PR is not in a stack: stop reading this file, say nothing about stacks, and review exactly as the command says. Print each entry of `warnings` once.

Read `prs` (bottom first), `open_count`, `merged_count`, `open_lines`, `ask` and `over_budget`. Each PR has a `role`:

- `review`: open. Read in full, and findings may go to it.
- `context`: merged. Read for context only. Never review it, never comment on it.
- `skip`: closed without merging. Not read, not commented on.

If the PR you were given is `context` or `skip`, say so in one line and stop: there is nothing to review on it.

## 2. Choose the scope

If `ask` is false (one open PR, with or without merged PRs below it), the scope is this PR only, and there is no question. When `merged_count` is above zero, still build the context in step 3 so the merged PRs are read as background.

If `ask` is true, print one line first: `Stack of <n> PRs (<open_count> open, <merged_count> merged), <open_lines> changed lines in the open PRs.` Then pick the scope:

- `--this-pr` in the arguments: this PR only, no question.
- `--whole-stack` in the arguments: the whole stack, no question.
- Auto mode: take the recommended answer, the whole stack with a quick review, unless `--this-pr` was passed. Log it with `Assumed: whole stack, quick review (auto mode).` If `over_budget` is true, take this PR only instead and log `Assumed: this PR only, the stack is over budget (<reasons>).`
- Otherwise ask, with these three options:
  1. This PR only.
  2. Whole stack, quick review. This is the default.
  3. Whole stack, deep review.

  `/playbook:quick-review` recommends option 2 and `/playbook:deep-review` recommends option 3. When the answer is for the other command, hand over: invoke that command with the Skill tool, passing `<PR_NUMBER> --whole-stack` and the same mode flags, then stop.

When the scope is a whole stack and `over_budget` is true, ask once more before starting, naming the reasons from `over_budget_reasons`: continue anyway, this PR only, or stop. The limits are `review.stackMaxPrs` and `review.stackMaxLines`.

## 3. Build the shared context

```bash
playbook pr stack <PR_NUMBER> --context --max-prs <n> --max-lines <n>
```

It prints `context_file`, `stack_file` and one `diff_file_<n>` per open PR. Read `context_file` once. Pass its path and every `diff_file` path to the reviewer or reviewers, so the whole stack is read with one context. Merged PRs appear in `context_file` as a short summary. If a summary is too thin to follow the code, summarize that PR with the agent `playbook route mechanical --json` names, not with the review model.

For a whole-stack scope, read the files at the head of the topmost open PR, since that is where all the stack's code exists together. If it is not `PR_NUMBER`, tear down any worktree from Step 1 and run Step 1's `review prepare` again with the topmost open PR's number. Keep every PR's own number for posting.

Tell the reviewers:

- Review only the PRs marked REVIEW. Use the merged ones to understand the code.
- Cite every finding as `file:line` as it appears in that PR's own diff file, and name the PR (`pr`).
- A finding that depends on another PR in the stack says which one.

## 4. Sweep and report

The Verification Sweep checks the anchor: the finding must sit on the PR that owns the line. Show each finding with its PR number.

## 5. Post

Not in `SELF_MODE` (report only, and always in auto mode). Otherwise, after the tier question, write the kept findings as JSON (`file`, `line`, `body`, `pr`) and map them:

```bash
playbook pr stack map --stack <stack_file> --findings <findings.json> --out <mapped.json>
```

`mapped.json` has `by_pr`, `unmapped` and `stale`. Show `unmapped` and `stale` to the user with their reasons and post none of those: a line no open PR changes, a merged or closed PR, a PR pushed to or retargeted since the review started. Offer to re-run detection when anything is `stale`.

For each PR in `by_pr`, create one pending review on that PR: `commit_id` is its `head_sha` and `comments` is its list, with no review `body`. Confirm each PR is still open first. Ask the submit verb once and apply it to every review created in this run.
