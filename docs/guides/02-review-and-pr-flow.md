# Review and PR Flow

Once a branch is ready, this config gives you three commands to get it reviewed and shipped: `/playbook:commit-and-push` to commit and push, `/playbook:quick-review` or `/playbook:deep-review` to review, and `/playbook:address-pr-comments` to work through feedback.

## `/playbook:commit-and-push`

Generates a commit message from the staged diff, commits signed (`--gpg-sign --signoff`), rebases onto the base branch if you're behind, then pushes. It runs in an isolated subagent (`context: fork`) on Haiku, so the diff and drafting stay out of your main context. There is no confirmation gate: it commits and pushes end to end.

```bash
/playbook:commit-and-push           # commit staged changes and push
/playbook:commit-and-push -A        # stage all files (git add -A), then commit
/playbook:commit-and-push -u        # stage tracked files only (git add -u), then commit
/playbook:commit-and-push -a        # amend the previous commit instead of creating a new one
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

After the review report, it asks two questions: which findings to post, then which submit verb to use (`approve`, `comment`, `request-changes`, or `skip`). It never auto-submits.

In self-review mode it detects you're the PR author and only offers `comment` or `skip`. GitHub rejects `approve` and `request-changes` from the author, so those aren't offered.

## `/playbook:deep-review`

Fans out a swarm of specialist reviewer subagents in parallel (logic, test, security, data, types, perf by default), consolidates their findings, deduplicates and fact-checks, then posts the same way `/playbook:quick-review` does.

```bash
/playbook:deep-review               # current branch's PR, auto-selects reviewers from the diff
/playbook:deep-review 123           # PR #123
/playbook:deep-review 123 --all     # every reviewer regardless of diff content
/playbook:deep-review --quick       # core reviewers only
/playbook:deep-review --preset security   # security + data + types + logic
/playbook:deep-review --self        # local self-review, never posts to GitHub
```

Available presets: `security`, `architecture`, `data`, `docs`, `thorough`.

In `auto` mode (the default), conditional reviewers (architecture, migration, docs, complexity, and others) activate based on what the diff contains. The orchestrating session runs on Opus; the specialist subagents run on Sonnet. See [06-internals-memory-and-routing.md](../internals/02-model-routing-and-memory.md) for why.

Use `/playbook:quick-review` for everyday PRs. Reach for `/playbook:deep-review` when the change is large, risky, or touches multiple layers.

## The grounding-review discipline

Both review commands follow the same discipline, which covers:

**Severity levels:**

| Level | Meaning |
|---|---|
| `critical` | Must not merge. Data loss, security breach, or production outage. |
| `high` | Should not merge without addressing. Incorrect behaviour or reliability risk. |
| `medium` | May merge; address soon. Maintainability or minor correctness. |
| `low` | Informational. Style, naming. Safe to defer. |

**Conventional Comments labels** go on every finding in plain text (no bold): `issue (blocking):`, `issue (non-blocking):`, `suggestion:`, `nitpick:`. Non-blocking findings are 1-2 sentences. Blocking ones may run longer when there's a decision to argue.

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
