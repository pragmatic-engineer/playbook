# ADR-0050: Start the Usage Collector From Playbook Commands and Keep It Running

- **Status:** Accepted
- **Date created:** 2026-10-19
- **Date modified:** 2026-10-20
- **Supersedes:** [ADR 0043](0043-collector-service-in-setup.md), which kept the collector alive with a launchd agent or a systemd user unit that setup installed.
- **Amends:** [ADR 0041](0041-retire-playbook-telemetry.md), whose context describes how the collector starts; [ADR 0044](0044-drop-the-shell-launcher.md), decision 5, which lists the `collector-service` part; and [ADR 0046](0046-migrations-by-release-version.md), which names the retired `collector-service` and `collector-service-label` ids.

## Context

`playbook setup --only telemetry` points Claude Code at a collector on `127.0.0.1`. ADR 0043 added a per-user service to keep that collector alive, because the collector that `playbook cc` and the `session-init` hook started stopped after 600 idle seconds, and a session with the hooks off started nothing.

The service cost more than it gave:

- It was 2436 lines in three files (`collector_service.rs`, `service_manager.rs`, `service.rs`), plus a setup part, two migrations, doctor rows and docs. Every change to it needed a fake launchd in the tests and a guard that keeps tests away from the real `launchctl`.
- The 0.37.0 label was wrong, and moving it took a migration of its own (`collector-service-label`, the ADR 0043 amendment).
- The service and the hook collector fought over the port. A service start that found a hook collector exited, and launchd tried again every 30 seconds until that collector idled out.
- Liveness was a guess: a nonzero pid, `kill(pid, 0)` and a TCP connect, then a header probe. The hook spawn used a `create_new` lock file with a 10 second stale rule and detached with `process_group(0)` only.
- The receiver polled a non-blocking `accept` every 25 ms and served one connection at a time. That is too busy for a process that never exits, and one slow client blocked every other session.

On 2026-10-19 the maintainer decided to remove the service and keep one long-lived collector that playbook itself starts on demand.

## Decision

1. **One collector per OS user, marked by a lifetime lock.** `playbook usage collect` takes an exclusive `File::try_lock` on `~/.config/playbook/data/collector.lock` (mode `0600`) before it opens the database or binds the port, and holds it until it exits. It then writes `<pid> <version>\n` into the file through the locked handle. A line without the newline means a start in progress. The OS drops the lock on any exit, so a held lock means a live collector and there is no stale state. A second start finds the lock busy and returns `collector already running` with exit 0 before it binds anything or writes `collector.json`. A busy lock is retried 3 times, 50 ms apart, and up to 1 second while the holder line is incomplete, because a starter may hold a shared probe lock for a moment. The lock is the first local of `run_collect_with`, so it drops last, after the listener closes and after `collector.json` records pid 0.

2. **A threaded receiver with one writer thread.** The accept loop blocks, with no sleep. Each connection gets its own thread with 5 second read and write deadlines, and at most 64 connections are live. The next one gets 503. The 50 requests per second limit is shared by all threads. Parsed rows go over a bounded channel (256 jobs) to one writer thread that owns the SQLite connection. The writer drains up to 64 jobs or 50 ms, commits them in one transaction and answers each connection after the commit, so 200 and 503 keep their meaning. A full channel answers 503 at once. The writer also runs the 6 hour prune. When the writer thread is gone or panics, the collector answers 503, stops and frees the lock, so the next starter replaces it instead of keeping a collector that stores nothing.

3. **Starters check the lock and start a detached collector.** `daemon::lock_state` opens `collector.lock` without creating it and takes one shared `try_lock`. A missing file counts as free, so `doctor` and `--version` create nothing. When the lock is free, `daemon::ensure` takes the respawn stamp `collector.spawned` and spawns `playbook usage collect --idle-seconds 0` with stdin and stdout closed, stderr appended to `collector.log` (mode `0600`), the data folder as its working directory and `setsid`, so it leaves the hook's session and process group. The stamp holds the next spawn off for 10 seconds. It is checked and rewritten under an exclusive lock on `collector.spawned`, so 16 parallel starters spawn once. When the collector cannot bind its port because another program holds it, it writes a 300 second stamp with the reason `port <N> in use by another program`, which doctor prints.

4. **Three triggers.** The `session-init` hook asks for a collector, waits up to 1.2 seconds for it to listen and sends a header probe to a running one. The two hooks wired only on `UserPromptSubmit`, `auto-model-detect` and `context-hygiene`, and most playbook commands, `cc` and `usage status` included, ask with no wait and no network call. These start nothing: every other hook, the hot-path commands (`statusline`, `version`, `--version`, `json`, `commands`), `doctor`, `update`, `uninstall`, `migrate`, `setup`, `init`, `state`, and `usage collect`, `warm`, `prune` and `stop`. No `PreToolUse` or `PostToolUse` hook starts anything.

5. **Gates, cheapest first.** A trigger stops at the first gate that fails: the home folder is usable, `collector.json` has a port and a token, the run is not headless (`PLAYBOOK_HEADLESS` or `CI`), and `HOME` is the account's own home. Then it checks the lock. A holder at this version with no probe is the end: one open and one `try_lock`. Only when a spawn or a hand-over could follow does it read `usage.telemetry` (it must be `auto`) and check consent: the Claude Code user settings point telemetry at this collector, the state the telemetry setup part writes. The `usage.telemetry` lookup can open `config.db`, so it runs after the lock check, which keeps the prompt hook cost within 0.33 ms of main at p99 (hyperfine, 200 runs).

6. **Hand-over after an upgrade, and a hung collector.** A holder line with another version is asked to stop through the authenticated `POST /v1/playbook/shutdown` and replaced once the lock frees. A prompt or a command waits at most 400 ms for that. The session hook waits 2 seconds, and it also replaces a collector that holds the lock but does not answer the probe. Past its budget, and only from the session hook or `playbook usage stop`, the starter sends `SIGKILL` after these checks: the lock is still held, the holder line and `collector.json` name the same pid, the pid is not the starter, and the pid's executable file name is `playbook` (with ` (deleted)` removed on Linux, since an upgrade replaces the binary under a running collector). A collector from 0.38.0 or older holds no lock. When the lock is free but such a collector answers the probe on the recorded port, the starter sends `SIGTERM` after the same pid checks, waits for the port to close, then spawns.

7. **Stop.** `playbook usage stop` sends the shutdown request, waits up to 3 seconds and then may `SIGKILL` as above. It prints `stopped` or `not running`, both with exit 0. The shutdown route checks the same bearer token as the other routes. A wrong token gets 401 and the collector keeps serving. `--idle-seconds` stays for a collector you run by hand. The daemon starts with 0, so it does not idle out.

8. **Migration and retirement.** A new `Owned`, `Idempotent` migration, `collector-service-removed`, stops and deletes every service file playbook wrote: the launchd agent under the current label and the 0.37.0 label, and the systemd user unit. It deletes the `collector-service/declined` and `collector-service/reminded` keys, and when it removed a file it starts a collector with no wait, so open sessions keep a receiver after `bootout` stops the service's collector. A file under the old label without playbook's marker stays, and the warning names the commands that remove it. In a home that is not the account's own, it changes nothing and says so. `collector-service` and `collector-service-label` leave the registry and join `RETIRED_IDS`, so old records load. The `collector-service` setup part is gone. `--collector-service`, `--no-collector-service` and `--only collector-service` stay accepted, hidden, and print `the collector service is no longer needed; playbook starts the collector itself`, as ADR 0044 did for `--aliases`. Doctor shows one row for the daemon (pid, version, lock state) and one WARN row with `playbook migrate` while a service file is still installed. `playbook uninstall` still removes leftover service files.

9. **Test-only variables, read only in debug builds.** `PLAYBOOK_COLLECTOR_STAMP_SECS` sets the stamp window, `PLAYBOOK_COLLECTOR_TEST_VERSION` sets the version in the holder line and `PLAYBOOK_COLLECTOR_TEST_HANG` makes connection threads read a request and never reply. The real spawner removes all three from the child's environment. A debug build that cargo or nextest started spawns and replaces nothing unless `PLAYBOOK_COLLECTOR_TEST_SPAWN=1`, because `setup --yes` in a test home writes a `collector.json` and any later command would leave a collector running for good.

## Alternatives considered

- **Keep the current start path and only delete the service.** The smallest diff. Rejected: liveness stays a pid and connect guess with a race between the check and the spawn, there is no hand-over after an upgrade, and the 25 ms poll stays in a process that no longer exits. It could not show at most one collector after 1000 parallel starts.
- **A heartbeat row in `usage.db` as the liveness check.** Rejected: staleness depends on time, the row means a write every few seconds, and the port bind would still be the real mutex.
- **A `Mutex<Connection>` shared by the connection threads.** Rejected: one writer thread gives batched commits and keeps SQLite on one thread.
- **A header probe on every trigger.** Rejected: it adds up to a 1 second timeout to prompt hooks. Only the session hook, which already waits, probes.
- **No respawn stamp.** Rejected: a crash loop or a foreign program on the port would spawn a collector on every prompt.

## Consequences

- After a reboot or a logout, Claude Code telemetry is dropped until the next playbook command or hook starts the collector. Those sessions are read from transcripts.
- A plain `claude`, the IDE or the desktop app with the hooks part off starts nothing. Their telemetry reaches a collector only when a playbook command started one, or when you run `playbook usage collect --idle-seconds 0` yourself.
- Windows is not supported. On a non-unix target `ensure` returns `NotSupported` and `stop` returns `NotRunning`, so a collector runs only when started by hand.
- Nobody has checked by hand that a collector spawned with `setsid` from a `session-init` hook survives Claude Code's hook timeout handling and the end of the session. The tests run the real binary, not a Claude Code session.
- The first prompt or command after an upgrade can wait up to 400 ms for the hand-over. It happens once per upgrade.
- `playbook uninstall` removes leftover service files but does not stop a running collector. Run `playbook usage stop` first.
- A pid can be reused between the checks and the `SIGKILL`. The window is microseconds and needs a `playbook` process to take the old pid.
- The service code shrinks to its removal paths: `collector_service::remove`, `service::find` and `find_old`, the labels and the manager's `remove`. They go once no install is left with a service file.
