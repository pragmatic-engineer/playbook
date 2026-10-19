# ADR-0042: Turn Off Claude Code Attribution in Setup

- **Status:** Accepted. Amended by [ADR 0044](0044-drop-the-shell-launcher.md), which makes `--yes` turn on every setup part, and by [the 2026-10-19 amendment](#amendment-2026-10-19-one-attribution-part-with-two-states), which renames the part to `attribution` with an ON and an OFF state and retires the `attribution-off` migration.
- **Date created:** 2026-10-17
- **Date modified:** 2026-10-19
- **Amends:** decisions 8 and 12 of [ADR 0034](0034-self-healing-upgrades.md), for this one key: `playbook doctor --fix` asks about it before the shared heal step, and may write it after a yes at a terminal. The 2026-10-19 amendment also amends decisions 7 and 9 of [ADR 0046](0046-migrations-by-release-version.md): a slug can be retired from the registry, and is never given to a new migration.

## Context

Claude Code adds attribution to the commits and pull requests it writes: a `Co-Authored-By` trailer, a generated-by line in pull request text, and, in cloud and Remote Control sessions, a `Claude-Session` trailer and a claude.ai session link. Playbook already removes these after the fact. The shipped system prompt tells Claude to leave them out, and the `commit-message-sanitizer` hook strips any that slip into a commit, tag, merge or pull request message. Both act after Claude Code has asked for the lines. Claude Code has a setting that stops it from adding them at all.

The settings reference (https://code.claude.com/docs/en/settings-reference, checked on 2026-10-17) says:

- `attribution` is an object with `commit` and `pr` strings and a `sessionUrl` Boolean. Its scope is any settings file.
- An empty `commit` hides the commit trailer. An empty `pr` hides the pull request line. `sessionUrl: false` omits the session link, which Claude Code adds as a `Claude-Session` trailer on commits and as a link in pull request text.
- `attribution: false` hides all three, but needs Claude Code 2.1.281 or later. Earlier versions reject it and skip the whole settings file that holds it. For a file that earlier versions also read, the reference asks for the three sub-keys instead.
- `includeCoAuthoredBy` is deprecated since 2.0.62. Claude Code still honors `includeCoAuthoredBy: false`, and ignores it once `attribution.commit` or `attribution.pr` is set.

So `{"commit": "", "pr": ""}` alone leaves the session link in place. The value has to include `"sessionUrl": false`.

## Decision

1. **A new setup part, `attribution-off`.** It sets `attribution` to `{"commit": "", "pr": "", "sessionUrl": false}` in `~/.claude/settings.json`. It is asked in a terminal, on by default, and `--yes` counts as a yes. Without a terminal and without `--yes` it stays off, like the other parts that write Claude Code settings. The flags are `--attribution-off` and `--no-attribution-off`, and `--only attribution-off` runs it alone.
2. **Only missing keys are added.** Each of the three sub-keys is added when it is missing. A sub-key that holds another value is the user's own and stays, unless the user answered yes to the question or passed `--attribution-off`. `--yes` and `--only attribution-off` never replace it, so the command the reminder names is always safe to run. `playbook setup --only attribution-off --attribution-off` replaces it. `attribution: false` counts as off. Any other shape is left alone with one line. The write keeps every other setting, keeps a sorted file sorted, keeps a symlinked file a symlink, and saves `settings.json.bak.<time>` first, as the telemetry part does.
3. **Migration 0015 tells existing installs.** `0015-attribution-off` is `Idempotent` with `Scope::Consent`. Its check reports a missing sub-key in an existing `~/.claude/settings.json`, and its command is `playbook setup --only attribution-off`. Its run step writes nothing, because only the setup part knows the answer to the question. Under ADR 0034 the `session-init` hook and the first run step name the command once per version change, and the hook repeats it each session until the key is set. A no to the question, `--no-attribution-off`, or a no in `doctor --fix` is stored as `attribution/declined` in the `state` table, and the check stays quiet from then on, across upgrades. A later yes clears it. A run that asked nothing, such as one with no terminal and no `--yes`, stores nothing, so the reminder stays. Setup runs the part before it writes the version record, so the record matches the answer.
4. **Doctor.** `playbook doctor check` shows one row: PASS when attribution is off, WARN with `playbook setup --only attribution-off` when a key is missing, INFO when a key holds the user's own value. `playbook doctor --fix` first asks once at a terminal when a key is missing and the user has not said no before, with no as the default, then runs the heal as before. A yes adds the missing keys and keeps the user's own values. Without a terminal it writes nothing, and the heal report names the command.
5. **The system prompt keeps its rule.** `prompts/SYSTEM_PROMPT.md` already tells Claude to add no trailer, session link or footer. The reference says a rule in the user's own instructions takes precedence over the attribution lines, so the rule and the setting work together. The sanitizer hook stays as the backstop for a session that runs without either.

## Alternatives considered

- **Write `"attribution": false`.** Rejected: Claude Code before 2.1.281 skips the whole user settings file, which would turn off every playbook hook and the status line.
- **Write only `commit` and `pr`.** Rejected: the session link is behind `sessionUrl`, so `Claude-Session` trailers would stay.
- **Set `includeCoAuthoredBy: false`.** Rejected: it is deprecated, and it does not cover the session link.
- **Let the hook or the first run step write the key.** Rejected: ADR 0034 decision 8 keeps settings writes behind consent. They name the command, and setup or `doctor --fix` asks.
- **Rely on the sanitizer alone.** Rejected: it only edits messages that pass through git and `gh` commands it sees, and it runs after Claude has written the lines.

## Consequences

- A new install with `--yes` or the default answers gets no Claude attribution from Claude Code itself.
- An existing install sees `playbook setup --only attribution-off` in the session message after the upgrade, until the key is set or the user says no.
- A project `.claude/settings.json`, a local settings file or managed settings can still set attribution for that scope. Playbook reads and writes only the user file.
- `playbook uninstall` keeps the key and prints one line that says so. The setup summary and `--explain` say to delete it by hand, and a backup sits next to the file.

## Amendment 2026-10-19: one attribution part with two states

The part was named for one state, so `--no-attribution-off` read as a double negative. It now has one name and two states. This replaces decisions 1, 3 and 4, the parts of decision 2 that name the old flags, the yes to the question or the reminder, and the second and fourth Consequences bullets.

1. **The part is `attribution`.** `--attribution` is ON: Claude Code adds its attribution, and setup removes each `attribution` sub-key that holds exactly the off value. Any other value stays, and with no key setup changes nothing. `--no-attribution` is OFF, the default: setup writes the off value as before and replaces a value of the user's own. `--yes`, `--only attribution` and a no to the question give OFF and keep the user's own values. The question asks for ON, with No as the default.
2. **The choice is stored.** A choice of ON or OFF is stored in the per-user key `attribution.enabled` (default false), as `launcher.shortName` stores its answer, so `--yes` and `--only attribution` keep an ON. A stored `attribution/declined` row from 0.36 or 0.37 counts as ON until a value is stored. A no in `playbook doctor --fix` stores ON.
3. **The `attribution-off` migration is retired.** `attribution-off` is no longer in the registry: no consent item, no session reminder and no `playbook migrate` row. A record that names it, as `attribution-off` or `0015-attribution-off`, still loads, and the id is ignored. `0015-attribution-off` stays in the fixed table of old ids from [ADR 0046](0046-migrations-by-release-version.md), and `attribution-off` is listed as retired so no new migration takes the slug.
4. **Doctor.** The row is PASS for OFF with every key set, or for ON by choice with no off value left. It is WARN with `playbook setup --only attribution` when OFF is the choice and a key is missing, and when ON is the choice and a key still holds the off value. It is INFO for a value of the user's own. `playbook doctor --fix` still asks at a terminal when OFF is the choice and a key is missing: a yes adds the missing keys and keeps the user's own values, and a no stores ON.
5. **Old names keep working.** `--attribution-off` maps to `--no-attribution`, `--no-attribution-off` maps to `--attribution`, and the part name `attribution-off` in `--only` and `--explain` maps to `attribution`, as hidden names with one stderr note at a terminal, as in [ADR 0040](0040-rename-init-to-setup.md). The note for `--no-attribution-off` prints without a terminal too, because that flag now writes settings. `--no-attribution-off` beside an `--only` that does not name the part stays a no-op, as in 0.37. In 0.37 `--no-attribution-off` left settings alone; it now turns attribution ON, which removes the off value.
6. **Uninstall** keeps the key and names `playbook setup --only attribution --attribution` as the way to get Claude Code's attribution back.
