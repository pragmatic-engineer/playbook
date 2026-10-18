# ADR-0036: A Release Check and Automatic Updates

- **Status:** Accepted. Decisions 6 and 7.7 are amended by [ADR 0037](0037-release-notes-from-conventional-commits.md), which reads `release-notes.json` in place of the ADR 0035 record. The amendment of [ADR 0044](0044-drop-the-shell-launcher.md) on 2026-10-19 removes `shell-init`, so decision 2 no longer lists it among the hot paths.
- **Date created:** 2026-10-15
- **Date modified:** 2026-10-16
- **Extends:** [ADR 0034](0034-self-healing-upgrades.md), whose migrations run after a binary update, and [ADR 0035](0035-breaking-change-notices.md), whose record tells the user about breaking changes.

## Context

A user learns about a new playbook release only by running `playbook update --check` or by reading the release page. Claude Code checks for its own updates and tells the user. Playbook should do the same, and it should be able to install a release on its own for users who want that.

Every playbook command, hook and status line redraw runs the same binary. Hooks and the status line run many times a minute and write machine output, so a check there would slow Claude Code and could corrupt its input. Some commands print JSON for other programs. A release check must stay out of all of these, and it must never make a command wait on the network.

## Decision

1. **One setting.** The config key `update.mode` takes `notify` (the default), `auto` or `off`. The key `update.checkIntervalHours` (default 24) sets how long a result stays fresh. Both are settings for each user: `playbook config set` writes them to the global tier even without `--global` and refuses `--org`, `playbook config import` refuses them outside the global section, and the check reads only the global tier.
2. **Where the check runs.** `main` calls `update::check::on_start` after the home check and after the first run migration step of ADR 0034. It returns at once for the commands `Command::is_hot_path` names (`hook`, `statusline`, `shell-init`, `version` and `json`), for `update`, for the background `usage warm` and `usage collect` runs that the session hooks start, for any run with a `--json` argument, and when `CI`, `PLAYBOOK_HEADLESS` or a CI vendor variable is set. When stderr is not a terminal it returns unless the mode is `auto` and the binary is a release build. It reads the mode for that test through a read-only connection, so a piped run that stops there writes nothing.
3. **No waiting.** The check reads one row, `update-check/cache`, from the `state` table of `config.db`. When the row is older than the interval, the command starts `playbook update --refresh-check` as a detached process in its own process group and goes on. That process asks the unauthenticated GitHub releases API for one page of 100 releases with a 10 second limit, and stores the newest published `vX.Y.Z` release and its publish time. Drafts, release candidates, plugin tags and telemetry tags do not count. A failed request stores nothing. The start time recorded before the process started makes the next try wait one full interval, so the check stays silent offline, when rate limited and behind a proxy. The request sends no credentials; its only identifying header is the `playbook/<version>` user agent that GitHub requires.
4. **Claims.** A command claims the start of the detached process and the notice inside one transaction against the current row. When two commands start together, only one starts the process and only one prints. A command that starts the detached process for an automatic update records the version and the time, so a command starts an attempt for one version at most once a day. The detached process claims the attempt itself before it installs. For 15 minutes after a start the attempt counts as running, and no command starts another process or prints the notice. An attempt that has not claimed its version by then, for example on an offline machine, falls back to the notice.
5. **The notice.** When the cached release is newer than the running binary, a command in a terminal prints one line on stderr: `playbook X.Y.Z is available (you have A.B.C). Run: playbook update`. A Homebrew install sees `brew upgrade playbook`. In `notify` mode, on an install that can update itself, the line also names `playbook config set --global update.mode auto`. The line shows at most once in 24 hours for each version.
6. **Automatic updates.** In `auto` mode the detached process installs the release through the same path as `playbook update`: the checksum, the build attestation, a backup and an atomic swap, then the new binary's migrations. With no person present, an attestation that cannot be checked stops the install, and the release falls back to the notice under 7.8. The command that started it keeps running on the old binary, and the next run uses the new one. The process adds a report to the row, and a new failure warning replaces an earlier one for the same version. The next command in a terminal prints `playbook updated from A.B.C to X.Y.Z`, the breaking change block of ADR 0035 and any warnings, then clears it. A failed refresh stops before the update, so an offline machine stays silent.
7. **When `auto` falls back to the notice.** The notice shows instead, with the reason, when:
   1. Homebrew manages the binary, so playbook does not replace it;
   2. `playbook update` does not support the platform, such as Windows;
   3. the binary is a development or release candidate build;
   4. the release is less than an hour old, so a release pulled right after it ships reaches nobody on its own, or its publish time is unknown;
   5. the install directory is not writable;
   6. a manual `playbook update` holds `update.lock`, which both paths take; auto tries again after an hour;
   7. the release's own `breaking-changes.json` asset lists an entry for a version the update crosses whose action is not `none, migrated automatically`, or the asset does not match its SHA256SUMS line, or it holds a playbook entry this binary cannot read, or the asset or SHA256SUMS cannot be downloaded. An entry that needs the user holds that version for good; a record that cannot be read or downloaded is tried again after a day. A release without the asset, which releases before ADR 0035 lack, has no known entries;
   8. the build attestation cannot be checked, for example when `gh` is missing; auto tries again after a day.
8. **Failures.** A failed automatic update leaves one warning with the reason and the fix, `playbook update` or `update.mode` set to `notify`. It never fails the command the user ran.
9. **Status.** `playbook update --check` and `playbook doctor` show the mode, the time the last background check started, the release it found and, in `auto` mode, the reason a newer release is held back. Both read `config.db` through read-only connections. `playbook init` does not ask about the mode; the install guide names the command that turns on automatic updates.

## Alternatives considered

- **A check with a short time limit inside the command.** Rejected: even a 1.5 second limit slows a command once a day, and a slow link would fail every time. The detached process costs the command nothing.
- **Install in the foreground before the command runs.** Rejected: a download and an attestation check take seconds, and a failure would block the command the user asked for.
- **Ask GitHub for the release marked latest.** Rejected: plugin and telemetry releases share the repository, so the release marked latest is often not a binary release.
- **Use `gh` when it is installed.** Rejected for now: the unauthenticated API is enough for one request a day, and it sends no account details.
- **A question in the `playbook init` wizard.** Rejected: the wizard asks about parts it installs, each with its own flags. A three value preference does not fit that list, and the default needs no answer.

## Consequences

- Users learn about a release within a day, without a step of their own, and users who choose `auto` stay current.
- A command in a terminal reads `config.db` once more at start. A piped command of a release build reads one row through a read-only connection, so it can find `auto`.
- Playbook now makes one network request a day by default. `playbook config set update.mode off` turns it off, and the docs that list network use say so.
- The update path now has a lock, so `playbook update` refuses to start while an automatic update runs.
- A real swap of a published release cannot run in CI. Tests cover the swap with a fake release endpoint.
