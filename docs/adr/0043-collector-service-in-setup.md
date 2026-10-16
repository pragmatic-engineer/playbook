# ADR-0043: Keep the Usage Collector Running With a Per-User Service

- **Status:** Accepted
- **Date created:** 2026-10-17
- **Date modified:** 2026-10-17
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
2. **One service per system.** On macOS it writes the launchd agent `~/Library/LaunchAgents/io.github.pragmatic-engineer.playbook.collector.plist` with `RunAtLoad`, `KeepAlive`, `ThrottleInterval` 30 and `ProcessType` `Background`, then runs `launchctl enable` and `launchctl bootstrap gui/<uid>`. On Linux it writes the systemd user unit `~/.config/systemd/user/playbook-collector.service` with `Restart=always` and `RestartSec=30`, then runs `systemctl --user daemon-reload`, `enable` and `restart`. On any other system it writes nothing and prints `playbook usage collect --idle-seconds 0` to run by hand. The service runs `<playbook> usage collect --idle-seconds 0`, so it never idles out. `<playbook>` is the copy on `PATH` that is the running binary, so a Homebrew upgrade keeps working, else the binary's own path.
3. **No secret in the service file.** The file names only the program, its arguments and the log path. A program or log path that is not UTF-8 or holds a control character is refused, so a line break cannot start a new directive. The collector reads its token from `collector.json`, as it does for every other start. A test checks that no token, header or `OTEL_` name is in either file.
4. **Modes and logs.** The service file is `0644`, as both managers expect, and is written through a temp file and a rename. Its output goes to `collector-service.log` in the folder of `collector.json` (`~/.config/playbook/data`), which setup creates `0600` before the manager opens it.
5. **Idempotent.** A file that matches what setup would write is not written again. When it matches and the service runs, the step reports no change. When it matches and the service is stopped, setup starts it. When it matches and the service runs a collector from another playbook version, as after an upgrade, setup restarts it with `launchctl kickstart -k` or `systemctl --user restart`; `collector.json` records the version of the collector that runs. When the program path changed, setup rewrites the file, boots out a loaded launchd agent and waits for launchd to let go of it before it loads the new one. For launchd, the service counts as running only when `launchctl print` says `state = running`. A service manager that refuses to start it leaves the file in place: the step reports the reason and the command that starts it. On macOS, launchd also loads an agent in that folder at the next login.
6. **Tests never load a real service.** Every `launchctl` and `systemctl` call goes through a `Manager` trait. The real manager is used only when `HOME` is the home that the account database lists for the user. Any other `HOME`, as in every test, gets a manager that runs nothing and says why, so a test writes the file into its scratch home and never loads it.
7. **Uninstall, doctor and status.** `playbook uninstall` stops the service and removes its file before it touches the binary, and `--dry-run` only names it. `playbook doctor`, when telemetry points at the collector, shows a row for the service: PASS when installed, WARN when the program it starts is gone, INFO with `playbook setup --only collector-service` when there is none. The collector row hint names `playbook usage collect` and the service command. `playbook usage status` adds a `background service:` line.
8. **Setup says when the collector is down.** After a run, when telemetry points at the collector, no collector answers and this run did not install or keep the service, the "Next steps" list starts with a line that names `playbook usage collect` and `playbook setup --only collector-service`.
9. **Migration 0016 reminds existing installs once.** `0016-collector-service` is `Idempotent` with `Scope::Consent`. Its check reports when telemetry points at the collector, the system has launchd or systemd, no service file exists and the user has not said no. Its command is `playbook setup --only collector-service`, and its run step writes nothing. The check records `collector-service/reminded` in the `state` table the first time it reports, so the session hook shows the line once and stays quiet after that. This is the one `Consent` check that writes, and it writes only playbook's own state. A no to the question, `--no-collector-service`, is stored as `collector-service/declined`, and a later yes clears it.
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
