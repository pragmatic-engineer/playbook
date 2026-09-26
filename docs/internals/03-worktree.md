# Internals: Worktree Engine

`cc worktree <branch>` creates or enters a git worktree, rebases it onto the base branch, then prints `Ready:` and returns the prompt. All remaining maintenance runs in a detached background subshell. This page covers what the subshell does, how the remote branch gets created, how `node_modules` is wired up, and how conflict resolution works.

## Background maintenance subshell

Once `_wt_main` prints `Ready:`, it starts a background subshell and calls `disown`. The subshell runs these steps in order:

1. `git fetch --prune` to refresh all remote refs.
2. Fix the upstream tracking ref when it's stale.
3. Fast-forward pull when the remote branch exists.
4. `git worktree prune` to remove stale worktree entries.
5. `_wt_setup_upstream` to create the remote branch when it's missing.
6. `_wt_node_modules` to clone `node_modules` with a copy-on-write copy.
7. `playbook worktree sweep` to remove worktrees, of any creation convention, that have already landed and are not locked.

The subshell runs with `</dev/null >/dev/null 2>&1` and is disowned, so all its output is silenced. Anything visible on-screen before `Ready:` comes from foreground code in `_wt_main` or `_wt_maybe_rebase`.

## Auto-push and upstream tracking

`_wt_setup_upstream` checks whether the remote branch exists. If it does, it sets the local tracking ref with `git branch --set-upstream-to`. If it doesn't, it runs `git push -u <remote> <branch>` to create it. This fires a CI job without a manual push step.

Pass `--no-push` or set `WORKTREE_NO_PUSH=1` to skip the push. No CI job fires until you push manually:

```
git push -u origin <branch>
```

## node_modules copy-on-write clone

`_wt_node_modules` runs inside the background subshell after `_wt_setup_upstream`. It hashes `package-lock.json` in both the base repo and the worktree with SHA-256. When the hashes match, it clones `node_modules` with a copy-on-write copy: `cp -cR` clonefiles on APFS (macOS), `cp -R --reflink=auto` reflinks on GNU filesystems that support it, and a plain `cp -R` full copy as the last fallback. Each branch yields an independent tree, so an edit under the worktree's `node_modules` never touches the base repo's copy. Older builds hardlinked the files, which shared inodes and let a worktree write corrupt the base.

After cloning, `_wt_node_modules` runs `npm install --prefer-offline --no-audit --no-fund` to reconcile packages the worktree needs beyond the cloned copy. When the lockfile hashes differ, the clone step is skipped and `node_modules` isn't created automatically.

## Rebase and AI resolution

`_wt_maybe_rebase` runs in the foreground before `Ready:`. On branches you authored (matched by git author name or GitHub username), it rebases onto `origin/<base>`. Protected branch names (`main`, `master`, `trunk`, `develop`, `staging`, `release/*`, `hotfix/*`) are never rebased.

Pass `--ai-resolve` to enable AI conflict resolution. On a conflict, `_wt_maybe_rebase` prints an info block describing the conflict and prompts you to confirm before handing off to Claude haiku. The prompt defaults to yes. Declining aborts the rebase normally.

Two env vars control this behavior:

| Variable | Effect |
|---|---|
| `WORKTREE_AI_RESOLVE_SILENT=1` | Skip the prompt and resolve immediately (previous behavior). |
| `WORKTREE_AI_RESOLVE=0` | Disable AI resolution entirely, even when `--ai-resolve` is passed. |

`cc worktree` always passes `--ai-resolve` internally. Set `WORKTREE_AI_RESOLVE=0` in your shell environment to turn it off for the `cc` path.

### Migration note

Before this change, `WORKTREE_AI_RESOLVE=1` triggered silent auto-resolution with no prompt. It now always prompts first. Set `WORKTREE_AI_RESOLVE_SILENT=1` to restore the old behavior.

## See also

- [Internals: Launcher and Hooks](01-launcher-and-hooks.md): the `cc` launcher that calls `_cc_worktree`.
- [Docs index](../index.md)
