# What init installs

`playbook init` changes files on your machine. This page lists every change, what each part is for, how to say no, and how to undo it. Read it before you answer the questions `init` asks. The per-hook detail is in the [hook reference](../reference/hooks.md).

## See it before you say yes

- In a terminal, `playbook init` is a guided wizard. It shows one screen per part ("Step 3 of 8"), with what happens if you accept and what happens if you decline, a mark for the recommended answer, and the keys y, n, b (back), s (use the default) and q (quit). A summary and a confirm come last. Nothing is written before you confirm.
- `--yes` (or `--defaults`) skips the wizard and takes the defaults. Each part also has a `--x` and `--no-x` flag, and a flag wins over the wizard and shows in the summary as set by a flag.
- Without a terminal there are no questions: flags and defaults apply.
- `playbook init --dry-run` prints exactly what would change for each part and changes nothing.
- `playbook init --explain` describes every part. `playbook init --explain <part>` describes one, for example `--explain hooks`.
- `playbook hooks list` prints every hook with its event, matcher and one line of purpose.
- After a real run, `init` prints a summary of what changed and how to undo each part.

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
[skip] aliases  not installed; pass --aliases to opt in
[fail] trust  could not write ~/.claude.json

1 done, 1 unchanged, 1 skipped, 1 failed
```

Before this wizard, `init` printed a flat plan and one yes or no question per part, then lines like `hooks: wired - ...`.

## Output in a terminal and in a pipe

- `uninstall`, `update` and `doctor` print marked lines (`[ok]`, `[=]`, `[skip]`, `[fail]`, `[plan]`, `[warn]`, `[info]`) with a heading, totals and next steps when output goes to a terminal. A UTF-8 terminal gets symbols instead of the bracket marks.
- Color is off when `NO_COLOR` is set, `TERM=dumb`, or output is not a terminal. Text wraps to the terminal width, between 40 and 100 columns.
- A pipe, `doctor check --json` and the `/playbook:doctor` skill keep the plain text they read before.

## Privacy

- Playbook stores everything on your machine, under `~/.config/playbook/` and the files named below.
- Playbook sends nothing anywhere. No hook, and no part of `init`, makes a network call. The only commands that reach the network are `playbook update` (it downloads a release from GitHub) and the status line, which calls the `gh` tool you already have to show PR and CI state.
- No hook reads a secret value. `precommit-check` looks at file names only, to flag names that look like credentials.
- `playbook usage` reads local Claude Code history. With `usage.telemetry` on `auto`, `playbook cc` points a session at a receiver on your own machine, and `init --telemetry` can do the same for plain `claude`, the IDE and the desktop app. See [usage telemetry privacy](../internals/04-usage-dashboard.md#telemetry-privacy).
- Claude Code's own settings are yours. Playbook edits `~/.claude/settings.json` only when you agree to it in `init`, and takes a timestamped backup first.

## The parts

Each part has a flag pair. With no flag, `init` asks in a terminal and uses the default otherwise. `--yes` accepts every default.

| Part | Default | Flags | Files it edits |
| :- | :- | :- | :- |
| Hooks | on | `--hooks`, `--no-hooks` | `~/.claude/settings.json`, `~/.claude/.settings.base.json` |
| Status line | on | `--statusline`, `--no-statusline` | `~/.claude/settings.json` |
| Security defaults | off | `--security`, `--no-security` | `~/.claude/settings.json` |
| PATH block | on | `--path`, `--no-path` | your shell startup files |
| Shell wrapper for `playbook cc` and `playbook ccd` | off | `--aliases`, `--no-aliases` | your shell rc file |
| System prompt | off | `--system-prompt`, `--no-system-prompt` | `~/.config/playbook/prompts/SYSTEM_PROMPT.md` |
| Usage telemetry | off, asked | `--telemetry`, `--no-telemetry` | `~/.claude/settings.json` (the `env` block), `~/.config/playbook/telemetry-env.json` |
| `pb` short name | off, asked | `--short-name`, `--no-short-name` | `~/.config/playbook` (the `launcher.shortName` key) |
| `claude` function | off, asked | `--claude-override`, `--no-claude-override` | `~/.config/playbook` (the `launcher.claudeOverride` key) |

`--settings` and `--no-settings` no longer exist. Each option that part used to carry now sits behind the hooks, status line or security part, or is not shipped.

Three steps run every time and have no flag: the memory migration, the trust step and the retired config-hash cleanup. They are described below.

### Hooks

Init adds 31 hook entries to `~/.claude/settings.json`. They run 24 different `playbook hook <name>` commands. The [hook reference](../reference/hooks.md) lists each one with its event, matcher, what it reads and writes, whether it can block a tool call, and how to turn it off.

The entries fall into six groups: safety guards, session lifecycle, usage tracking and auto mode, memory, hygiene, and worktree. Only a few hooks can stop a tool call: `rm-workspace-guard`, `no-slop-guard`, `policy-guard`, `preread-size-check`, and (in `auto` mode) `auto-guard` and `auto-cost`. `memory-capture` can pause a stop. `worktree-create` replaces Claude Code's own worktree creation. The rest add context or warn.

While the plugin is disabled in Claude Code, every hook and the status line pause. See the [hook reference](../reference/hooks.md#how-to-turn-a-hook-off).

Init adds or updates its own entries only. A hook you added by hand, or an event group init does not manage, stays as it is. Running init twice changes nothing the second time.

### Status line and the settings merge

When the hooks, status line or security part is on, init three-way merges the shipped `settings.shared.json` into `~/.claude/settings.json`. A key you set yourself is kept. A key you never touched takes the template value. The merge records its base in `~/.claude/.settings.base.json` so the next run can tell your edits from the template's.

The template includes:

- A `statusLine` entry that runs `playbook statusline`. It reads the session data Claude Code passes on stdin, the git state of the working directory, and PR and CI state through `gh` when `gh` is installed. It caches PR lookups under your cache directory. It writes nothing to your repo.
- The `hooks` block, which is the same guards as the Hooks part.
- The `permissions` block, only when you opt in to the security defaults.

The template ships nothing else. Earlier templates also carried display options, notification toggles and `env` entries such as telemetry opt-outs and `DISABLE_AUTOUPDATER`. No hook or launcher reads them, so they are gone. A value you already have stays as it is: init never removes a key from your file. A fresh install gets none of them, so Claude Code keeps its own defaults and your update control stays yours (`playbook update` and Homebrew).

When a merge skips a key because you changed it, init writes a `settings-merge-skipped.<time>.json` report next to the backup. Init keeps the five newest backups and reports.

### Security defaults

Off unless you pass `--security` or set `security.defaults` to true. When on, init also applies the template's `permissions` block (allow, ask and deny lists). Nothing else changes, and update control (the Claude Code autoupdater) stays with you, `playbook update` and Homebrew. When off, init leaves your value exactly as it is, and never removes one you already have. To go back, run `playbook init --no-security` and delete the permissions entries from `~/.claude/settings.json` by hand.

### PATH block

Init adds a block that starts with `# playbook binary` to your shell startup files, so hooks find the `playbook` binary. Hook commands run in a non-interactive shell that never reads `.zshrc`, so zsh gets `~/.zshenv`, bash gets `~/.bashrc` and its login file, and fish gets its own `conf.d` file. Other shells get `~/.profile`. Open a new terminal afterward. `playbook uninstall --remove-binary` removes the block.

### Launcher shortcuts and shell-init

`--aliases` adds one line to your `.zshrc` or `.bashrc`: `eval "$(playbook shell-init)"`. That command prints one small `playbook` shell function, a few helpers it uses, and `ccc` and `ccd` stubs that tell you to run `playbook cc` and `playbook ccd`. For `playbook cc` and `playbook ccd` it calls `playbook cc launch`, which starts Claude Code with the playbook system prompt, the model chain and your effort ceiling. `playbook ccd` also skips permission prompts, so use it only where you would use `--dangerously-skip-permissions`. The functions use a temp file to move your shell into a worktree after a session. Older installs that sourced a script from the rc file get that line replaced with the `eval` line. See [the launcher](../internals/01-launcher-and-hooks.md).

### System prompt

`--system-prompt` copies `SYSTEM_PROMPT.md` to `~/.config/playbook/prompts/`. Claude Code uses it only when you start it through `playbook cc` or `playbook ccd`. If you already have a copy, init refreshes it on every run, unless you edited it, in which case your copy stays. Without the flag and without an existing copy, nothing is placed.

`--telemetry` writes the OpenTelemetry variables that point Claude Code at a collector on 127.0.0.1 into the `env` block of `~/.claude/settings.json`, so plain `claude`, the IDE and the desktop app feed `playbook usage` too. Init first reads whether telemetry is on, off or not set, from your environment and every Claude Code settings file (user, project, local, managed). When it is already on, set up by you, or switched off by something init will not change, init says so in one line and asks nothing. When it is not set, or switched off by a lone off switch in `~/.claude/settings.json`, init suggests turning it on as a yes or no question, and nothing is written without a yes. It writes nothing when your environment or any Claude Code settings file already names a telemetry variable, with one exception: a yes replaces an off switch that stands alone in `~/.claude/settings.json`. Any other setting you made, including values beside an off switch or in another file, is never overridden. Prompt text, tool content, emails and tokens are not sent, and the account id stays out of the metrics. `playbook uninstall` removes exactly the keys init wrote, unless you changed them, and puts back an off switch that a yes replaced. See [telemetry by default](../internals/04-usage-dashboard.md#telemetry-by-default).

### The `claude` function

`--claude-override` asks whether a plain `claude` should load the playbook system prompt every time. A yes sets `launcher.claudeOverride` to `true` in playbook's own config. It changes no Claude Code setting and no memory. With the key on, `playbook shell-init` also defines a `claude` function that starts the launcher for a plain interactive session. Calls that use their own flags (`-p`, subcommands, `--settings`, your own prompt flags, piped or scripted use) and `PLAYBOOK_BYPASS=1` go straight to the real `claude`, and a `claude` you already defined as an alias or function is left alone. The function needs the `eval` line from `--aliases` in your rc file. Undo it with `playbook config set launcher.claudeOverride false`. See [the launcher](../internals/01-launcher-and-hooks.md#the-claude-function).

### The `pb` short name

`--short-name` asks whether `pb` should be a short name for `playbook`. A yes sets `launcher.shortName` to `true` in playbook's own config, and nothing else changes. Init looks for a command named `pb` on your PATH first and writes nothing when it finds one. With the key on, `playbook shell-init` defines `pb` only when no command, alias, function or builtin of that name exists in your shell at that moment, so a `pb` you install or define later always wins and playbook never overwrites it. The function needs the `eval` line from `--aliases` in your rc file. Undo it with `playbook config set launcher.shortName false`. See [the launcher](../internals/01-launcher-and-hooks.md#the-pb-short-name).

### Memory migration

Early installs kept playbook's memory in `~/.claude/memory`. On the first run, init moves that tree to `~/.config/playbook/memory` and renames `graph.json` to `memory.graph.json`. The move runs once and a `.migration-complete` marker records it. This is playbook's own store. Claude Code's auto memory under `~/.claude/projects/` is never touched. The `session-init` and `memory-capture` hooks run the same check, so a store left behind by a partial move heals itself.

### Trust step

If `~/.claude.json` exists, init marks `~/.config/playbook` as trusted there, so a session started in that folder skips Claude Code's trust dialog. This is the one edit init makes to `~/.claude.json`. If the file is missing or cannot be written, init skips the step and reports it.

### Config-hash cleanup

Older installs placed `~/.config/playbook/hooks/lib/config-hash.sh`. The config hash now runs inside `playbook`, so init removes that script once. The hash itself is a fingerprint of your Claude Code config files that `session-init` stores in the session directory, to warn when a resumed session's config has changed. It is never sent anywhere.

## Undo

- `playbook uninstall` removes the hook entries and status line init wrote, the launcher block, and the files init placed under `~/.config/playbook`. Run it with `--dry-run` first to see the list. Add `--remove-binary` to remove the binary and its PATH block too.
- Memory, your own settings, sessions, history and earlier backups are never removed.
- Backups of `settings.json` are timestamped files next to it, named `settings.json.bak.<time>`.
- Any single part can be turned off by running `init` again with its `--no-` flag, then removing the entries by hand. Init never deletes a value you have.
