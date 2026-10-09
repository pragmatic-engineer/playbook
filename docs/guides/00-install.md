# Install

The quickest route is one command. This page covers every route, requirements, how the binary reaches `PATH`, upgrades, the settings merge and uninstall.

## Requirements

| Tool | Status | Why |
|---|---|---|
| `claude` v2.1.224 or later | recommended | The marketplace serves a trimmed plugin archive that older versions cannot install. Below that, or missing, `install.sh` skips the plugin step with a hint. The binary, hooks, guards and settings still install. |
| `bash` or `zsh` | required | The `ccc` and `ccd` shell functions work in both. |
| `git`, `shasum` | required | Used by hooks and the installer. |
| `gh` | optional | Status line PR and CI info, and the build attestation check. |
| `agent-browser` | optional | Browser automation `/playbook:plan` uses for web-only tickets. |

## Install

### One command

```bash
curl -fsSL https://raw.githubusercontent.com/pragmatic-engineer/playbook/main/install.sh | bash
```

It installs the `playbook` binary into `PLAYBOOK_BIN_DIR` (default `~/.local/bin`) after checking its SHA256 and a `--version` smoke test, and puts that directory on `PATH`. Then it runs `playbook init`, which wires the guards and hooks into `settings.json`, merges the shared settings and installs the status line. It also installs the plugin, and asks before adding the launcher and system prompt. Open a new terminal afterward.

Pass `--yes` to accept every default. Flags go after `-s --` when piping:

| Flag | Effect |
|---|---|
| `--yes`, `-y` | Accept every default. |
| `--skip-plugin`, `--no-setup` | Skip the plugin. Guards, settings and shell wiring still run. |
| `--aliases` | Install `ccc` and `ccd` without asking. |
| `--system-prompt` | Install the system prompt without asking (implies `--aliases`). |
| `--binary-only` | Binary, `PATH`, guards and settings only. No plugin, launcher, system prompt or prompts. |
| `--ref <ref>` | Source ref, same as `PLAYBOOK_REF`. |

Pin a version with `PLAYBOOK_REF`:

```bash
curl -fsSL https://raw.githubusercontent.com/pragmatic-engineer/playbook/main/install.sh | PLAYBOOK_REF=v0.20.1 bash
```

A release tag installs that release and its binary. A branch or commit pins only the source and installs no binary.

### Homebrew

```bash
brew install pragmatic-engineer/tap/playbook
playbook init
```

`playbook init` wires the hooks and the `PATH` block, so hook shells find the binary even when Homebrew's directory is not on their `PATH`. Upgrade with `brew upgrade playbook`.

### Without `curl | bash`

**Plugin, then `/playbook:setup`.**

```bash
claude plugin marketplace add pragmatic-engineer/marketplace
claude plugin install playbook@pragmatic-engineer
```

Then run `/playbook:setup` in a Claude Code session. It installs the release binary (checksum verified) when none is on `PATH`, wires the guards and settings, and offers the launchers and system prompt.

The plugin alone gives you skills, commands and subagents, and nothing else. Without the binary every hook is dead, so `/playbook:doctor` reports the binary, guards and status line as missing. The plugin ships a `bin/playbook` shim, and the first `playbook` call installs the binary once (binary and `PATH` only).

**Fully manual.**

1. Install the plugin as above.
2. From the [latest release](https://github.com/pragmatic-engineer/playbook/releases/latest), download the asset for your platform and `SHA256SUMS`.

   | Platform | Asset |
   |---|---|
   | macOS, Apple silicon | `playbook-<version>-aarch64-apple-darwin` |
   | macOS, Intel | `playbook-<version>-x86_64-apple-darwin` |
   | Linux, x86_64 | `playbook-<version>-x86_64-unknown-linux-musl` |
   | Linux, arm64 | `playbook-<version>-aarch64-unknown-linux-musl` |
   | Windows | `playbook-<version>-x86_64-pc-windows-msvc.exe` |

3. Verify it. `SHA256SUMS` is not signed, so it is only a corruption check. The attestation shows where the binary was built.

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

   In a terminal, `init` asks about hooks, settings, `PATH`, the launcher and the system prompt. Each has `--x` and `--no-x` flags. With `--yes`, or without a terminal, hooks, settings and `PATH` are on and the launcher and system prompt are off. Every step is safe to repeat.

6. Run `/playbook:doctor`.

## Upgrade

| Install | Command |
|---|---|
| Any | `playbook update` (`--check` to look, `--list` for releases, `--pre` for pre-releases, `--yes` in auto mode) |
| Homebrew | `brew upgrade playbook` |
| Installer | Run it again |

`playbook update` verifies SHA256SUMS and the attestation, keeps the old binary as `playbook.<version>.bak` (the newest three are kept), refuses a Homebrew install, and warns when another `playbook` earlier on `PATH` hides the new one. To roll back, move a backup over the binary:

```bash
mv -f ~/.local/bin/playbook.0.19.0.bak ~/.local/bin/playbook
```

See [release channels](../internals/06-release-channels.md) for how a release reaches each channel.

## How the binary gets on PATH

Every route ends with `playbook` on `PATH` for each shell start.

| Shell | File that gets the `# playbook binary` block |
|---|---|
| zsh | `~/.zshenv` (read by every zsh, including non-interactive ones) |
| bash | `~/.bashrc` and the login file (`~/.bash_profile`, `~/.bash_login` or `~/.profile`, whichever exists first) |
| fish | `~/.config/fish/conf.d/playbook.fish` |
| other | `~/.profile` |

`install.sh` and `playbook init` write the same block and skip a file that already names the directory.

Hooks do not read these files. Measured on Claude Code 2.1.293 (2026-10-09), a hook runs in a non-interactive `bash -c` with exactly the `PATH` of the process that started Claude Code. So:

- **Started from a terminal:** the inherited `PATH` has the block's directory. Hooks find `playbook`.
- **Started with `ccc` or `ccd`:** the launcher adds the binary's directory when it is missing.
- **Started from a desktop app or IDE:** hooks find `playbook` only if its directory is already on that `PATH`. Use `ccc`, or install with `PLAYBOOK_BIN_DIR` set to a directory that environment has.

## Settings merge

`playbook init` merges the shipped template into your `settings.json` and never overwrites it. New product config lands automatically. Keys you customised stay as you set them.

It tracks a baseline in `~/.claude/.settings.base.json` and compares it with the new template and your live file. After each run, `$CLAUDE_HOME/settings-merge-skipped.<epoch>.json` lists every key where your value won, as `{"key":"...", "template_had":..., "yours":...}`. Review it and adopt template values by hand if you want them.

The security defaults, the `permissions` block and `DISABLE_AUTOUPDATER`, are opt-in. A plain `playbook init` does not add them and never removes ones you have. Pass `--security` or set `security.defaults` to true to merge them. `permissions` is one top-level key. If you customised it, the whole block counts as contested and the template's version is withheld, so your rules win. See [Security](../../README.md#security).

If a run is interrupted between writing `settings.json` and the baseline, delete `.settings.base.json`. The next run treats the missing baseline as empty and only adds keys.

## Uninstall

```bash
playbook uninstall --dry-run   # list what would go, change nothing
playbook uninstall --yes       # remove it
```

It removes only what playbook placed: the hook entries and status line in `settings.json`, the launcher block in your rc files, and the files under `~/.config/playbook` that playbook copied (such as the system prompt). Without `--yes` it lists the plan and exits with status 1.

Everything else stays: the rest of `settings.json` and your rc files, `.settings.base.json`, `backups/`, memory and all runtime state. Each changed file is backed up first (`settings.json.bak.<epoch>`). A status line script or system prompt you edited by hand is kept. If `settings.json` cannot be edited, such as invalid JSON, the placed files and the binary stay too.

`--remove-binary` also removes the binary, its `.bak` backups and the `PATH` block the installer added.

## Notes

Edits to `settings.json` or hooks take effect in a fresh session only. Run `ccc fresh` or plain `claude` after changing them. `ccc` warns when a resumed session runs on stale config.

## See also

- [Launcher and hooks](../internals/01-launcher-and-hooks.md)
- [The system prompt](../concepts/01-system-prompt.md)
- [Docs index](../index.md)
