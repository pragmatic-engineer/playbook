# Disk use of agent worktrees

Every agent worktree that builds Rust grows its own cargo `target` folder. On 2026-10-19 sixteen worktrees held 1.5 to 6 GB each, about 45 GB in total, and the full disk corrupted `target/debug/incremental` and stalled agents with `ENOSPC`. Design: [ADR 0047](../adr/0047-disk-guard.md).

## What keeps it small

- **One private, small build per worktree.** Agents run `eval "$(playbook disk target-env)" && cargo test` in the same Bash call. The line sets `CARGO_TARGET_DIR` to `<worktrees root>/cargo-<name>-<hash>`, and sets `CARGO_INCREMENTAL=0`, `CARGO_PROFILE_DEV_DEBUG=0` and `CARGO_PROFILE_TEST_DEBUG=0`. Panics keep their file and line. Backtraces and debuggers show only function names.
- **Set per command, not in `.cargo/config.toml`.** Your own builds keep incremental compilation and debug info. Run the `eval` line in your shell, or set the variables yourself, when you want the same for a build you start by hand.
- **Removed with the worktree.** `playbook worktree remove`, the sweep and the worktree-remove hook delete the folder with its worktree.
- **Removed when idle.** `playbook disk clean` also removes a `cargo-*` folder and the `target` folder inside a linked worktree once nothing in it changed for 30 minutes. It keeps a folder when a cargo build holds its lock or a running process names it. The main worktree is never touched.
- **Warned about.** `playbook disk status` warns when target folders hold more than `disk.maxTargetGb` together. `playbook disk target-dir` and `target-env` warn on stderr when free space is below `disk.warnFreeGb`, or more than `disk.maxTargetDirs` folders exist. They never refuse. The `disk-guard` hook still stops builds and new agents below `disk.blockFreeGb`.

## Measured

`cargo test --workspace --no-run` on this workspace, macOS arm64, clean target folder each time, while other builds ran on the machine, so times are rough:

| Setting | Target folder | Time |
| --- | --- | --- |
| cargo defaults | 3.0 GB | 61 s |
| `CARGO_INCREMENTAL=0` | 1.9 GB | 68 s |
| and debug info `line-tables-only` | 1.6 GB | 54 s |
| and debug info `0` (what `target-env` sets) | 1.1 GB | not comparable |
| debug info `0`, incremental on | 2.1 GB | 65 s |

Debug info and incremental data are the two big parts. Turning off both cuts the folder by about 63%. `line-tables-only` keeps line numbers in backtraces at 1.6 GB.

Two options were not adopted as defaults. A shared `CARGO_TARGET_DIR` makes parallel agents wait for cargo's build lock. `sccache` shares compiled dependencies across target folders but does not shrink each one, does not cache incremental builds, and is an extra install. If it is installed, `RUSTC_WRAPPER=sccache` with `SCCACHE_CACHE_SIZE=10G` works alongside the settings above.

## Knobs

| Key | Default | Effect |
| --- | --- | --- |
| `disk.warnFreeGb` | 15 | Warn below this much free space. |
| `disk.blockFreeGb` | 6 | The hook stops builds and new agents below this. |
| `disk.maxTargetDirs` | 4 | Warn when more `cargo-*` folders exist. |
| `disk.maxTargetGb` | 20 | Warn when all cargo target folders together hold more. |
| `disk.autoCleanup` | `true` | Run `playbook disk clean` on its own when space is low. |

Set one with `playbook config set <key> <value>`. To see where the space went, run `playbook disk status`. To free it, run `playbook disk clean --dry-run`, then `playbook disk clean`.
