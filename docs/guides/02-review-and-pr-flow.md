# Review and PR Flow

Four commands take a ready branch to a merged PR: `/playbook:commit-and-push`, `/playbook:create-pull-request`, `/playbook:quick-review` or `/playbook:deep-review`, and `/playbook:address-pr-comments`.

## Commit and push

`/playbook:commit-and-push` writes a commit message from the staged diff, commits signed with a `Signed-off-by` trailer (see [PR and commit settings](#pr-and-commit-settings)), rebases if you are behind, then pushes. It runs in a forked `git` subagent, so the diff stays out of your main context. There is no confirmation gate.

```bash
/playbook:commit-and-push               # commit staged changes and push
/playbook:commit-and-push -A            # git add -A first
/playbook:commit-and-push -u            # git add -u first
/playbook:commit-and-push -a            # amend the previous commit
/playbook:commit-and-push --no-signoff  # no Signed-off-by on this commit
```

Flags combine (`-Au`). After a rebase or amend it pushes with `--force-with-lease`. Hooks run normally: if one fails, fix the issue and commit again.

## Create a pull request

`/playbook:create-pull-request` runs pre-flight checks, drafts a conventional-commit title and the team template body, then opens the PR. It opens a draft unless `pr.draft` is `false`.

## Quick review

A single-pass review under the `grounding-review` discipline, posted as a pending GitHub review for you to submit.

```bash
/playbook:quick-review              # the PR for the current branch
/playbook:quick-review 4265         # PR #4265
```

It shows a report with a drafted comment under each finding. Then it asks which findings to post and which verb to use (`approve`, `comment`, `request-changes` or `skip`). It never submits on its own. A PR you authored gets a local report only.

## Deep review

A swarm of specialist reviewers runs in parallel (logic, test, security, data, types and perf by default). The command consolidates, deduplicates and fact-checks the findings, then posts the way quick review does.

```bash
/playbook:deep-review                     # the current branch's PR, reviewers picked from the diff
/playbook:deep-review 123 --all           # every reviewer
/playbook:deep-review --preset security   # security + data + types + logic
/playbook:deep-review --self              # local report, never posts
```

Presets are `security`, `architecture`, `data` and `docs`. Conditional reviewers (architecture, migration, docs, complexity) switch on from what the diff contains. Full reviewers run on Opus. A lens that triage marks narrow gets a cheap Haiku checker. See [Model routing](../internals/02-model-routing-and-memory.md).

Use quick review for everyday PRs. Use deep review for large, risky or cross-layer changes.

## Review a stack

A stack is a series of PRs where each one targets the branch of the PR below it. Playbook finds GitHub native stacks, and stacks made with Graphite, ghstack, git-town or by hand, by following the branch chain. Design: [ADR-0019](../adr/0019-pr-stack-review.md).

A PR outside a stack is reviewed with no extra question. When the PR sits in a stack, even with a single open PR, both commands print the size and always ask:

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

## How findings are graded

Both commands follow the `grounding-review` discipline.

**Labels** are Conventional Comments in plain text: `blocking:`, `issue:`, `suggestion:`, `nitpick:`, `question:`. Use `blocking` for must-fix before merge and `issue` for a real problem that does not block. Each finding is one or two sentences.

Every review ends with a **Verification Summary**: each file, whether it was read, which lines were checked, and its findings. Confidence is HIGH (all verified), MEDIUM (1 or 2 unverified) or LOW (more).

## Address review comments

`/playbook:address-pr-comments` walks unresolved threads one at a time. It reads the code, proposes a fix or reply, waits for your approval, then applies it.

```bash
/playbook:address-pr-comments           # current branch's PR
/playbook:address-pr-comments 123       # PR #123
/playbook:address-pr-comments --bots    # include bot comments (CodeRabbit, Copilot, etc.)
/playbook:address-pr-comments --dry-run # preview everything, no edits or posts
/playbook:address-pr-comments -y        # skip the final commit confirmation
```

For each comment you choose `[F]ix`, `[R]eply`, `[B]oth`, `[S]kip`, `[Q]uit` or `[E]dit-then-fix`. Replies queue until after the commit. At the end it runs `commit-and-push -A`, then posts the replies. It never resolves threads. Bot authors are skipped unless you pass `--bots`.

## Auto mode

In `ask` mode, the default, nothing changes. Pass `--auto` or `--ask` to override the mode for one run. See [Auto mode](05-auto-mode.md).

- `/playbook:commit-and-push` commits and pushes without asking. It never force-pushes. A push that would need a force is left on the local branch and reported, so you decide how to publish it.
- `/playbook:quick-review` and `/playbook:deep-review` run as `--self`. They report locally and never post a review to GitHub, because a posted review speaks as you.
- `/playbook:address-pr-comments` refuses to run. It replies on GitHub in your name, so each reply needs your approval.

## PR and commit settings

Six keys shape the PR flow and the commit trailer. Read one with `playbook config get <key>`. Set it with `playbook config set [--global|--org] <key> <value>`: no flag writes the repo tier. Repo wins over org, and org over global. All keys: [Config keys](04-config-keys.md).

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

### How auto-merge behaves

The merge step is cautious on purpose.

- It waits two minutes after marking the PR ready, so checks have registered. A PR with no checks at all merges once that look finds none.
- It polls every check on the PR every 30 seconds, for up to 45 minutes. `pass` and `skipping` are fine. `fail`, `cancel` or a check still pending at the deadline stops it without merging.
- It runs `gh pr merge <n> --auto --match-head-commit <sha>`, so a commit pushed during the wait is never merged unseen. With no method flag, a merge queue picks how the PR lands. If `gh` asks for a method, it retries with the one the repo allows, preferring squash. It never passes `--admin` or `--delete-branch`.
- It then reads the PR and reports one of: merged, queued, rejected, or closed.

A stacked PR whose base is another open PR is not merged before its base. In auto mode, a PR that no review covered is marked ready but never merged, even with `autoMerge.enabled` on. The settings work the same in `ask` and `auto` mode and ask no question, because setting the key is your opt-in.

`commit.signOff` stands down when the message already has the trailer, when you pass `--no-signoff`, or when the repo's `prepare-commit-msg` or `commit-msg` hook writes the trailer itself. A hook that only checks for it does not count. It does not control cryptographic signing: commits are signed whenever your git config has a `user.signingkey` (`--gpg-sign`), using the key and format from that config.

## A typical cycle

```bash
/playbook:commit-and-push -A          # commit and push
/playbook:create-pull-request         # open the PR
/playbook:quick-review                # self-review; or deep-review for a bigger change
/playbook:address-pr-comments         # after a reviewer leaves comments
```

## See also

- [Plan and Implement](01-plan-and-implement.md): producing the branch that this flow starts from.
- [Internals: Model Routing and Memory](../internals/02-model-routing-and-memory.md): why deep-review's subagents run on a different model than the orchestrating session.
- [Docs index](../index.md)
