# Worktree engine

`ccc worktree <branch>` creates or enters a git worktree, rebases it onto the base branch, and starts a session there. The engine is Rust: `playbook cc worktree` (`src/cc/worktree_run.rs`, `src/cc/worktree.rs`). Slow upkeep runs afterward in a detached process, so the session starts at once.

## What it does

In order:

1. Finds the repo's base branch through `origin/HEAD`, falling back to `main`, `master`, `trunk` or `develop`.
2. Stashes a dirty main worktree and restores it afterward, even when a later step fails.
3. Names the folder after the JIRA key in the branch (`PROJECT-1234-foo-bar` gives `PROJECT-1234/`), or the branch leaf when there is none.
4. Creates the worktree at `<repo-parent>/<base>/<repo>/<folder>`. `<base>` is `WORKTREE_BASE_DIR` (default `.worktrees`, a relative value sits under the repo parent, an absolute one is used as is). An existing worktree on the right branch is fast-forwarded instead.
5. Copies `.env` from the base repo without overwriting.
6. Rebases the branch onto the latest base when you authored it. Protected names (`main`, `master`, `trunk`, `develop`, `staging`, `release/*`, `hotfix/*`) are never rebased. A conflict aborts the rebase and leaves the worktree on the branch as created. AI conflict resolution is not wired in the binary yet (#446).
7. Hands the rest to the background.

## Background upkeep

A detached, silent process (`playbook cc housekeep`) runs these best-effort steps:

1. `git fetch --prune`.
2. Fix a stale upstream tracking ref, then fast-forward pull when the remote branch exists.
3. `git worktree prune`.
4. Set upstream: track the remote branch if it exists, else `git push -u <remote> <branch>`, which starts CI without a manual push. Skip the push with `--no-push` or `WORKTREE_NO_PUSH=1`.
5. Reuse `node_modules` (below).
6. `playbook worktree sweep`: remove worktrees of any creation convention that have landed and are not locked or in use. It is gated by `worktreeCleanup.enabled`, and a new worktree is protected from it.

### node_modules

When `package-lock.json` hashes match between the base repo and the worktree, `node_modules` is cloned copy-on-write: `cp -cR` on APFS, `cp -R --reflink=auto` on GNU filesystems, a plain `cp -R` as the last fallback. Each worktree gets an independent tree. Then a bounded `npm install --prefer-offline --no-audit --no-fund` reconciles extra packages. When the hashes differ, nothing is cloned.

## The daily sweep

`maybe_sweep_worktrees` (`src/hooks/session_init.rs`) also runs the sweep on every Claude Code `SessionStart`, at most once per 24 hours per repo (a row under `worktree-sweep/<repo slug>` in the [state store](08-state-store.md)). So a repo is swept when you create a worktree and again the first time a session opens each day. Both call the same function. The overlap is deliberate.

A repo nobody opens is swept by neither trigger. That is a known limit. Fixing it needs an OS scheduler, which nothing here uses.

Settings: `worktreeCleanup.enabled`, `worktreeCleanup.staleAfterDays` and `worktreeCleanup.conflictGracePeriodDays`. See [Config keys](../guides/04-config-keys.md).

## Claude Code worktree hooks

`playbook hook worktree-create` and `playbook hook worktree-remove` are registered in `hooks/hooks.json` for the `WorktreeCreate` and `WorktreeRemove` events. They stop `claude --worktree`, `isolation: "worktree"` agents, and background sessions from creating worktrees under `.claude/worktrees/`.

**Create.** The hook reads `cwd` and `worktree_name` (or `name`) from stdin, finds the main worktree of the repo at `cwd`, and targets `<main-parent>/.worktrees/<repo>/<name>` (or the `WORKTREE_BASE_DIR` base, like the launcher), using the same `main_worktree` and `resolve_base` the launcher uses. The name must be a single safe path segment, and `worktree-<name>` must be a valid branch name: empty names, `..`, slashes, a leading `-`, and anything git rejects are refused. If the target is already a registered worktree of the repo, it is reused; a registered entry whose folder is gone is pruned and re-added. Otherwise the hook runs `git worktree add -b worktree-<name> <target> <base>`, matching Claude Code's default branch name. The base is the payload's `base_commit`, then the cached `origin/HEAD`, then local `HEAD`. When the payload gives no `base_commit`, the hook first refreshes `origin/HEAD`, like Claude Code does: it fetches the default branch if neither `FETCH_HEAD` nor the `playbook-origin-fetch` stamp in the git directory is under 24 hours old, waits at most 5 seconds, never prompts for credentials, and keeps the cached ref when the fetch fails. For a newly created worktree (not a reused one) the hook also copies the files that match a pattern in the main checkout's `.worktreeinclude` and are ignored by git, so local files such as `.env` follow the worktree. Tracked files are never copied, an existing file is never overwritten, symlinks are skipped, and a path that leaves the repo is ignored. Stdout carries only the absolute path; all git output goes to stderr. Because the path follows the launcher convention, `classify` reports it as `CcLauncher` and the sweep cleans it once its work has landed.

If anything fails, the hook says why on stderr and falls back to `<repo>/.claude/worktrees/<name>` with the same branch and base. It exits non-zero only if the fallback fails too.

**Remove.** The hook acts only on a registered worktree of the repo and never on the main worktree. It keeps the worktree, prints a reason on stderr, and exits 0 when the tree has uncommitted or untracked changes, or when any commit is not reachable from a remote-tracking ref or the local default branch. Otherwise it runs `git worktree remove` without `--force`, then deletes the branch only if its name starts with `worktree-` (so a launcher branch is never touched) and it passes the same reachability rule.

## See also

- [Launcher and hooks](01-launcher-and-hooks.md): the `ccc` launcher that sets up the worktree.
- [Docs index](../index.md)
