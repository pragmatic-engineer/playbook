# ADR-0043: Keep the Usage Collector Running With a Per-User Service

- **Status:** Superseded by [ADR 0050](0050-collector-daemon-started-by-commands.md). Earlier amended by [ADR 0044](0044-drop-the-shell-launcher.md), which makes `--yes` turn on every setup part, and on 2026-10-18 to change the launchd label, see [the amendment](#amendment-2026-10-18-the-launchd-label).
- **Date created:** 2026-10-17
- **Date modified:** 2026-10-20
- **Amends:** [ADR 0041](0041-retire-playbook-telemetry.md), whose context says no launchd or systemd unit starts the collector.

## Context

`playbook setup --only telemetry` writes the OpenTelemetry variables into `~/.claude/settings.json`. They point Claude Code at `http://127.0.0.1:<port>`, port 47318 by default, with a bearer token in `OTEL_EXPORTER_OTLP_HEADERS`. Something has to listen on that port. Today three things start `playbook usage collect`:

- `playbook cc`, for the sessions it starts, when `usage.telemetry` is `auto` and a usage database exists.
- The `session-init` hook, when the hooks part is on.
- The user, by hand, in a terminal or a tmux session.

The first two start a collector that stops after 10 idle minutes. A plain `claude`, the IDE extension or the desktop app with the hooks off starts nothing. Claude Code drops telemetry that it cannot send, so the pbtop Tools view and `playbook usage` stay empty and `playbook usage status` says `collector: not running`. The setup summary never mentions this. `docs/internals/04-usage-dashboard.md` told users to write their own launchd agent or systemd unit.

The collector already keeps its port and token in `collector.json` (owner-only) and reads them on every start. A second start finds the running collector and exits with status 0.

## Decision

1. **A new setup part, `collector-service`.** It sits right after `telemetry` in the part list. It is asked in a terminal, on by default, and `--yes` counts as a yes. Without a terminal and without `--yes` it stays off, like the other parts that change how Claude Code sessions behave. The flags are `--collector-service` and `--no-collector-service`, and `--only collector-service` runs it alone. The part acts only when the Claude Code user settings point telemetry at the playbook collector. Otherwise it skips with one line that names `playbook setup --only telemetry`. When the service file is already in place and its program exists, setup prints `collector-service: already installed as ...` and does not ask; a run keeps the file up to date.
2. **One service per system.** On macOS it writes the launchd agent `~/Library/LaunchAgents/io.github.pragmatic-engineer.playbook.collector.plist` with `RunAtLoad`, `KeepAlive`, `ThrottleInterval` 30 and `ProcessType` `Background`, then runs `launchctl enable` and `launchctl bootstrap gui/<uid>`. The [2026-10-18 amendment](#amendment-2026-10-18-the-launchd-label) changes the label and the file name. On Linux it writes the systemd user unit `~/.config/systemd/user/playbook-collector.service` with `Restart=always` and `RestartSec=30`, then runs `systemctl --user daemon-reload`, `enable` and `restart`. On any other system it writes nothing and prints `playbook usage collect --idle-seconds 0` to run by hand. The service runs `<playbook> usage collect --idle-seconds 0`, so it never idles out. `<playbook>` is the copy on `PATH` that is the running binary, so a Homebrew upgrade keeps working, else the binary's own path.
3. **No secret in the service file.** The file names only the program, its arguments and the log path. A program or log path that is not UTF-8 or holds a control character is refused, so a line break cannot start a new directive. The collector reads its token from `collector.json`, as it does for every other start. A test checks that no token, header or `OTEL_` name is in either file.
4. **Modes and logs.** The service file is `0644`, as both managers expect, and is written through a temp file and a rename. Its output goes to `collector-service.log` in the folder of `collector.json` (`~/.config/playbook/data`), which setup creates `0600` before the manager opens it.
5. **Idempotent.** A file that matches what setup would write is not written again. When it matches and the service runs, the step reports no change. When it matches and the service is stopped, setup starts it. When it matches and the service runs a collector from another playbook version, as after an upgrade, setup restarts it with `launchctl kickstart -k` or `systemctl --user restart`; `collector.json` records the version of the collector that runs. When the program path changed, setup rewrites the file, boots out a loaded launchd agent and waits for launchd to let go of it before it loads the new one. For launchd, the service counts as running only when `launchctl print` says `state = running`. A service manager that refuses to start it leaves the file in place: the step reports the reason and the command that starts it. On macOS, launchd also loads an agent in that folder at the next login.
6. **Tests never load a real service.** Every `launchctl` and `systemctl` call goes through a `Manager` trait. Setup calls `launchctl` or `systemctl` only when `HOME` is the home that the account database lists for the user. Any other `HOME`, as in every test, gets a manager that runs nothing and says why, so a test writes the file into its scratch home and never loads it.
7. **Uninstall, doctor and status.** `playbook uninstall` stops the service and removes its file before it touches the binary, and `--dry-run` only names it. `playbook doctor`, when telemetry points at the collector, shows a row for the service: PASS when installed, WARN when the program it starts is gone, INFO with `playbook setup --only collector-service` when there is none. The collector row hint names `playbook usage collect` and the service command. `playbook usage status` adds a `background service:` line.
8. **Setup says when the collector is down.** After a run, when telemetry points at the collector, no collector answers and this run did not install or keep the service, the "Next steps" list starts with a line that names `playbook usage collect` and `playbook setup --only collector-service`.
9. **Migration 0017 reminds existing installs once.** `0016-collector-service` is `Idempotent` with `Scope::Consent`. Its check reports when telemetry points at the collector, the system has launchd or systemd, no service file exists and the user has not said no. Its command is `playbook setup --only collector-service`, and its run step writes nothing. The check writes nothing, as decision 6 of ADR 0034 requires. After the session hook, the first-run step, `playbook migrate` or `doctor --fix` prints the open items, the heal records `collector-service/reminded` in the `state` table, so the line shows once and later checks stay quiet. `playbook migrate --status` and `--dry-run` do not record it, nor does a headless session (CI or `claude -p`), nor setup, which asks the question itself. A no to the question, `--no-collector-service`, is stored as `collector-service/declined`, and a later yes clears it.
10. **pbtop says which cause applies.** The empty state of the Agents, Skills, Tools and Health views is now short lines: the cause first (telemetry off, the collector not running, or no rows yet with the collector running), then each command on its own line. A short panel shows the cause and the first command.

## Alternatives considered

- **Start the service from the telemetry part without a question.** Rejected: a login item is a change to the machine a person should agree to, and some users run the collector their own way.
- **Write the token into the service file.** Rejected: the token is a secret, and the collector already reads it from an owner-only file.
- **`KeepAlive` with `SuccessfulExit` false.** Rejected: a collector that `cc` or the hook started holds the port, so a service start exits with status 0 and launchd would then never start it again after that collector idles out. With `KeepAlive` true and a 30 second throttle, the service takes over within 30 seconds.
- **Repeat the reminder every session until answered.** Rejected: the collector is useful but not needed for playbook to work, and the setup summary and `doctor` already say when it is down.
- **Honor `XDG_CONFIG_HOME` for the unit.** Not done: playbook keeps its own files under `~/.config` and a user who moves the systemd folder can place the unit there.

## Consequences

- A new install with the default answers keeps a collector running after every login, so telemetry from a plain `claude`, the IDE and the desktop app is stored.
- An existing install with telemetry on sees one session line that names `playbook setup --only collector-service`.
- One more process runs per user. It listens on loopback only and does nothing while no session sends data.
- A service start that finds a `cc` or hook collector exits at once and launchd or systemd tries again 30 seconds later, until that collector idles out. A collector you run yourself with `--idle-seconds 0` never idles out, so the service tries every 30 seconds and adds one line to its log each time. Run one or the other.

## Amendment 2026-10-18: the launchd label

**Context.** Playbook 0.37.0 shipped the launchd label `io.github.pragmatic-engineer.playbook.collector`. That was a mistake: the maintainer chose `com.github.pragmatic-engineer.playbook.collector` as the label for the collector service. Machines that ran setup on 0.37.0 already have `~/Library/LaunchAgents/io.github.pragmatic-engineer.playbook.collector.plist` loaded.

**Decision.**

1. **The label is `com.github.pragmatic-engineer.playbook.collector`,** and the agent file is `~/Library/LaunchAgents/com.github.pragmatic-engineer.playbook.collector.plist`. The label is defined once, as `LABEL` in `crates/telemetry/src/service.rs`. The setup text builds the file name from the same literal. The old label is kept as `OLD_LABEL`, only to find and remove the old agent.
2. **The systemd unit keeps its name, `playbook-collector.service`.** It never held the reverse-DNS string, and systemd user units are named by plain words, so Linux needs no move. The log file keeps its name, `collector-service.log`.
3. **Migration `0020-collector-service-label` moves the old agent.** It is `Idempotent` with `Scope::Owned`, so the session hook, the first-run step, `playbook migrate`, `doctor --fix` and setup run it without a question. When the old agent file exists and carries playbook's comment and the old label, it:
   1. runs `launchctl bootout gui/<uid>/io.github.pragmatic-engineer.playbook.collector`, where a job that is not loaded is fine, and waits up to two seconds for launchd to let go of it. If launchd still knows the old job, it stops here and loads nothing.
   2. writes the new agent with the program the old file named, or this binary when that program is gone, and the same log, then loads it.
   3. deletes the old file, and only that file.
   4. waits up to three seconds for `launchctl print` to say `state = running`, or for a collector to answer.
   The old job stops before the new one starts, so two collectors never compete for port 47318.
4. **A failed step leaves the machine as it was and says how to fix it.** A new agent that is loaded but not running is booted out before the move, so at most one agent is loaded. When the new agent does not start, it is booted out, its file goes back to what it was, and the old agent is loaded again if it was loaded before. The migration then fails with the reason and `to try again, run: playbook migrate`, so the heal records the failure and `playbook doctor` names `playbook migrate`. When the new agent runs but the old file cannot be deleted, the migration fails the same way, and a later run deletes it. When the collector does not come up after the move, the migration fails with the log path and `launchctl kickstart -k gui/<uid>/com.github.pragmatic-engineer.playbook.collector`.
5. **A file playbook did not write is never touched.** An `io.github.pragmatic-engineer.playbook.collector.plist` without playbook's comment and label is only reported, with `launchctl bootout` and `rm` commands to remove it by hand.
6. **It runs launchctl only for the account's own home, and never from a debug build that cargo started.** With any other `HOME` it moves nothing, and it prints a note when an old agent playbook wrote is there. While this change was built, a test started a debug `playbook` with the real `HOME`, and the first-run heal moved the author's real agent. The test isolation net from the same change set now covers this path: `service_manager::manager_for` hands out no real `launchctl` or `systemctl` when `common::real_home` refuses, and the migration then prints a note instead. Tests use a fake launchd that tracks the loaded labels.
7. **A machine that already has the new label is fine.** With no old file the migration reads one path and does nothing, and `doctor` shows the new agent as installed.
8. **Setup moves the old agent too.** The collector-service step calls the same move before it installs, so a setup run after a failed migration never loads a second agent.
9. **Doctor, status and uninstall know the old label.** `playbook doctor` shows a WARN row for an old agent file with `run: playbook migrate`, whatever telemetry says, because launchd loads that file at the next login. `playbook usage status` names an old agent as the background service with `run playbook migrate to move it`. `playbook uninstall` removes both the new and the old agent file.

**Why `Owned` and automatic.** The agent is playbook's own file, written by playbook setup and marked as such, and the move keeps the program, the arguments and the log. The step is idempotent: with no old file it reads one path and runs nothing. It undoes itself on failure: a failed start puts the machine back as it was. A downgrade to 0.37.0 is not a clean reversal: that setup writes the old agent next to the new one, so run `launchctl bootout gui/<uid>/com.github.pragmatic-engineer.playbook.collector` and remove its file first, or upgrade again and let this migration move it back. Asking first would leave the old label in place on every machine until the user answers.

**Alternatives considered.**

- **Keep the old label.** Rejected: the maintainer chose the `com.github` label, and the old one shipped in a single release, so few machines need the move.
- **A `Consent` migration that setup applies.** Rejected: the change is to playbook's own file and asks nothing of the user, and a question would keep two names in use for longer.
- **Rename the systemd unit to the reverse-DNS name too.** Rejected: the unit never used it, and renaming it would add a Linux move with no gain.
