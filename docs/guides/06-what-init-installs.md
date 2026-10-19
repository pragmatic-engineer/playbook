# What setup installs

`playbook setup` changes files on your machine. This page lists every change, what each part is for, how to say no, and how to undo it. Read it before you answer the questions `setup` asks. The per-hook detail is in the [hook reference](../reference/hooks.md).

## See it before you say yes

- In a terminal, `playbook setup` is a guided wizard. It shows one screen per part ("Step 3 of 8"), with what happens if you accept and what happens if you decline, a mark for the recommended answer, and the keys y, n, b (back), s (use the default) and q (quit). A summary and a confirm come last. Nothing is written before you confirm.
- `--yes` (or `--defaults`) skips the wizard and takes the defaults. Each part also has a `--x` and `--no-x` flag, and a flag wins over the wizard and shows in the summary as set by a flag.
- Without a terminal there are no questions. Only `PATH` is on: hooks, the status line, the system prompt, the `pb` link, telemetry, and the collector service stay off, and attribution is left as it is, unless you pass `--yes` or their own flag.
- `playbook setup --dry-run` prints exactly what would change for each part and changes nothing.
- `playbook setup --explain` describes every part. `playbook setup --explain <part>` describes one, for example `--explain hooks`.
- `playbook hooks list` prints every hook with its event, matcher and one line of purpose.
- After a real run, `setup` prints a summary of what changed and how to undo each part.

All of these read from the same tables in the code, and a test keeps the [hook reference](../reference/hooks.md) in line with them.

## What the wizard looks like

```text
Step 3 of 8  Path  (recommended: yes)
------------------------------------------------------------
Add the playbook binary's directory to PATH in your shell startup files?

If you accept:
  + Puts the playbook binary on PATH so hooks find it.
  + Adds a block that starts with `# playbook binary` and one `export PATH=...` line.
  + Undo: `playbook uninstall --remove-binary` removes the block.

If you decline:
  - Your shell files are not touched. Hooks may not find playbook unless it is already on PATH.

Do this? [Y/n, b back, s skip, q quit]
```

After the summary, each action prints one line, then the totals and the next steps:

```text
Applying
------------------------------------------------------------
[ok] hooks  wired the ported hooks into settings.json
[=] statusline  already up to date
[skip] short-name  playbook is not installed in ~/.local/bin, so no link was added; to add one, run: ln -s ...
[fail] trust  could not write ~/.claude.json

1 done, 1 unchanged, 1 skipped, 1 failed
```

Before this wizard, the command (then called `init`) printed a flat plan and one yes or no question per part, then lines like `hooks: wired - ...`.

## Output in a terminal and in a pipe

- `uninstall`, `update` and `doctor` print marked lines (`[ok]`, `[=]`, `[skip]`, `[fail]`, `[plan]`, `[warn]`, `[info]`) with a heading, totals and next steps when output goes to a terminal. A UTF-8 terminal gets symbols instead of the bracket marks.
- Color is off when `NO_COLOR` is set, `TERM=dumb`, or output is not a terminal. Text wraps to the terminal width, between 40 and 100 columns.
- A pipe, `doctor check --json` and the `/playbook:doctor` skill keep the plain text they read before.

## Privacy

- Playbook stores everything on your machine, under `~/.config/playbook/` and the files named below.
- Playbook sends nothing anywhere. No hook, and no part of `setup`, makes a network call. The commands that reach the network are `playbook update` (it downloads a release from GitHub), the background release check that `update.mode` controls (one unauthenticated request to the GitHub releases API every `update.checkIntervalHours`, 24 by default, whose only identifying header is the `playbook/<version>` user agent; with `update.mode` set to `auto` the same background process also downloads a newer release from GitHub and checks its attestation through the `gh` tool you are logged into; `playbook config set --global update.mode off` turns it off), and the status line, which calls the `gh` tool you already have to show PR and CI state.
- No hook reads a secret value. `precommit-check` looks at file names only, to flag names that look like credentials.
- `playbook usage` reads local Claude Code history. With `usage.telemetry` on `auto`, `playbook cc` points a session at a receiver on your own machine, and `setup --telemetry` can do the same for plain `claude`, the IDE and the desktop app. See [usage telemetry privacy](../internals/04-usage-dashboard.md#telemetry-privacy).
- Claude Code's own settings are yours. Playbook edits `~/.claude/settings.json` only when you agree to it in `setup`, and takes a timestamped backup first.

## The parts

Each part has a flag pair. With no flag, `setup` asks in a terminal and uses the default otherwise. `--yes` accepts every default, so it turns every part but the security defaults on. It keeps the system prompt off when `systemPrompt.enabled` is false or on an earlier install that had none, and the `pb` link off after a stored no. `--aliases` and `--claude-override` are retired: setup accepts them and does nothing, with a note on stderr when it is a terminal.

| Part | Default | Flags | Files it edits |
| :- | :- | :- | :- |
| Hooks | on | `--hooks`, `--no-hooks` | `~/.claude/settings.json`, `~/.claude/.settings.base.json` |
| Status line | on | `--statusline`, `--no-statusline` | `~/.claude/settings.json` |
| Security defaults | off | `--security`, `--no-security` | `~/.claude/settings.json` |
| PATH block | on | `--path`, `--no-path` | your shell startup files |
| System prompt | on, asked | `--system-prompt`, `--no-system-prompt` | `~/.config/playbook/prompts/SYSTEM_PROMPT.md`, and `~/.config/playbook` (the `systemPrompt.enabled` key) |
| Usage telemetry | on, asked | `--telemetry`, `--no-telemetry` | `~/.claude/settings.json` (the `env` block), `~/.config/playbook/telemetry-env.json` |
| Collector service | on, asked | `--collector-service`, `--no-collector-service` | `~/Library/LaunchAgents/com.github.pragmatic-engineer.playbook.collector.plist` (macOS) or `~/.config/systemd/user/playbook-collector.service` (Linux) |
| `pb` short name | on, asked | `--short-name`, `--no-short-name` | the `pb` symlink in the install folder, and `~/.config/playbook` (the `launcher.shortName` key) |
| Attribution | OFF, asked | `--attribution` (ON), `--no-attribution` (OFF) | `~/.claude/settings.json` (the `attribution` key), and `~/.config/playbook` (the `attribution.enabled` key) |

`--settings` and `--no-settings` no longer exist. Each option that part used to carry now sits behind the hooks, status line or security part, or is not shipped.

Three steps run every time and have no flag: the memory migration, the trust step and the retired config-hash cleanup. They are described below.

### Hooks

Setup adds 33 hook entries to `~/.claude/settings.json`. They run 25 different `playbook hook <name>` commands. The [hook reference](../reference/hooks.md) lists each one with its event, matcher, what it reads and writes, whether it can block a tool call, and how to turn it off.

The entries fall into six groups: safety guards, session lifecycle, usage tracking and auto mode, memory, hygiene, and worktree. Only a few hooks can stop a tool call: `rm-workspace-guard`, `no-slop-guard`, `policy-guard`, `disk-guard`, `preread-size-check`, and (in `auto` mode) `auto-guard` and `auto-cost`. `memory-capture` can pause a stop. `worktree-create` replaces Claude Code's own worktree creation. The rest add context or warn.

While the plugin is disabled in Claude Code, every hook and the status line pause. See the [hook reference](../reference/hooks.md#how-to-turn-a-hook-off).

Setup adds or updates its own entries only. A hook you added by hand, or an event group setup does not manage, stays as it is. Running setup twice changes nothing the second time.

### Status line and the settings merge

When the hooks, status line or security part is on, setup three-way merges the shipped `settings.shared.json` into `~/.claude/settings.json`. A key you set yourself is kept. A key you never touched takes the template value. The merge records its base in `~/.claude/.settings.base.json` so the next run can tell your edits from the template's.

The template includes:

- A `statusLine` entry that runs `playbook statusline`. It reads the session data Claude Code passes on stdin, the git state of the working directory, and PR and CI state through `gh` when `gh` is installed. It caches PR lookups under your cache directory. It writes nothing to your repo.
- The `hooks` block, which is the same guards as the Hooks part.
- The `permissions` block, only when you opt in to the security defaults.

The template ships nothing else. Earlier templates also carried display options, notification toggles and `env` entries such as telemetry opt-outs and `DISABLE_AUTOUPDATER`. No hook or launcher reads them, so they are gone. A value you already have stays as it is: setup never removes a key from your file. A fresh install gets none of them, so Claude Code keeps its own defaults and your update control stays yours (`playbook update` and Homebrew).

When a merge skips a key because you changed it, setup writes a `settings-merge-skipped.<time>.json` report next to the backup. Setup keeps the five newest backups and reports.

### Security defaults

Off unless you pass `--security` or set `security.defaults` to true. When on, setup also applies the template's `permissions` block (allow, ask and deny lists). Nothing else changes, and update control (the Claude Code autoupdater) stays with you, `playbook update` and Homebrew. When off, setup leaves your value exactly as it is, and never removes one you already have. To go back, run `playbook setup --no-security` and delete the permissions entries from `~/.claude/settings.json` by hand.

### PATH block

Setup adds a block that starts with `# playbook binary` to your shell startup files, so hooks find the `playbook` binary. Hook commands run in a non-interactive shell that never reads `.zshrc`, so zsh gets `~/.zshenv`, bash gets `~/.bashrc` and its login file, and fish gets its own `conf.d` file. Other shells get `~/.profile`. Open a new terminal afterward. `playbook uninstall --remove-binary` removes the block.

### No shell line

Setup writes no line to your `.zshrc` or `.bashrc`. `playbook cc`, `playbook ccd`, `pb cc` and `pb ccd` start Claude Code from the binary with the playbook system prompt, the model chain and your effort ceiling. `playbook ccd` also skips permission prompts, so use it only where you would use `--dangerously-skip-permissions`. After a session in a new worktree, `playbook cc` prints the `cd` command to run. Older setups wrote a launcher line: `playbook migrate` lists it, and a full `playbook setup` removes it after a backup (the `retired-launcher-rc-line` migration). A hand-written `eval "$(playbook shell-init)"` goes too: the command is removed, and until 1.0 it only prints a line on stderr asking you to delete the rc line. See [the launcher](../internals/01-launcher-and-hooks.md).

### System prompt

`--system-prompt` (on by default; `--yes` is a yes unless the prompt was turned off or never installed, as above) copies `SYSTEM_PROMPT.md` to `~/.config/playbook/prompts/` and stores `systemPrompt.enabled` as true. Claude Code uses it only when you start it through `pb cc`, `playbook cc` or `playbook ccd`. A plain `claude` never loads it. A no, or `--no-system-prompt`, stores false, so `pb cc` starts with Claude Code's own prompt, and leaves an existing copy in place. Run setup again with a yes to turn it back on. If you already have a copy, setup refreshes it on every run, unless you edited it, in which case your copy stays. An earlier install without the prompt gets a `playbook migrate` item (the `system-prompt-default-on` migration) until you answer.

`--telemetry` (on by default; `--yes` is a yes) writes the OpenTelemetry variables that point Claude Code at a collector on 127.0.0.1 into the `env` block of `~/.claude/settings.json`, so plain `claude`, the IDE and the desktop app feed `playbook usage` too. Setup first reads whether telemetry is on, off or not set, from your environment and every Claude Code settings file (user, project, local, managed). When it is already on, set up by you, or switched off by something setup will not change, setup says so in one line and asks nothing. When it is not set, or switched off by a lone off switch in `~/.claude/settings.json`, setup suggests turning it on as a yes or no question, and nothing is written without a yes. It writes nothing when your environment or any Claude Code settings file already names a telemetry variable, with one exception: a yes replaces an off switch that stands alone in `~/.claude/settings.json`. Any other setting you made, including values beside an off switch or in another file, is never overridden. Plain `--yes` never replaces an off switch you set; answer the question or run `playbook setup --only telemetry` for that. `DISABLE_TELEMETRY` and `DO_NOT_TRACK` are shown and left as they are, because they only turn off Anthropic's own analytics. Prompt text, tool content, emails and tokens are not sent, and the account id stays out of the metrics. The collector keeps one port (47318 unless that was taken on the first run) and the `session-init` hook starts it when settings point at it. Restart Claude Code after setup. `playbook uninstall` removes exactly the keys setup wrote, unless you changed them, and puts back an off switch that a yes replaced. See [telemetry by default](../internals/04-usage-dashboard.md#telemetry-by-default).

### The `pb` short name

`--short-name` asks whether `pb` should be a short name for `playbook`. `install.sh` and the Homebrew formula already make a `pb` symlink, so on those installs the part only checks it. Otherwise, when no other `pb` is on your PATH and playbook sits in the install folder (`$PLAYBOOK_BIN_DIR`, default `~/.local/bin`), setup adds the relative symlink `pb -> playbook` there, the same link `install.sh` makes. It never replaces a `pb` that runs something else, and when playbook lives anywhere else it prints the `ln -s` command to run by hand. The answer is stored as `launcher.shortName`, and a stored no keeps `--yes` from adding the link. `playbook uninstall --remove-binary` removes the link. See [the launcher](../internals/01-launcher-and-hooks.md#the-pb-short-name).

### Collector service

`--collector-service` (on by default; `--yes` is a yes) keeps the usage collector running, so the telemetry that the telemetry part points Claude Code at has a receiver after every login. It acts only when your Claude Code user settings point telemetry at the playbook collector. On macOS it writes a launchd agent to `~/Library/LaunchAgents/com.github.pragmatic-engineer.playbook.collector.plist` with `RunAtLoad` and `KeepAlive`, then loads it with `launchctl bootstrap`. An agent that playbook 0.37.0 wrote under the old label, `io.github.pragmatic-engineer.playbook.collector`, moves to the new one by itself on the next `playbook migrate`, session start or setup run. On Linux it writes a systemd user unit to `~/.config/systemd/user/playbook-collector.service` with `Restart=always`, then runs `systemctl --user enable` and `restart`. On other systems it writes nothing and prints `playbook usage collect --idle-seconds 0` for you to run. The service runs `playbook usage collect --idle-seconds 0` and writes its output to `~/.config/playbook/data/collector-service.log` (mode `0600`). The service file (mode `0644`) names only the program and the log: the collector reads its token from `collector.json`, so no secret is written. Running setup again changes nothing when the file is current, and starts the service when it is stopped. A no is remembered, so the one-time session reminder for existing installs stays quiet. `playbook doctor` and `playbook usage status` name the service, and `playbook uninstall` stops it and removes the file. See [ADR 0043](../adr/0043-collector-service-in-setup.md).

### Attribution

Attribution is what Claude Code adds to commits and pull requests: a `Co-Authored-By` trailer, a generated-by line, and a `Claude-Session` trailer or claude.ai link. The part has two states.

- OFF, the default (`--no-attribution`; a no to the question gives it too, and so does `--yes` unless you chose ON before): setup sets `attribution` to `{"commit": "", "pr": "", "sessionUrl": false}` in `~/.claude/settings.json`, so Claude Code adds none of it. Setup adds only the keys that are missing. A key that holds your own value stays, unless you pass `--no-attribution`.
- ON (`--attribution`, or a yes to the question): setup removes each key that holds exactly that off value, so Claude Code adds its attribution again. Any other value stays.

The choice is stored as `attribution.enabled`, so `--yes` and `--only attribution` keep an ON. A no to the old question in 0.36 or 0.37 counts as ON. `playbook doctor` shows a pass for OFF with every key set or for ON by your choice, a warning with `playbook setup --only attribution` when OFF is the choice and a key is missing, or when ON is the choice and a key still holds the off value, and info for a value of your own. `playbook doctor --fix` asks before it adds a missing key, and a no stores ON. A project or managed settings file can still set its own value. The old flags `--attribution-off` (now `--no-attribution`) and `--no-attribution-off` (now `--attribution`) and the part name `attribution-off` still work and print a note. See [ADR 0042](../adr/0042-attribution-off-in-setup.md).

### Memory migration

Early installs kept playbook's memory in `~/.claude/memory`. On the first run, setup moves that tree to `~/.config/playbook/memory` and renames `graph.json` to `memory.graph.json`. The move runs once and a `.migration-complete` marker records it. This is playbook's own store. Claude Code's auto memory under `~/.claude/projects/` is never touched. The `session-init` and `memory-capture` hooks run the same check, so a store left behind by a partial move heals itself.

### Trust step

If `~/.claude.json` exists, setup marks `~/.config/playbook` as trusted there, so a session started in that folder skips Claude Code's trust dialog. This is the one edit setup makes to `~/.claude.json`. If the file is missing or cannot be written, setup skips the step and reports it.

### Config-hash cleanup

Older installs placed `~/.config/playbook/hooks/lib/config-hash.sh`. The config hash now runs inside `playbook`, so setup removes that script once. The hash itself is a fingerprint of your Claude Code config files that `session-init` stores in the session directory, to warn when a resumed session's config has changed. It is never sent anywhere.

## Undo

- `playbook uninstall` removes the hook entries and status line setup wrote, the launcher block, the files setup placed under `~/.config/playbook`, and the collector service. Run it with `--dry-run` first to see the list. Add `--remove-binary` to remove the binary and its PATH block too.
- Memory, your own settings, sessions, history and earlier backups are never removed.
- Backups of `settings.json` are timestamped files next to it, named `settings.json.bak.<time>`.
- Any single part can be turned off by running `setup` again with its `--no-` flag, then removing the entries by hand. Setup never deletes a value you have.
