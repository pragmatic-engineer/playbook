# Review and PR Flow

Once a branch is ready, this config gives you three commands to get it reviewed and shipped: `/playbook:commit-and-push` to commit and push, `/playbook:quick-review` or `/playbook:deep-review` to review, and `/playbook:address-pr-comments` to work through feedback.

## `/playbook:commit-and-push`

Generates a commit message from the staged diff, commits signed (always `--gpg-sign`, with the key and format from git config) with a `Signed-off-by` trailer (see [PR and commit settings](#pr-and-commit-settings)), rebases onto the base branch if you're behind, then pushes. It runs in an isolated subagent (`context: fork`) on Haiku, so the diff and drafting stay out of your main context. There is no confirmation gate: it commits and pushes end to end.

```bash
/playbook:commit-and-push           # commit staged changes and push
/playbook:commit-and-push -A        # stage all files (git add -A), then commit
/playbook:commit-and-push -u        # stage tracked files only (git add -u), then commit
/playbook:commit-and-push -a        # amend the previous commit instead of creating a new one
/playbook:commit-and-push --no-signoff  # leave the Signed-off-by trailer off this commit
```

Flags combine: `-Au`, `-a -u`, and so on.

If the branch is behind the base, it rebases automatically before pushing. After a rebase or amend, it uses `--force-with-lease` so the push fails safely if the remote moved unexpectedly. Hooks run normally; if one fails, fix the issue and commit again rather than skipping it.

## `/playbook:quick-review`

A single-pass PR review under the grounding-review discipline, posted as a pending GitHub review for you to submit.

```bash
/playbook:quick-review              # self-review: resolves the PR for the current branch
/playbook:quick-review 4265         # review PR #4265
/playbook:quick-review #4265        # same
```

Reviewers return plain findings. When something can be posted, it loads `playbook:writing-style` after the verification sweep and shows the report with a drafted comment under each finding. Then it asks two questions: which findings to post, then which submit verb to use (`approve`, `comment`, `request-changes`, or `skip`). The drafts you saw are posted as they are, and it never auto-submits. A self-review or report-only run never loads `playbook:writing-style`.

A PR you authored gets a local report only; nothing is posted.

## `/playbook:deep-review`

Fans out a swarm of specialist reviewer subagents in parallel (logic, test, security, data, types, perf by default), consolidates their findings, deduplicates and fact-checks, then posts the same way `/playbook:quick-review` does.

```bash
/playbook:deep-review               # current branch's PR, auto-selects reviewers from the diff
/playbook:deep-review 123           # PR #123
/playbook:deep-review 123 --all     # every reviewer regardless of diff content
/playbook:deep-review --preset security   # security + data + types + logic
/playbook:deep-review --self        # local self-review, never posts to GitHub
```

Available presets: `security`, `architecture`, `data`, `docs`.

By default, conditional reviewers (architecture, migration, docs, complexity, and others) activate based on what the diff contains. The command and its full-lens `reviewer` subagents run on Opus; a lens the triage step marks narrow gets a `cheap-checker` on Haiku. See [Model routing and memory](../internals/02-model-routing-and-memory.md) for why.

Use `/playbook:quick-review` for everyday PRs. Reach for `/playbook:deep-review` when the change is large, risky, or touches multiple layers.

## Reviewing a pull request stack

A stack is a series of PRs where each one targets the branch of the PR below it. Playbook finds stacks made with GitHub's native stacked PRs, and also stacks made with Graphite, ghstack, git-town or by hand, by following the branch chain. See [ADR-0019](../adr/0019-pr-stack-review.md) for the design.

A PR that is not in a stack is reviewed as before, with no extra question. When the PR sits in a stack with two or more open PRs, both review commands first print the size (PR count and changed lines) and ask:

1. This PR only.
2. Whole stack, quick review. This is the default.
3. Whole stack, deep review.

Pass `--this-pr` or `--whole-stack` to skip the question. In auto mode the command takes option 2 and logs the assumption. Nothing is posted in auto mode.

```bash
/playbook:quick-review 123 --whole-stack   # quick review of the stack PR #123 belongs to
/playbook:deep-review 123 --this-pr        # only PR #123, even if it is stacked
playbook pr stack 123 --json               # what the commands read: the PRs, their state and size
```

What a whole-stack review does:

- **One shared context.** Open PRs are read in full. Merged PRs are read as a short summary (title, description, file list) so the reviewer understands the code below.
- **Merged PRs are never reviewed or commented on.** A PR closed without merging is skipped.
- **Findings go to the PR that holds the line.** `playbook pr stack map` matches each finding's `file:line` against the diff hunks of the open PRs and creates one pending review per PR. A finding no open PR changes is shown in the report and not posted.
- **Safe against change.** If a PR was pushed to, retargeted or closed after the review started, its findings are held back and listed as stale.
- **Cost guard.** The command asks again before it starts when the open PRs exceed `review.stackMaxPrs` (default 6) or `review.stackMaxLines` (default 3000). See [Config keys](04-config-keys.md).

A stack with a single open PR and merged PRs below it is not asked about: the merged PRs are read as background and only the open PR is reviewed.

## The grounding-review discipline

Both review commands follow the same discipline, which covers:

**Severity levels:**

| Level | Meaning |
|---|---|
| `critical` | Must not merge. Data loss, security breach, or production outage. |
| `high` | Should not merge without addressing. Incorrect behaviour or reliability risk. |
| `medium` | May merge; address soon. Maintainability or minor correctness. |
| `low` | Informational. Style, naming. Safe to defer. |

**Conventional Comments labels** go on every finding in plain text (no bold): `blocking:`, `issue:`, `suggestion:`, `nitpick:`, `question:`. `blocking` replaces `issue` for a finding that must be fixed before merge; `issue` is reserved for a real problem that is not merge-blocking. Every finding is one sentence when possible, two at most.

Every review ends with a **Verification Summary** table listing each file, whether it was read, which lines were checked, and which findings it carries. Confidence is HIGH (every finding verified), MEDIUM (1-2 unverified), or LOW (multiple unverified).

## `/playbook:address-pr-comments`

Walks unresolved review threads one at a time. For each one it reads the code, proposes a fix or reply, waits for your approval, then applies it.

```bash
/playbook:address-pr-comments           # current branch's PR
/playbook:address-pr-comments 123       # PR #123
/playbook:address-pr-comments --bots    # include bot comments (CodeRabbit, Copilot, etc.)
/playbook:address-pr-comments --dry-run # preview everything, no edits or posts
/playbook:address-pr-comments -y        # skip the final commit confirmation
```

For each comment, you choose: `[F]ix`, `[R]eply`, `[B]oth`, `[S]kip`, `[Q]uit`, or `[E]dit-then-fix`. Reply-only comments post immediately. Fix-and-reply comments queue the reply until after commit.

At the end it invokes `commit-and-push -A`, then posts any queued replies. It never resolves threads; resolving is the reviewer's call.

Bot authors (CodeRabbit, Copilot review, Greptile, github-actions, and others) are skipped by default. Pass `--bots` to include them.

## Auto mode

These commands read the run mode first. In `ask` mode, which is the default, nothing changes. Pass `--auto` or `--ask` to override the mode for one run. See [Auto mode](../../README.md#auto-mode) for how the mode is set.

- `/playbook:commit-and-push` commits and pushes without asking. It never force-pushes. A push that would need a force is left on the local branch and reported, so you decide how to publish it.
- `/playbook:quick-review` and `/playbook:deep-review` run as `--self`. They report locally and never post a review to GitHub, because a posted review speaks as you.
- `/playbook:address-pr-comments` refuses to run. It replies on GitHub in your name, so each reply needs your approval.

## PR and commit settings

Six config keys shape the PR flow and the commit trailer. Read one with `playbook config get <key>`, and set it with `playbook config set [--global|--org] <key> <value>`. Without a flag it writes the repo tier; `--org` writes the org tier and `--global` writes your own, for every repo. The repo tier wins over org, and org over global. The full list of keys is in [Config keys](04-config-keys.md).

| Key | Default | What it does |
|---|---|---|
| `autoReview.enabled` | `true` | `playbook:finish-pull-request` reviews the PR before it goes ready. |
| `autoReview.type` | `auto` | Which review it runs: `deep`, `quick`, or `auto`. `auto` runs `playbook pr review-triage`, which asks a small model three times and picks `quick` only when all three agree; any disagreement or failure runs `deep`. |
| `autoReview.fix` | `false` | After the self-review, fixes every finding that survived the review's verification, pushes the fixes to the PR branch, and re-runs the scoped checks. |
| `autoMerge.enabled` | `false` | After the review and fixes, marks the PR ready, waits until every check on the PR head is green, runs `gh pr merge <n> --auto`, then re-reads the PR and reports whether it merged, is queued, was rejected, or was closed. |
| `commit.signOff` | `true` | Commits made through `/playbook:commit-and-push` carry a `Signed-off-by` trailer. |
| `pr.draft` | `true` | `/playbook:create-pull-request` opens the PR as a draft. Set it to `false` to open PRs ready for review. |

```bash
playbook config set --global autoReview.fix true
playbook config set --global autoMerge.enabled true
playbook config set --global pr.draft false
```

The merge step is cautious on purpose. It waits two minutes after marking the PR ready, so checks have registered and a run skipped while the PR was a draft can't read as green. It then polls every check on the PR (not only the required ones, since a repo may require none and a merge queue would merge at once) every 30 seconds, for up to 45 minutes. `pass` and `skipping` are fine, `fail` and `cancel` stop it, and a check still pending at the deadline stops it without merging. A PR with no checks at all merges once that two-minute look finds none.

It then runs `gh pr merge <n> --auto --match-head-commit <sha>` with the head commit it recorded, so a commit pushed during the wait is never merged unseen. It passes no method flag first, so a merge queue picks how the PR lands. If `gh` says a method is required, it reads which methods the repo allows and retries with the one allowed, preferring squash. It never passes `--admin` or `--delete-branch`. Afterwards it reads the PR and reports one of: merged, queued or auto-merge armed, rejected or auto-merge cleared, or closed.

A stacked PR whose base is another open PR is not merged before its base: the command says so and stops. In auto mode, a PR that no review covered (any caller other than `/playbook:implement`) is marked ready but never merged, even with `autoMerge.enabled` on. The settings work the same in `ask` and `auto` mode, and neither asks a question, because setting the key is your opt-in.

`commit.signOff` stands down when the message already has the trailer, when you pass `--no-signoff` on purpose, or when the repo's `prepare-commit-msg` or `commit-msg` hook writes the trailer itself (through `git interpret-trailers`, `--signoff`, or an appended `Signed-off-by:` line). A hook that only checks for the trailer does not count, so the command still adds it. It does not control cryptographic signing: commits are signed whenever your git config has a `user.signingkey` (`--gpg-sign`), using the key and format from that config.

## A typical review cycle

```bash
# 1. Branch is ready. Stage and commit.
/playbook:commit-and-push -A

# 2. Self-review before asking others.
/playbook:quick-review           # or /playbook:deep-review for a bigger change

# 3. Pick findings to post and choose a submit verb.
# The command asks; you answer.

# 4. Reviewer leaves comments. Address them.
/playbook:address-pr-comments    # walks each thread, fixes and replies, then commits and posts
```

That's the full loop. Run `/playbook:quick-review` again after a round of feedback if you want a second pass before merging.

## See also

- [Plan and Implement](01-plan-and-implement.md): producing the branch that this flow starts from.
- [Internals: Model Routing and Memory](../internals/02-model-routing-and-memory.md): why deep-review's subagents run on a different model than the orchestrating session.
- [Docs index](../index.md)
