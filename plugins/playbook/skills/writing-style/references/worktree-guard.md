# Worktree guard

A forked git skill (`/playbook:commit-and-push`, `/playbook:create-pull-request`) starts in the session's checkout. That is not always the worktree of the agent that called it. Run this guard as the first step, before any other command, and again by `cd` at the top of every later bash block (each block is a new shell).

The caller passes two values in the fork's prompt: the absolute worktree root (its `git rev-parse --show-toplevel`) and the expected branch (its `git branch --show-current`). Call them `root` and `branch`.

1. `cd <root>`. If that fails, stop as in step 4 with `found` set to `none`.
2. Run `git rev-parse --show-toplevel`.
3. Run `git branch --show-current`. When it prints nothing (a detached HEAD), run `git symbolic-ref --short HEAD` for an unborn branch; if that fails too, the branch is `(detached)`. Never use `git rev-parse --abbrev-ref HEAD`, which fails on an unborn branch and is ambiguous for a branch named like a tag.
4. Compare both values to what the caller passed. On any mismatch stop with one line and nothing else:

   `WRONG_DIRECTORY: expected <root> on <branch>, found <toplevel> on <found-branch>`

   Commit nothing, push nothing, open nothing. Do not retry in another checkout, do not fix it with `git checkout`, and do not tell the caller to commit by hand.

When the caller passed no root and branch, run steps 2 and 3 anyway and name the top level and branch you found in your final message, so the caller sees where you acted.
