# ADR-0047: Guard Free Disk Space While Agents Build

- **Status:** Accepted
- **Date created:** 2026-10-19
- **Date modified:** 2026-10-19
- **Builds on:** [ADR 0045](0045-rm-guard-roots-and-parser.md), whose rm guard lets the model delete `.worktrees/cargo-*` and `/tmp/pb-*`, and [ADR 0046](0046-migrations-by-release-version.md), which says how a new migration is added.

## Context

On 2026-10-18 the disk reached 100% with 0 GB free. Many agents were building Rust projects at the same time. Each one had its own cargo target folder of 0.5 to 2 GB, plus its own worktree. Every Bash call then failed with `ENOSPC`, `df` included. One agent stopped because deleting files needed Bash too. Space came back only after someone deleted stale `.worktrees/cargo-*` folders by hand.

Two facts made it worse than a slow build:

- Claude Code writes tool output and hook scratch files to disk. At 0 bytes free the shell itself cannot start, so the model cannot run the cleanup that would fix it. Any guard has to act well before 0.
- Nothing in playbook knew how much space was left, and nothing removed a target folder once its worktree was gone.

The user keeps a floor of 6 GB free by hand today.

On 2026-10-19 the disk fell from 34 GB to 0 GB free again while this work was in progress. Most of the space went to cargo target folders of other projects on the same volume, which playbook did not create and could not name.

## Decision

1. **Five config keys, per user.** `disk.warnFreeGb` (default 15), `disk.blockFreeGb` (default 6, the user's own floor), `disk.autoCleanup` (default `true`), `disk.paths` (default empty) and `disk.maxTargetDirs` (default 4). Free space belongs to the machine, so all five live in the global tier, like `update.mode`. Thresholds are whole GB as `df -g` counts them (1024³ bytes), from 0 to 10000. A warn value below the block value is raised to it. `disk.blockFreeGb` at 0 turns blocking off. `disk.paths` holds extra absolute or `~/` paths, at most 16. A bad stored value falls back to its default, so a broken config store does not turn the guard off. The model cannot change `disk.*`: the policy guard refuses it, the same as `hooks.rmGuard.*`, since a model that hits the floor could otherwise lower it.

2. **One cheap probe.** `disk::probe::space` calls `statvfs(2)` once and reports `f_bavail` times `f_frsize`, the space an ordinary user can write. It runs no subprocess and takes a few microseconds (the test checks under 5 ms). A process keeps a reading for 5 seconds, so `disk status` and the doctor read each volume once. A hook is a new process on every tool call, so it reads fresh. A cache file would cost more than the call it saves, and it cannot be written on a full disk. The guard measures the home folder's volume, the current repo's volume, the temp folder's volume (where Claude Code writes its scratch files), an absolute `WORKTREE_BASE_DIR` (where worktrees and target folders grow) and each `disk.paths` entry. It counts each volume once and judges by the one with the least free space.

3. **A `disk-guard` hook on `PreToolUse` for `Bash`, `Agent` and `Task`.**
   - Under `disk.blockFreeGb` it denies a Bash command that has a build-class command in command position: `cargo build`, `test`, `check`, `clippy`, `nextest`, `run`, `bench`, `doc` and `install`, `rustc`, `npm`, `pnpm` and `yarn` install, add and build, `pip install`, `docker build`, `brew install` and `upgrade`, `git clone`, `git worktree add`, `make` (except `make clean`) and `go build`, `test` and `install`. It reads the command with the shared shell lexer, so a word inside quotes, a commit message or a heredoc is data. It looks behind wrappers such as `env`, `sudo`, `timeout` and `nohup`, and inside `sh -c`.
   - Every other command passes. Reads, `git status`, `rm`, `cargo clean`, `git worktree remove`, `df`, `du` and every `playbook` command stay allowed, so the model can still recover. The rm guard still judges each `rm` on its own.
   - Under `disk.blockFreeGb` it also denies the `Agent` and `Task` tools, since a new agent creates a worktree and a target folder.
   - The denial says how many GB are free, the threshold, the cleanup the model may run (`playbook disk clean`, `cargo clean` in a finished worktree, `git worktree remove` of a merged worktree), and that the user can change the threshold with `playbook config set disk.blockFreeGb <gb>`.
   - Under `disk.warnFreeGb` it adds a warning to the model's context, at most once every 10 minutes per session.
   - With `disk.autoCleanup` on, the first denial in a 10 minute window starts `playbook disk clean --auto` in the background.
   - `SessionStart` prints the warning under `disk.warnFreeGb`. With `disk.autoCleanup` on, it also starts `playbook disk clean --auto` in the background at that level.
   - The hook writes no file to decide. It writes its throttle stamp after it has decided. On a nearly full disk that write can fail, and then the throttle does not hold: the warning repeats and each denial may start another background cleanup. `disk clean` takes an exclusive lock on a file under `~/.config/playbook` and exits at once when another run holds it, so only one cleanup runs at a time.

4. **A ballast file, off by default.** We decided the ballast is worth it. `playbook disk reserve` writes a 1 GB file of real blocks at `~/.cache/playbook/ballast/ballast.bin`. The folder carries a `CACHEDIR.TAG`, and on macOS `tmutil addexclusion` keeps the file out of Time Machine. `playbook setup` offers it and creates it only on a yes. `playbook disk release` deletes it. The guard hook also deletes it when free space falls below 1 GB, which is when tool calls start to fail, and its message says so and names `playbook disk reserve` to put it back after the cleanup. The release is one `unlink`, which needs no free space. `reserve` refuses when it would leave less than `disk.warnFreeGb` free.

5. **A safe cleanup.** `playbook disk clean [--dry-run]` removes only what playbook can prove is disposable, and prints what it freed:
   - Cargo target folders named `cargo-*` directly under a worktrees root. A folder is skipped when a cargo process holds its build lock, when any running process names it on its command line, or when anything under it changed in the last 30 minutes.
   - `/tmp/pb-*` and `/private/tmp/pb-*` entries owned by the user and untouched for 60 minutes.
   - The contents of `~/.config/playbook/tmp` and `~/.cache/playbook`, except the ballast.
   - Landed worktrees, through the existing `playbook worktree` sweep and its `worktreeCleanup.*` keys. The sweep removes a worktree whose branch merged into the default branch, and also one whose branch has had no commit for `worktreeCleanup.staleAfterDays` (30 by default). It skips a dirty worktree, a locked one and one in use, and it keeps the branch, so every commit stays in git.
   - Plugin cache folders that Claude Code marked orphaned (`.orphaned_at`, found by `plugin_root::orphaned_cache_dirs`) more than a day ago.

   It never deletes uncommitted work, a branch, the memory store, `config.db` or any path the rm guard protects. Each target passes the rm guard's protected-path check before it goes.

6. **`playbook disk status`**, with `--json`, prints the free space per volume, the thresholds, and the biggest playbook-owned consumers with sizes: target folders, worktrees, caches and the ballast. It also lists cargo target folders of any project under the home folder and `disk.paths`, found by the `CACHEDIR.TAG` and `.rustc_info.json` cargo writes. That scan only reads, goes at most 5 folders deep and reads at most 20,000 folders, skips hidden folders other than `.worktrees` and `~/Library`, and sizes what it found for at most 5 seconds. A folder it could not finish sizing shows its size so far. The guard measures free space on the volume and does not care which project used it, so a build in another project still triggers the warning and the block. The doctor adds a row that is PASS at or above `disk.warnFreeGb`, WARN below it and FAIL below `disk.blockFreeGb`. The status line shows a red low-disk warning below `disk.warnFreeGb`.

7. **Private target folders, removed with their worktree.** An agent that builds Rust in a worktree sets `CARGO_TARGET_DIR=<worktrees root>/cargo-<worktree name>`. The agent instructions say so. When playbook removes a worktree, through the worktree-remove hook, `playbook worktree remove` or the sweep, it also deletes the matching `cargo-<name>` folder. The guard warns when more than `disk.maxTargetDirs` target folders exist. To share compiled crates across agents, set `RUSTC_WRAPPER=sccache` with a cap such as `SCCACHE_CACHE_SIZE=10G`.

   We chose private folders over one shared folder per repo. Cargo holds a lock on a target folder for the whole build, so parallel agents on one shared folder wait for each other. Agents on different branches also overwrite each other's build outputs and rebuild. Private folders cost space, and decisions 3, 5 and 7 bound that cost.

8. **Wiring.** The hook goes in the shipped settings template. Existing installs get it through a `Consent` migration that asks to run `playbook setup --hooks`, added with `since: "next"` as ADR 0046 decision 9 says. Setup does not edit `settings.json` without that consent.

## Alternatives considered

- **One shared target folder per repo.** Rejected for the lock and rebuild reasons in decision 7.
- **A cache file for the probe.** Rejected. `statvfs` is cheaper than reading a file, and the file cannot be written when the disk is full.
- **Block every Bash command under the floor.** Rejected. The model then cannot clean up, which is how the 2026-10-18 agent got stuck.
- **Run the full cleanup inside the hook.** Rejected. Measuring and removing folders can take many seconds, longer than a hook should hold a tool call. The hook starts it in the background.
- **Delete target folders by age alone.** Rejected. A long build can leave a folder untouched for minutes while a process still holds it. The lock and process checks cover that.
- **No ballast.** Rejected. The 2026-10-18 failure had no way back from inside the session. The ballast gives back 1 GB with one `unlink`.

## Consequences

- New builds stop at 6 GB free by default, with room left for the shell, hook files and cleanup.
- A build that started before space fell under the floor keeps running. Several agents that each start a 0.5 to 2 GB build at 6.5 GB free can still reach 0. The ballast, when the user said yes to it, is the fallback for that case.
- An agent that hits the floor gets a message that names the fix, and the cleanup it may run stays open.
- Target folders go away with their worktrees, and stale ones are found by `disk clean`.
- The ballast costs 1 GB on machines whose user said yes. An APFS local snapshot taken after the ballast was written keeps its blocks until the snapshot goes, so a release may free less than 1 GB on such a machine.
- A build started outside Claude Code, or by a command the classifier does not know, is not stopped. The block list covers the build tools named in decision 3.
- Not verified: Windows, where the probe reports nothing and the guard lets every call through.
