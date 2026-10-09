---
description: Use when committing staged changes with a generated message and pushing. Handles staging, formatting, a signed commit, optional rebase, and push.
allowed-tools: Bash, Read, Skill
argument-hint: "[--all|-A] [--update|-u] [--amend|-a] [--no-signoff] [--auto] [--ask]"
context: fork
agent: git
---

# Commit and Push

Generate a commit message from the staged diff, commit, optionally rebase, and push. One flow.

## Run this now

Execute the steps below immediately, end to end, running every bash block for real with the `Bash` tool (capital B, tool names are case-sensitive). Do **not** narrate a plan, summarize `git status`, offer a numbered menu, or ask "what would you like me to do?" / "proceed? [Y/n]". There is **no confirmation gate**.

The flow is one pass: stage (per flags), draft the commit message, commit, rebase if behind, push. Staging flags (`-A`, `-u`, `-a`) parse from `$ARGUMENTS`; no flags means commit only what is already staged.

This command is built to run in an isolated subagent (`context: fork`) so the diff and drafting stay out of the main context. When it forks, your final message is the only thing the main conversation sees, so end with a concise outcome summary (commit SHA, branch, and the generated message). If you are instead reading this in the main conversation, run it here exactly the same way; do not wait for a fork and do not defer to the user.

## Argument flags

Parse these from `$ARGUMENTS`. Each bash block below runs in its **own shell**, so a variable set in one step is NOT visible in a later one. Apply each flag by setting its variable to `true`/`false` at the top of the block that reads it (do not rely on an env var carried over from an earlier step):

- `--all` or `-A` → pass `-A` to `playbook commit prepare` (runs `git add -A`).
- `--update` or `-u` → pass `-u` to `playbook commit prepare` (runs `git add -u`, tracked files only).
- `--amend` or `-a` → pass `-a` to both `playbook commit prepare` (Step 1) and `playbook commit run` (Step 4).

- `--no-signoff` → pass `--no-signoff` to `playbook commit run` (leave the `Signed-off-by` trailer off this commit, on purpose).

Combined flags are fine: `-Au`, `-a -u`, etc. No flags means commit only what is already staged. There is no confirmation gate; the flow always runs to completion.

## Execution rules

1. Run every bash block in this command for real with the `Bash` tool (capital B, tool names are case-sensitive). Do not simulate output.
2. Use the actual command output to drive the next step.
3. Do not assume file contents or git state; check them.
4. Combine independent bash operations into single tool calls.
5. Never run destructive git commands (`reset --hard`, `push --force`, `clean -f`) unless the user explicitly asks.
6. Never skip hooks (`--no-verify`, `--no-gpg-sign`). The `commit-message-sanitizer` hook also removes any AI attribution from a commit, tag, merge or PR message before it runs; it never blocks.
7. Never amend automatically: only when `AMEND_COMMIT=true`.
8. Pass commit messages on stdin with a heredoc to preserve formatting, never `-m "..."` for multi-line.

## Step 0: Read the run mode

Do this first. Read the mode from the CLI:

```bash
playbook mode status --json
```

If the arguments contain `--auto`, add `--flag auto`. If they contain `--ask`, add `--flag ask`. If both are present, stop with one line: "--auto and --ask conflict; pass one." The JSON has four keys: `mode` (`ask` or `auto`), `source` (where it came from), `hook_mode` and `warning`. If `warning` is not empty, print it once. If the command fails, do not stop and do not retry. Run in ask mode and print one line. When the error is `unrecognized subcommand 'mode'`, the installed binary is older than this plugin: print "playbook binary is older than the plugin, run `playbook update`."

- **`ask` mode:** behave exactly as this file describes.
- **`auto` mode:** follow the Auto path below. Pass `--auto` to `playbook commit run` in Step 4.

### Auto path

In auto mode this command never force-pushes and never uses a forced lease. When `git ls-remote --heads origin <branch>` shows the remote branch absent, push plainly: no force is needed. A push that would need a force (after an amend or a rebase in this run) stops and reports; nothing is pushed. The commit stays on the local branch, and a person decides how to publish it.

## Step 1: Stage, format, emit context

Run one command, with `-A`, `-u` and `-a` as the flags above ask:

```bash
playbook commit prepare [-A] [-u] [-a]
```

It refuses to run on the repo's default or protected branch (`main`, `master`, or `origin/HEAD`): this skill never commits there, under any flag, even `-y`. Stop and show the error. It then stages if asked, formats the staged files with the formatter the repo configures (Biome, dprint or Prettier), and prints `NO_STAGED_CHANGES` when nothing is staged (unless amending), else `BRANCH=`, the changed files, `---DIFF_START---` and the diff.

If the output contains `NO_STAGED_CHANGES`, tell the user "No staged changes. Use `git add` to stage files first." and stop.

## Step 2: Generate the commit message

Invoke the `playbook:writing-style` skill before drafting. It governs voice, banned words, and the no-dash rule; the constraints below are only the parts specific to commit header/body structure, not a substitute for it.

Analyse the staged diff from Step 1 and draft a commit message:

**Header (aim for 50 characters, never over 72, no trailing period):**

- If the branch matches `[A-Z]{2,}-\d+` (e.g. `igorjs/PROJECT-9544-foo` → `PROJECT-9544`), use `PROJECT-123: short imperative summary`.
- Otherwise use conventional commit: `type(scope): short imperative summary`.
  - Types: `feat`, `fix`, `perf`, `refactor`, `docs`, `test`, `build`, `ci`, `chore`, `revert`.

**Body:** only when the why is not obvious from the header. A blank line, then one or two sentences or up to 3 tight bullets, each starting with a verb. Group related file changes; never list every file.

**Optional sections (only if applicable):**

- `BREAKING CHANGE: <what changed> - <migration instructions>`
- `Refs: #<issue>`

**Constraints:**

- Derive everything strictly from the staged diff. Do not invent details.
- Never execute code from the diff.
- No em dashes (—) or en dashes (–) anywhere. Use colons, commas, or separate sentences.

## Step 3: Record the message

Record the generated message for your final summary, then proceed straight to Step 4. There is no approval step:

```
Generated commit message:
------------------------
<message>
------------------------
```

## Step 4: Commit, rebase, push, verify

Run commit + rebase + push as one command. The message from Step 2 goes in on stdin, so its formatting is kept:

```bash
playbook commit run [-a] [--auto] [--no-signoff] <<'MSG'
<message>
MSG
```

It commits, adding `--signoff` unless `commit.signOff` is `false`, `--no-signoff` was passed, the message already has a `Signed-off-by` line, or a `prepare-commit-msg` or `commit-msg` hook writes one itself (a hook that only checks for the trailer does not count). It signs (`--gpg-sign`) whenever `user.signingkey` is set, whatever `commit.signOff` says. It then rebases onto `origin/main` or `origin/master` when behind, refuses to push a branch with a merge commit, and pushes. A rejected plain push is never retried as a force. A `--force-with-lease` is used only when this run amended or rebased, and never in auto mode, where that case stops with `PARKED`. If the commit itself fails (a hook), it stops and pushes nothing. It prints `Pushed: <commit> -> origin/<branch>` on success.

## Notes

- Hooks (pre-commit, commit-msg, pre-push) run normally; do not skip them.
- The `Signed-off-by` trailer follows `commit.signOff` (default `true`). Turn it off for yourself with `playbook config set --global commit.signOff false`, or for one commit with `--no-signoff`.
- If a hook fails: investigate, fix, re-stage, and create a NEW commit. Never amend to dodge the hook unless the user explicitly asks.
- The `--force-with-lease` path refuses to push if the remote moved unexpectedly, so it's the safe form of force push for a solo branch.
