# Install

This page covers requirements, the full local install path with the curl one-liner, the settings merge behaviour, and uninstall. For the primary path (the Claude Code plugin plus `/playbook:setup`), see the [README](../../README.md).

## Requirements

| Tool | Status | Why |
|---|---|---|
| `claude` on PATH, v2.1.224+ | recommended | Claude Code itself. The marketplace serves a trimmed plugin archive that needs 2.1.224 or later. Below that, or missing entirely, `install.sh` skips the plugin step with an upgrade hint rather than failing; the rest of the install (binary, hooks, guards, settings) still completes |
| `bash` | required | hooks and the setup script run in bash |
| zsh | required for `cc worktree` | the worktree subcommand is zsh-only; all other `cc` subcommands and `ccd` work in bash |
| `git`, `shasum` | required | used by hooks and the install script |
| `gh` | optional | statusline PR and CI status |
| `agent-browser` | optional | browser automation MCP used by `/playbook:plan` for web-only tickets and attachments |

## Full local install with curl

The `curl | bash` one-liner is a two-step install: it fetches, verifies (SHA256, then a `--version` smoke test), and installs the `playbook` binary into `PLAYBOOK_BIN_DIR` (default `~/.local/bin`), puts that directory on `PATH`, then hands off to `playbook init`, which wires the safety guards and functional hooks into `settings.json`, seeds or merges the rest of the local config, and installs the shell launcher and statusline. It also runs the plugin install and prompts for the opt-in layers. Use this if you want the full `~/.claude` file set locally (for example, to clone and edit the config).

```bash
curl -fsSL https://raw.githubusercontent.com/pragmatic-engineer/playbook/main/install.sh | bash
```

Open a new terminal (or source your rc file) afterwards so the binary resolves on `PATH`.

Re-running the installer upgrades in place. It keeps the binary it replaces as `playbook.<version>.bak` next to it (for example `~/.local/bin/playbook.0.17.0.bak`), and the newest three are kept. To roll back, move the backup over the binary:

```bash
mv -f ~/.local/bin/playbook.0.17.0.bak ~/.local/bin/playbook
```

If the old binary could not report its version, the backup is named `playbook.unknown-<UTC timestamp>.bak`. A failed backup only prints a warning and the install goes on. Uninstall removes the backups too.

Pass `--yes` to accept every default without prompting. Pin a version:

```bash
PLAYBOOK_REF=v0.17.0 curl -fsSL https://raw.githubusercontent.com/pragmatic-engineer/playbook/main/install.sh | bash
```

Skip the plugin (the binary, guards, settings and shell wiring still run). Same as `--skip-plugin`:

```bash
curl -fsSL https://raw.githubusercontent.com/pragmatic-engineer/playbook/main/install.sh | bash -s -- --no-setup
```

Flags (pass after `-s --` when piping):

| Flag | Effect |
|---|---|
| `--yes`, `-y` | non-interactive: accept every step's default |
| `--skip-plugin` | don't add the marketplace or install the plugin |
| `--skip-deps` | accepted, ignored: `playbook init` installs no deps itself |
| `--aliases` | install the shell launchers without prompting |
| `--system-prompt` | install the custom system prompt without prompting (implies `--aliases`) |
| `--no-setup` | skip the plugin only; guards, settings, and shell wiring still run |
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

After cloning or adopting, run `/playbook:setup` inside a Claude Code session to wire the local layers.

## Other ways to install and upgrade

- macOS or Linux with Homebrew: `brew install pragmatic-engineer/tap/playbook`. Upgrade with `brew upgrade playbook`.
- Any other install: `playbook update`. Use `--check` to look without installing, `--list` for releases, `--pre` for pre-releases and `--yes` in auto mode. It keeps the three newest backups and refuses a Homebrew install.
- The plugin from the marketplace is a trimmed archive (plugin files only) and needs Claude Code 2.1.224 or later. See [release channels](../internals/06-release-channels.md).

## Settings merge

Each `install.sh` run merges the shipped template into your `settings.json` rather than overwriting it. New product config lands automatically; keys you have customised stay as you set them.

The merge tracks a baseline in `~/.claude/.settings.base.json`. On each install it compares that baseline against the new template and your live file to decide which keys to update and which to leave alone.

After each install, check `$CLAUDE_HOME/settings-merge-skipped.<epoch>.json`. It lists every key the new template tried to change but your customisation took precedence. Entries look like `{"key":"...", "template_had":..., "yours":...}`. Review them and decide whether to adopt the template value manually.

`permissions` is a single top-level key. If you have customised it (for example, added rules to `permissions.deny`), the whole `permissions` block is treated as contested and the template's version is withheld. Your custom rules take precedence. The skip file will show the entry so you can compare and merge manually if the template shipped new deny rules you want.

If an install is interrupted after writing `settings.json` but before writing the baseline, the files are out of sync. Delete `~/.claude/.settings.base.json` to reset. The next install treats the missing baseline as an empty object and falls back to additive mode: all your keys are kept and new template keys are added.

## Uninstall

```bash
playbook uninstall --dry-run   # list what would be removed, change nothing
playbook uninstall --yes       # remove it
```

It removes only what playbook placed: the hook entries and status line `playbook init` wrote into `~/.claude/settings.json`, the launcher block in `~/.zshrc` and `~/.bashrc`, the files playbook copied under `~/.config/playbook` (`statusline.sh`, `prompts/SYSTEM_PROMPT.md`, `hooks/lib/config-hash.sh`), and `install.sh` and `uninstall.sh` in `~/.claude`. Without `--yes` it only lists the plan and exits with status 1.

Everything else stays: the rest of `settings.json` (including any hook or status line you added yourself), the rest of your rc files, `.settings.base.json`, `backups/`, memory, and all runtime state. Each changed file is backed up first, as `settings.json.bak.<epoch>` and `<rc file>.bak-<epoch>`. A `statusline.sh` or system prompt you edited by hand is kept.

**Flags:**

- `--yes`: do the removal (nothing changes without it).
- `--dry-run`: list what would be removed and change nothing.
- `--remove-binary`: also remove the `playbook` binary, its `playbook.<version>.bak` backups, and the `# playbook binary` PATH line the installer added.

**Older installs:** a machine installed before the installer shrank to a few files may still hold a full copy of the source tree in `~/.claude` (`docs`, `src`, `tests`, and so on). `playbook uninstall` does not guess at those; remove them by hand if you want them gone.

## Notes

Config edits (`settings.json` or hooks) take effect on a fresh session only. After changing them, run `cc fresh` or plain `claude`. `cc` warns you when a resumed session runs on stale config. The repo tracks config files, not runtime state. The allowlist `.gitignore` keeps sessions, caches, plugin manifests, and credentials out of git.

## See also

- [Launcher and hooks](../internals/01-launcher-and-hooks.md): what `/playbook:setup` wires and how the `cc` launcher runs.
- [The system prompt](../concepts/01-system-prompt.md): the optional persona `/playbook:setup` can install.
- [Docs index](../index.md)
