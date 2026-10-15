# ADR-0033: Turn Claude Code Telemetry On by Default at Install

- **Status:** Accepted
- **Date created:** 2026-10-15
- **Date modified:** 2026-10-15

## Context

`playbook usage` is most useful when Claude Code sends OpenTelemetry data to a collector on the same machine. Before this decision, `playbook init --telemetry` was opt in and its question defaulted to no, so most installs had no telemetry rows. Even a user who wanted it often got nothing:

- Claude Code applies the `env` block of `~/.claude/settings.json` over the shell, so exporting variables in a shell does nothing for a plain `claude`.
- A settings file can already hold `CLAUDE_CODE_ENABLE_TELEMETRY=0`, `DISABLE_TELEMETRY=1` and `DO_NOT_TRACK=1`. Init left all of it alone.
- Nothing started the collector for sessions that did not go through `playbook cc`, so the data was dropped.

Claude Code documents `DISABLE_TELEMETRY` and `DO_NOT_TRACK` as opting out of Anthropic's own analytics and error reporting. It documents `CLAUDE_CODE_ENABLE_TELEMETRY` as the switch for export to your own collector and says it "doesn't control Anthropic's analytics". So only the enable switch gates OpenTelemetry (OTLP) export.

## Decision

1. **The telemetry question defaults to yes.** It is still an explicit confirmation. `--yes` counts as a yes. A run with no terminal and no `--yes` writes nothing and prints `playbook init --only telemetry`.
2. **What is written.** Eight keys in the `env` block of `~/.claude/settings.json`: `CLAUDE_CODE_ENABLE_TELEMETRY=1`, `OTEL_LOGS_EXPORTER=otlp`, `OTEL_METRICS_EXPORTER=otlp`, `OTEL_TRACES_EXPORTER=none`, `OTEL_EXPORTER_OTLP_PROTOCOL=http/json`, `OTEL_EXPORTER_OTLP_ENDPOINT=http://127.0.0.1:<port>`, `OTEL_EXPORTER_OTLP_HEADERS=Authorization=Bearer <token>` and `OTEL_METRICS_INCLUDE_ACCOUNT_UUID=false`. Prompt, tool detail, tool content and raw body logging are never turned on. The file is backed up first, a record of what was written goes to `~/.config/playbook/telemetry-env.json`, and `playbook uninstall` removes exactly those keys and puts back any value they replaced, unless the user changed a key since.
3. **Off switches.** A lone `CLAUDE_CODE_ENABLE_TELEMETRY` off value is replaced only after an answer to the question, `--telemetry` or `--only telemetry`. `--yes` alone leaves it and prints the command. `DISABLE_TELEMETRY` and `DO_NOT_TRACK` are listed and left as they are, because they do not block OTLP export. A user's own endpoint or any other telemetry variable is never overridden. Variables in the process environment that only mirror settings.json do not count as a second setup, because Claude Code copies its settings env into the commands it runs.
4. **`--only`.** `playbook init --only <part>[,<part>]` runs just the named parts. A bare `--telemetry` (no `--yes` and no other part flag) means `--only telemetry`, so a flag named after one part does not run the whole install.
5. **The port is chosen once.** The collector records its port in `collector.json`: 47318 by default (the standard OTLP port 4318 is avoided because other collectors use it), or a free port when that is taken on the first run. Every later start binds the same port and retries for up to a second while a killed predecessor lets go. When another program owns it, the start fails with a message and the port is never changed silently, because the endpoint in Claude Code settings names it. Moving to a new port is `playbook usage collect --port <N>` then `playbook init --only telemetry`.
6. **The collector starts from SessionStart.** This needs the `session-init` hook, which is part of the hooks init wires; without it the collector is started by `playbook usage collect` or `playbook cc`. When settings point at the playbook collector and it is down, the `session-init` hook starts it detached. A spawn lock keeps parallel sessions to one collector, the wait is at most 1.2 seconds, the hook never fails, and the collector's messages go to `collector.log`. If another program already answers on the port, nothing is spawned.
7. **Doctor reports three checks:** settings point at the collector, the collector runs, and rows arrived in the last day. Each failing row names the command that fixes it.
8. **Claude Code must be restarted** after init writes the settings. Init says so at the end.

## Alternatives considered

- **Keep it opt in.** Rejected: most installs would have no data, and the feature that needs it would stay empty.
- **Clear `DISABLE_TELEMETRY` and `DO_NOT_TRACK` too.** Rejected: they control Anthropic's analytics, not export to a local collector, and a user set them on purpose.
- **A new random port on every start.** Rejected: the endpoint in settings would go stale at each restart.
- **A system service for the collector.** Rejected: it needs privileges and an install step that `uninstall` cannot cleanly undo. The session hook is enough, and the collector stops by itself after ten idle minutes.

## Consequences

- A fresh install on a terminal turns telemetry on unless the user says no.
- A user with an off switch keeps it unless they say yes. `--yes` is not that yes.
- Telemetry sent while no collector listens is lost. The first message of a session may be sent before the hook has started the collector; transcripts remain the authority for totals.
- The collector stops after ten idle minutes, and the hook runs only at session start. A session left idle that long loses export until the next session start, and is read from transcripts.
- The recorded port survives restarts. A machine that recorded 4318 earlier keeps it.
- Settings written by init can go stale only when the user moves the port; `playbook doctor` says so and `init --only telemetry` repairs it.
