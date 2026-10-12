# What init installs

`playbook init` changes files on your machine. This page lists every change, what each part is for, how to say no, and how to undo it. Read it before you answer the questions `init` asks. The per-hook detail is in the [hook reference](../reference/hooks.md).

## See it before you say yes

- `playbook init` prints a short plan first: each part, what it does and the files it edits. Then it asks about each part, and each question names the files it will edit.
- `playbook init --dry-run` prints exactly what would change for each part and changes nothing.
- `playbook init --explain` describes every part. `playbook init --explain <part>` describes one, for example `--explain hooks`.
- `playbook hooks list` prints every hook with its event, matcher and one line of purpose.
- After a real run, `init` prints a summary of what changed and how to undo each part.

All of these read from the same tables in the code, and a test keeps the [hook reference](../reference/hooks.md) in line with them.

## Privacy

- Playbook stores everything on your machine, under `~/.config/playbook/` and the files named below.
- Playbook sends nothing anywhere. No hook, and no part of `init`, makes a network call. The only commands that reach the network are `playbook update` (it downloads a release from GitHub) and the status line, which calls the `gh` tool you already have to show PR and CI state.
- No hook reads a secret value. `precommit-check` looks at file names only, to flag names that look like credentials.
- `playbook usage` reads local Claude Code history. With `usage.telemetry` on `auto`, `ccc` points a session at a receiver on your own machine. See [usage telemetry privacy](../internals/04-usage-dashboard.md#telemetry-privacy).
- Claude Code's own settings are yours. Playbook edits `~/.claude/settings.json` only when you agree to it in `init`, and takes a timestamped backup first.

## The parts

Each part has a flag pair. With no flag, `init` asks in a terminal and uses the default otherwise. `--yes` accepts every default.

| Part | Default | Flags | Files it edits |
| :- | :- | :- | :- |
| Hooks | on | `--hooks`, `--no-hooks` | `~/.claude/settings.json`, `~/.claude/.settings.base.json` |
| Status line | on | `--statusline`, `--no-statusline` | `~/.claude/settings.json` |
| Security defaults | off | `--security`, `--no-security` | `~/.claude/settings.json` |
| PATH block | on | `--path`, `--no-path` | your shell startup files |
| Launcher shortcuts `ccc` and `ccd` | off | `--aliases`, `--no-aliases` | your shell rc file |
| System prompt | off | `--system-prompt`, `--no-system-prompt` | `~/.config/playbook/prompts/SYSTEM_PROMPT.md` |

`--settings` and `--no-settings` no longer exist. Each option that part used to carry now sits behind the hooks, status line or security part, or is not shipped.

Three steps run every time and have no flag: the memory migration, the trust step and the retired config-hash cleanup. They are described below.

### Hooks

Init adds 26 hook entries to `~/.claude/settings.json`. They run 20 different `playbook hook <name>` commands. The [hook reference](../reference/hooks.md) lists each one with its event, matcher, what it reads and writes, whether it can block a tool call, and how to turn it off.

The entries fall into five groups: safety guards, session lifecycle, usage tracking and auto mode, memory, and hygiene. Only a few hooks can stop a tool call: `rm-workspace-guard`, `no-slop-guard`, `policy-guard`, `preread-size-check`, and (in `auto` mode) `auto-guard` and `auto-cost`. `memory-capture` can pause a stop. The rest add context or warn.

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

`--aliases` adds one line to your `.zshrc` or `.bashrc`: `eval "$(playbook shell-init)"`. That command prints two small shell functions, `ccc` and `ccd`, and nothing else. Both call `playbook cc launch`, which starts Claude Code with the playbook system prompt, the model chain and your effort ceiling. `ccd` also skips permission prompts, so use it only where you would use `--dangerously-skip-permissions`. The functions use a temp file to move your shell into a worktree after a session. Older installs that sourced a script from the rc file get that line replaced with the `eval` line. See [the launcher](../internals/01-launcher-and-hooks.md).

### System prompt

`--system-prompt` copies `SYSTEM_PROMPT.md` to `~/.config/playbook/prompts/`. Claude Code uses it only when you start it through `ccc` or `ccd`. If you already have a copy, init refreshes it on every run, unless you edited it, in which case your copy stays. Without the flag and without an existing copy, nothing is placed.

### Memory migration

Early installs kept playbook's memory in `~/.claude/memory`. On the first run, init moves that tree to `~/.config/playbook/memory` and renames `graph.json` to `memory.graph.json`. The move runs once and a `.migration-complete` marker records it. This is playbook's own store. Claude Code's auto memory under `~/.claude/projects/` is never touched. The `session-init` and `memory-capture` hooks run the same check, so a store left behind by a partial move heals itself.

### Trust step

If `~/.claude.json` exists, init marks `~/.config/playbook` as trusted there, so a session started in that folder skips Claude Code's trust dialog. This is the one edit init makes to `~/.claude.json`. If the file is missing or cannot be written, init skips the step and reports it.

### Config-hash cleanup

Older installs placed `~/.config/playbook/hooks/lib/config-hash.sh`. The config hash now runs inside `playbook`, so init removes that script once. The hash itself is a fingerprint of your Claude Code config files that `session-init` stores in the session directory, to warn when a resumed session's config has changed. It is never sent anywhere.

## Undo

- `playbook uninstall` removes the hook entries and status line init wrote, the launcher block, and the files init placed under `~/.config/playbook`. Run it with `--dry-run` first to see the list. Add `--remove-binary` to remove the binary and its PATH block too.
- If you remove the plugin and leave the hooks behind, the status line and the session start add one hint line that tells you to run `playbook uninstall --yes`. Set `playbook config set --global hints.pluginRemoved false` to hide it. Nothing is removed for you.
- Memory, your own settings, sessions, history and earlier backups are never removed.
- Backups of `settings.json` are timestamped files next to it, named `settings.json.bak.<time>`.
- Any single part can be turned off by running `init` again with its `--no-` flag, then removing the entries by hand. Init never deletes a value you have.
