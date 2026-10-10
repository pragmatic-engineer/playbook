---
description: Use after install or update, or when hooks or the system prompt seem not to run. Checks the seven playbook layers and prints a status table with a fix hint for each miss.
allowed-tools: Bash, Read
argument-hint: ""
model: haiku
effort: low
---

# Doctor

Run the checks with the `Bash` tool:

```bash
playbook doctor check
```

The binary runs all seven layer checks and the informational ones, and prints
one row per line as `LEVEL  message -- fix hint`, then a closing verdict.
Print its output as the status table. Add `--json` for the same rows as JSON
(`layer`, `level`, `message`, `hint`).

If the command fails with `command not found`, the binary is not on PATH: print
`FAIL  playbook binary not on PATH -- every ported hook is dead` and the install
hint from Layer 6 below, and stop. If it fails with
`unrecognized subcommand 'check'`, the installed binary is older than this
plugin: print "playbook binary is older than the plugin, run `playbook update`."
and stop.

## What each layer checks

1. **Plugin enabled.** `claude plugin list` names playbook and does not show it
   disabled. Hint on a miss: `claude plugin marketplace add pragmatic-engineer/marketplace && claude plugin install playbook@pragmatic-engineer`.
2. **Safety guards wired.** `settings.json` names `playbook hook <name>` for the
   five PreToolUse guards (`rm-workspace-guard`, `bg-await-guard`,
   `no-slop-guard`, `precommit-check`, `commit-message-sanitizer`) and for the
   `commit-message-sanitizer` backstop on PostToolUse. A legacy `.sh` command or
   a near-miss name does not count. Fix: `playbook init`.
3. **Launcher (opt-in).** The rc file for your shell has the `playbook
   shell-init` line and `playbook` is on PATH. An old `source .../cc.zsh` line
   reads as outdated (run `playbook init`). Not installed is INFO, never FAIL.
4. **System prompt (opt-in).** `~/.config/playbook/prompts/SYSTEM_PROMPT.md`
   exists. Absent is INFO.
5. **Status line command is current.** `playbook statusline` passes. The retired
   `bash ~/.config/playbook/statusline.sh` fails (run `playbook init`). A custom
   command whose file is missing fails. A custom command that exists, or none at
   all, is INFO.
6. **Binary resolves.** `settings.json` runs every ported hook as a bare
   `playbook hook <name>`, so the binary must be on PATH or all of them silently
   do nothing. It also compares the binary version with the plugin manifest
   (a difference is INFO, since a source build can legitimately be ahead),
   requires `gate record --source`, and warns when an older `playbook` earlier on
   PATH hides a newer one. Install hint: `curl -fsSL https://raw.githubusercontent.com/pragmatic-engineer/playbook/main/install.sh | bash`
   or `brew install pragmatic-engineer/tap/playbook`, with its directory on PATH.
7. **No hook command points at a missing file.** A hook that names a path
   which does not exist fails open: it never runs and nothing says so. Bare
   `playbook hook <name>` commands are skipped, as is any path with an
   unresolved `$VARIABLE`. `playbook init` does not remove a stray entry, so
   delete it from `~/.claude/settings.json` by hand.

Layer numbers stay as they are. Do not renumber them: users and docs refer to
them.

## Informational rows

These never fail the check and carry no fix hint:

- The effective `autoReview.*`, `autoMerge.enabled`, `commit.signOff` and
  `pr.draft` values, each as `key: value (source: tier)`.
- Worktrees that `playbook worktree sweep --dry-run` reports (read the
  convention from the path: `.claude/worktrees/agent-*` is the Agent tool,
  `.worktrees/<repo>/` is the `ccc worktree` launcher, `review-worktrees/` is
  the PR review, `repos/<owner>/<repo>/*/worktrees/` is a `/playbook:implement`
  Work Unit), or "no stale worktrees found".
- Files you edited after playbook placed them, as `migration <id>: <message>`.

## Output format

Print the binary's rows unchanged, then its closing line. If a required layer
failed, the closing line names the right tool: `/playbook:setup` for layers 1 to
5, an install of the binary for layer 6 (setup cannot fix it), and a hand edit of
`~/.claude/settings.json` for layer 7.
