# Install

This page covers requirements, the full local install path with the curl one-liner, the settings merge behaviour, and uninstall. For the primary path (the Claude Code plugin plus `/setup`), see the [README](../../README.md).

## Requirements

| Tool | Status | Why |
|---|---|---|
| `claude` on PATH | required | Claude Code itself |
| `bash` | required | hooks and the setup script run in bash |
| zsh | required for `cc worktree` | the worktree subcommand is zsh-only; all other `cc` subcommands and `ccd` work in bash |
| `git`, `jq`, `shasum` | required | used by hooks and the install script |
| `python3` 3.9+ | required | used by two bash hooks (path resolution and the memory-graph rebuild); the hooks themselves are bash |
| `rtk` (Rust Token Killer) | required | a PreToolUse hook routes every Bash command through it to cut token use |
| `gh` | optional | statusline PR and CI status |
| `agent-browser` | optional | browser automation MCP used by `/brainstorm` for web-only tickets and attachments |

## Full local install with curl

The `curl | bash` one-liner downloads the files into `~/.claude`, runs the plugin install, and prompts for the opt-in layers. Use this if you want the full `~/.claude` file set locally (for example, to clone and edit the config).

```bash
curl -fsSL https://raw.githubusercontent.com/pragmatic-engineer/playbook/main/install.sh | bash
```

Pass `--yes` to accept every default without prompting. Pin a version:

```bash
PLAYBOOK_REF=v0.2.1 curl -fsSL https://raw.githubusercontent.com/pragmatic-engineer/playbook/main/install.sh | bash
```

Install files only (no plugin, no local wiring):

```bash
curl -fsSL https://raw.githubusercontent.com/pragmatic-engineer/playbook/main/install.sh | bash -s -- --no-setup
```

Flags (pass after `-s --` when piping):

| Flag | Effect |
|---|---|
| `--yes`, `-y` | non-interactive: accept every step's default |
| `--skip-plugin` | don't add the marketplace or install the plugin |
| `--skip-deps` | skip `brew bundle` |
| `--aliases` | install the shell launchers without prompting |
| `--system-prompt` | install the custom system prompt without prompting (implies `--aliases`) |
| `--no-setup` | install files only: no plugin, deps, or shell edits |
| `--ref <ref>` | source ref (same as `PLAYBOOK_REF`) |

Prefer git? Clone fresh:

```bash
git clone https://github.com/pragmatic-engineer/playbook.git ~/.claude
```

Already have a `~/.claude` from Claude Code? Adopt it in place. The `.gitignore` is an allowlist so sessions, caches, and runtime files stay ignored:

```bash
cd ~/.claude
git init
git remote add origin https://github.com/pragmatic-engineer/playbook.git
git fetch origin
git checkout -f main
```

After cloning or adopting, run `/setup` inside a Claude Code session to wire the local layers.

## Settings merge

Each `install.sh` run merges the shipped template into your `settings.json` rather than overwriting it. New product config lands automatically; keys you have customised stay as you set them.

The merge tracks a baseline in `~/.claude/.settings.base.json`. On each install it compares that baseline against the new template and your live file to decide which keys to update and which to leave alone.

After each install, check `backups/install-<stamp>/settings-merge-skipped.json`. It lists every key the new template tried to change but your customisation took precedence. Entries look like `{"key":"...", "template_had":..., "yours":...}`. Review them and decide whether to adopt the template value manually.

`permissions` is a single top-level key. If you have customised it (for example, added rules to `permissions.deny`), the whole `permissions` block is treated as contested and the template's version is withheld. Your custom rules take precedence. The skip file will show the entry so you can compare and merge manually if the template shipped new deny rules you want.

If an install is interrupted after writing `settings.json` but before writing the baseline, the files are out of sync. Delete `~/.claude/.settings.base.json` to reset. The next install treats the missing baseline as an empty object and falls back to additive mode: all your keys are kept and new template keys are added.

## Uninstall

```bash
bash ~/.claude/uninstall.sh
```

This removes every shipped file from `~/.claude` and strips the launcher source lines from `~/.zshrc` and `~/.bashrc`. It backs up the rc files before editing.

**Preserved by default:** `settings.json`, `.settings.base.json`, `backups/`, and all runtime state (`sessions/`, `projects/`, `history*`, `plugins/`, `memory/`, `plans/`, `runtime/`, `cache/`, `logs/`, `todos/`, `shell-snapshots/`, `.credentials*`, `cc-state/`, `ccd-state/`).

Pass `--purge` to also remove `settings.json`, `.settings.base.json`, and `backups/`.

**Flags:**

- `--yes`: skip the confirmation prompt.
- `--force`: bypass the git-repo guard (see below).
- `--purge`: remove user config in addition to shipped files.

**Git-repo guard:** if `~/.claude` is a git working tree, the script refuses to run. Raw `rm` leaves index entries dangling; the correct path for decommissioning is `git rm -r <entries>`. Pass `--force` to bypass this guard if you know what you are doing. `--force` bypasses only the git guard; it does not skip the confirmation prompt.

## Notes

Config edits (`settings.json` or hooks) take effect on a fresh session only. After changing them, run `cc fresh` or plain `claude`. `cc` warns you when a resumed session runs on stale config. The repo tracks config files, not runtime state. The allowlist `.gitignore` keeps sessions, caches, plugin manifests, and credentials out of git.

## See also

- [Launcher and hooks](../internals/01-launcher-and-hooks.md): what `/setup` wires and how the `cc` launcher runs.
- [The system prompt](../concepts/01-system-prompt.md): the optional persona `/setup` can install.
- [Docs index](../index.md)
