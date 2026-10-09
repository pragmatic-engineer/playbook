# Install

This page covers requirements, the full local install path with the curl one-liner, the settings merge behaviour, and uninstall. For the primary path (the Claude Code plugin plus `/playbook:setup`), see the [README](../../README.md).

## Requirements

| Tool | Status | Why |
|---|---|---|
| `claude` on PATH, v2.1.224+ | recommended | Claude Code itself. The marketplace serves a trimmed plugin archive that needs 2.1.224 or later. Below that, or missing entirely, `install.sh` skips the plugin step with an upgrade hint rather than failing; the rest of the install (binary, hooks, guards, settings) still completes |
| `bash` | required | hooks and the setup script run in bash |
| zsh | required for `ccc worktree` | the worktree subcommand is zsh-only; all other `ccc` subcommands and `ccd` work in bash |
| `git`, `shasum` | required | used by hooks and the install script |
| `gh` | optional | statusline PR and CI status |
| `agent-browser` | optional | browser automation MCP used by `/playbook:plan` for web-only tickets and attachments |

## Install without `curl | bash`

Two routes avoid piping a script into a shell.

**Route 1: plugin, then `/playbook:setup`.**

```bash
claude plugin marketplace add pragmatic-engineer/marketplace
claude plugin install playbook@pragmatic-engineer
```

Then run `/playbook:setup` in a Claude Code session. It installs the release binary (checksum verified) when none is on `PATH`, wires the guards and settings, and offers the launchers and system prompt. Check with `/playbook:doctor`.

`claude plugin install` alone gives you the skills, commands and subagents, and nothing else. Without the binary every hook is dead and the guards stay unwired, so `/playbook:doctor` reports the binary, guards and status line as missing. The plugin also ships a `bin/playbook` shim: the first `playbook` call installs the binary once (binary and `PATH` only).

**Route 2: fully manual.**

1. Install the plugin as above.
2. From the [latest release](https://github.com/pragmatic-engineer/playbook/releases/latest), download the asset for your platform and `SHA256SUMS`.

   | Platform | Asset |
   |---|---|
   | macOS, Apple silicon | `playbook-<version>-aarch64-apple-darwin` |
   | macOS, Intel | `playbook-<version>-x86_64-apple-darwin` |
   | Linux, x86_64 | `playbook-<version>-x86_64-unknown-linux-musl` |
   | Linux, arm64 | `playbook-<version>-aarch64-unknown-linux-musl` |
   | Windows | `playbook-<version>-x86_64-pc-windows-msvc.exe` |

3. Verify it. `SHA256SUMS` is not signed, so treat it as a corruption check. To verify where the binary was built, use the GitHub CLI:

   ```bash
   grep "  <asset>$" SHA256SUMS | shasum -a 256 -c -
   gh attestation verify <asset> --repo pragmatic-engineer/playbook
   ```

   `install.sh` and `playbook update` run the attestation check when `gh` is available. They warn when `gh` is missing or the release predates attestations, and abort on a real mismatch. Set `PLAYBOOK_REQUIRE_ATTESTATION=1` to abort on a missing check too.

4. Put it on `PATH`:

   ```bash
   mkdir -p ~/.local/bin
   mv <asset> ~/.local/bin/playbook
   chmod 0755 ~/.local/bin/playbook
   ```

5. Wire the local config. `CLAUDE_PLUGIN_ROOT` is required, or `init` skips almost every step:

   ```bash
   CLAUDE_PLUGIN_ROOT=~/.claude/plugins/cache/pragmatic-engineer/playbook/<version> \
     playbook init
   ```

   In a terminal, `playbook init` asks about hooks, shared settings, `PATH`, the launcher and the system prompt. Each has a `--x` and `--no-x` flag to skip the question. With `--yes`, or without a terminal, hooks, settings and `PATH` are on and the launcher and system prompt are off. Every step is idempotent.

6. Run `/playbook:doctor`.

## Full local install with curl

The `curl | bash` one-liner is a two-step install: it fetches, verifies (SHA256, then a `--version` smoke test), and installs the `playbook` binary into `PLAYBOOK_BIN_DIR` (default `~/.local/bin`), puts that directory on `PATH` (see below), then hands off to `playbook init`, which wires the safety guards and functional hooks into `settings.json`, seeds or merges the rest of the local config, and installs the shell launcher and statusline. It also runs the plugin install and prompts for the opt-in layers. Use this if you want the full `~/.claude` file set locally (for example, to clone and edit the config).

```bash
curl -fsSL https://raw.githubusercontent.com/pragmatic-engineer/playbook/main/install.sh | bash
```

Open a new terminal (or source your rc file) afterwards so the binary resolves on `PATH`.

### How the binary gets on PATH

Every install route ends with `playbook` on `PATH` for every shell start:

| Shell | File that gets the `# playbook binary` block |
|---|---|
| zsh | `~/.zshenv` (every zsh reads it, including non-interactive ones; `~/.zshrc` is skipped by those) |
| bash | `~/.bashrc` and the login file (`~/.bash_profile`, `~/.bash_login` or `~/.profile`, whichever exists first, else `~/.bash_profile`) |
| fish | `~/.config/fish/conf.d/playbook.fish` |
| other | `~/.profile` |

`install.sh` and `playbook init` write the same block and skip a file that already names the directory, so running both never doubles it. `playbook uninstall --remove-binary` removes it.

### How hooks find the binary

Hooks do not read these files. Measured on Claude Code 2.1.293 (2026-10-09): a hook command runs in a non-interactive `bash -c`, reads no shell startup file, and gets exactly the `PATH` of the process that started Claude Code. So:

- **Claude Code started from a terminal:** the terminal's `PATH` is inherited, and the block above put the binary on it. Hooks find `playbook`.
- **Claude Code started with `ccc` or `ccd`:** the launcher adds the binary's directory to the session's `PATH` when it is missing. Hooks find `playbook` whatever your startup files say.
- **Claude Code started from a desktop app, an IDE or any minimal environment:** no startup file is read, so hooks find `playbook` only if its directory is already on that environment's `PATH`. Start the session with `ccc`, or install with `PLAYBOOK_BIN_DIR` set to a directory that environment already has.

- **Installer:** puts the binary in `PLAYBOOK_BIN_DIR`, adds the block, then runs `playbook init`.
- **Claude Code marketplace:** the plugin ships a `bin/playbook` shim, and Claude Code puts a plugin's `bin/` on the Bash tool's `PATH`. The shim runs the real binary when it finds one. When it finds none, it runs the shipped `install.sh --yes --binary-only`, pinned to the plugin version, which installs the binary, the `PATH` block, the guards and the settings, but not the launcher or the system prompt. It tries once and never loops.
- **Homebrew:** `brew install pragmatic-engineer/tap/playbook` puts the binary in the Homebrew `bin`. Run `playbook init` once afterwards. It wires the hooks and adds the `PATH` block, so hook shells find the binary even when Homebrew's directory is not on their `PATH`. The formula prints this as a caveat.

Re-running the installer upgrades in place. It keeps the binary it replaces as `playbook.<version>.bak` next to it (for example `~/.local/bin/playbook.0.17.0.bak`), and the newest three are kept. To roll back, move the backup over the binary:

```bash
mv -f ~/.local/bin/playbook.0.17.0.bak ~/.local/bin/playbook
```

If the old binary could not report its version, the backup is named `playbook.unknown-<UTC timestamp>.bak`. A failed backup only prints a warning and the install goes on. Uninstall removes the backups too.

Pass `--yes` to accept every default without prompting. Pin a version:

```bash
curl -fsSL https://raw.githubusercontent.com/pragmatic-engineer/playbook/main/install.sh | PLAYBOOK_REF=v0.20.0 bash
```

A version tag installs that release and its binary. A branch or commit pins only the source and installs no binary.

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
| `--binary-only` | binary, `PATH`, guards and settings only: no plugin, no launcher, no system prompt, no prompts |
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

It removes only what playbook placed: the hook entries and status line `playbook init` wrote into `~/.claude/settings.json`, the launcher block in `~/.zshrc` and `~/.bashrc`, the files playbook copied under `~/.config/playbook` (`prompts/SYSTEM_PROMPT.md`), and `install.sh` and `uninstall.sh` in `~/.claude`. Without `--yes` it only lists the plan and exits with status 1.

Everything else stays: the rest of `settings.json` (including any hook or status line you added yourself), the rest of your rc files, `.settings.base.json`, `backups/`, memory, and all runtime state. Each changed file is backed up first, as `settings.json.bak.<epoch>` and `<rc file>.bak-<epoch>`. A `statusline.sh` or system prompt you edited by hand, or that playbook has no record of placing, is kept, and so is `statusline.sh` while your own status line still runs it. If `settings.json` cannot be edited (invalid JSON, for example), the placed files and the binary stay too, because the hooks in it still need them.

**Flags:**

- `--yes`: do the removal (nothing changes without it).
- `--dry-run`: list what would be removed and change nothing.
- `--remove-binary`: also remove the `playbook` binary, its `playbook.<version>.bak` backups, and the `# playbook binary` PATH line the installer added.

**Older installs:** a machine installed before the installer shrank to a few files may still hold a full copy of the source tree in `~/.claude` (`docs`, `src`, `tests`, and so on). `playbook uninstall` does not guess at those; remove them by hand if you want them gone.

## Notes

Config edits (`settings.json` or hooks) take effect on a fresh session only. After changing them, run `ccc fresh` or plain `claude`. `ccc` warns you when a resumed session runs on stale config. The repo tracks config files, not runtime state. The allowlist `.gitignore` keeps sessions, caches, plugin manifests, and credentials out of git.

## See also

- [Launcher and hooks](../internals/01-launcher-and-hooks.md): what `/playbook:setup` wires and how the `ccc` launcher runs.
- [The system prompt](../concepts/01-system-prompt.md): the optional persona `/playbook:setup` can install.
- [Docs index](../index.md)
