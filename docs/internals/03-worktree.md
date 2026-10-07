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

## Independent daily sweep

The sweep in step 7 above is not the only trigger for `playbook worktree sweep`'s cleanup logic. `maybe_sweep_worktrees` (`src/hooks/session_init.rs`) runs it again on every Claude Code `SessionStart`, independent of creating a worktree at all. It checks the same `worktreeCleanup.enabled` config key, then rate-limits itself to once per 24 hours per repo using a marker file's modified time, so it does not run on every single session start, only the first one after the previous day's run.

This means a repo gets swept two ways: eagerly, every time `cc worktree` creates one (step 7 above), and independently, once a day, the next time any Claude Code session opens in that repo. Both call the same underlying sweep function. The overlap is deliberate defense in depth, not duplication to fix.

The one gap neither path closes: a repo nobody opens anymore is swept by neither trigger, since both need a session or launcher event in that repo to fire. This is a known limitation, not a bug. Fixing it would mean building a real OS-level scheduler (cron, launchd, or similar), which nothing in this codebase does today for any purpose; that's a bigger decision to make only if real evidence shows disk usage from abandoned repos is actually a problem, not something to build speculatively.

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

The binary launcher does not run AI resolution yet (#446 tracks the port): a rebase conflict aborts the rebase and the worktree is left on the branch as created.

### Migration note

Before this change, `WORKTREE_AI_RESOLVE=1` triggered silent auto-resolution with no prompt. It now always prompts first. Set `WORKTREE_AI_RESOLVE_SILENT=1` to restore the old behavior.

## Claude Code worktree hooks

`playbook hook worktree-create` and `playbook hook worktree-remove` are registered in `hooks/hooks.json` for the `WorktreeCreate` and `WorktreeRemove` events. They stop `claude --worktree`, `isolation: "worktree"` agents, and background sessions from creating worktrees under `.claude/worktrees/`.

**Create.** The hook reads `cwd` and `worktree_name` (or `name`) from stdin, finds the main worktree of the repo at `cwd`, and targets `<main-parent>/.worktrees/<repo>/<name>` (or the `WORKTREE_BASE_DIR` base, like the launcher), using the same `main_worktree` and `resolve_base` the launcher uses. The name must be a single safe path segment, and `worktree-<name>` must be a valid branch name: empty names, `..`, slashes, a leading `-`, and anything git rejects are refused. If the target is already a registered worktree of the repo, it is reused; a registered entry whose folder is gone is pruned and re-added. Otherwise the hook runs `git worktree add -b worktree-<name> <target> <base>`, matching Claude Code's default branch name. The base is the payload's `base_commit`, then the cached `origin/HEAD`, then local `HEAD`. Stdout carries only the absolute path; all git output goes to stderr. Because the path follows the launcher convention, `classify` reports it as `CcLauncher` and the sweep cleans it once its work has landed.

If anything fails, the hook says why on stderr and falls back to `<repo>/.claude/worktrees/<name>` with the same branch and base. It exits non-zero only if the fallback fails too.

**Remove.** The hook acts only on a registered worktree of the repo and never on the main worktree. It keeps the worktree, prints a reason on stderr, and exits 0 when the tree has uncommitted or untracked changes, or when any commit is not reachable from a remote-tracking ref or the local default branch. Otherwise it runs `git worktree remove` without `--force`, then deletes the branch only if its name starts with `worktree-` (so a launcher branch is never touched) and it passes the same reachability rule.

## See also

- [Internals: Launcher and Hooks](01-launcher-and-hooks.md): the `cc` launcher that sets up the worktree.
- [Docs index](../index.md)
