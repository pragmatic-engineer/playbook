---
description: Use to review a PR or your own branch. It recommends a quick or a deep review from the diff, asks you to pick, then runs it. The single review entry point.
allowed-tools: Bash, Read, Grep, Glob, Agent, Skill
argument-hint: "[PR number] [--quick] [--deep] [--self] [--no-post] [--all] [--preset <name>] [--this-pr] [--whole-stack] [--auto] [--ask] [--help]"
model: sonnet
effort: low
---

# Review

One command for every review. It looks at the diff, recommends a quick or a deep review, lets you pick, then runs it. You always have the final say. `/playbook:quick-review` and `/playbook:deep-review` stay as direct entry points and do the work: this command only picks between them.

## Help

If the arguments contain `--help`, print the argument-hint line and these lines, then stop:

```
/playbook:review [PR number] [options]

  --quick      Quick review, no question
  --deep       Deep review, no question
  --all, --preset <name>   Deep review options (they imply --deep)
  Every other option goes to the review that runs: --self, --no-post,
  --this-pr, --whole-stack, --auto, --ask.
```

## Step 0: Read the run mode

Do this first, before Step 1. Read the run mode and the effort ceiling for `review` with `$ARGUMENTS`: read `skills/delegating-subagents/references/run-context.md` in this plugin and follow it. It covers the tool, the plain steps without it, the warning, the ceiling, the flag conflict and any failure.

**Routing.** Where you choose what to spawn, route the `classify` kind. Intent: route-model. If the session's tool map (printed at session start) lists it, use that command and follow its `action` under the `routing.escalate` gate: on `ask` stop and ask the user, on `downgraded` (`deny`) use the lower tier it names, and in auto mode (`auto`) proceed and log the assumption. If the command fails, carry on. Otherwise do this: spawn the agent this file names, with the model its agent file pins.

- **`ask` mode:** behave exactly as this file describes.
- **`auto` mode:** follow the Auto path below.

## Step 1: Read the arguments

Parse `$ARGUMENTS`:

- `--quick` and `--deep` together: stop with `--quick and --deep conflict; pass one.`
- `--all` or `--preset <name>` exist only in the deep review. They mean `--deep`. With `--quick`, stop with `--all and --preset belong to the deep review; drop --quick.`
- The target is the first integer or `#N`, else the first branch name, else the current branch. A branch name resolves to its open PR number with `gh pr list --head <branch> --json number -q '.[0].number'`; stop with `error: no open PR for branch <name>; create one first or pass a PR number` when there is none.
- Every other word goes to the review that runs, unchanged, with `--quick` and `--deep` removed. The review reads `--self`, `--no-post`, `--this-pr`, `--whole-stack`, `--all`, `--preset`, `--auto` and `--ask` itself.

A depth from `--quick`, `--deep`, `--all` or `--preset` is the user's choice: skip to Step 4 and say `Depth: <depth> (from <flag>)`.

## Step 2: Recommend a depth

Intent: review-suggest. If the session's tool map lists it, use that command with the PR number (omit it for the current branch) and `--json`. It reads git, and `gh` only for a PR number, then prints `recommendation` (`quick` or `deep`), `confidence` (`high` or `low`), up to four `reasons` and the `signals` it used. Otherwise do this: read `skills/grounding-review/references/review-routing.md` in this plugin and apply Part 1 by hand with plain `git` and `gh`. If the command fails, do the same by hand.

With `high` confidence, go to Step 4.

## Step 3: Get a second opinion when the answer is unsure

Only for `low` confidence. Read `skills/grounding-review/references/review-routing.md` in this plugin and follow Part 2: one cheap `playbook:review-triage` pass, once. Its answer becomes the recommendation. If it cannot run, keep the first recommendation and say so in one line.

## Step 4: Choose

Print the recommendation first, in one line: `Recommended: <depth> (<confidence> confidence): <reasons joined in one short line>`.

**Q1:** "Which review? 1. <recommended depth> review (Recommended): <reasons, one line>. 2. <other depth> review." Ask in ask mode only, with the recommended depth first and marked `(Recommended)`. Accept `1`, `2`, `quick`, `deep`, or an empty answer for the recommendation. Whatever the user picks wins. When Step 1 already set a depth, do not ask.

### Auto path

Take the recommendation without asking and print `Auto mode chose <depth>: <reasons in one line>`, so the choice is on record. The review that runs gets `--auto` and keeps its own auto rules. Auto mode does not skip a review's post question: only `--self` or `--no-post` does, so an unattended caller passes one of them.

## Step 5: Run the review

Load the review for the chosen depth with the Skill tool and give it the arguments from Step 1 (the PR number when there is one, plus every other word):

- Quick: `/playbook:quick-review`.
- Deep: `/playbook:deep-review`.

From here that review owns everything: the worktree, the reviewers, the sweep and the posting questions. Its report is the result. Add one line at the end: `Depth: <depth> (<recommended | chosen by you | from a flag>).`
