# Install

To see every change `playbook init` makes before you agree to it, read [What init installs](06-what-init-installs.md) and the [hook reference](../reference/hooks.md).

The quickest route is one command. This page covers every route, requirements, how the binary reaches `PATH`, upgrades, the settings merge and uninstall.

## Pick an install mode

| Mode | Commands, in order | You get | You lack |
|---|---|---|---|
| Skills only (plugin) | `claude plugin marketplace add pragmatic-engineer/marketplace`, then `claude plugin install playbook@pragmatic-engineer`. Or the install script with `--skills-only`, which does the same and nothing else | Skills, commands and subagents. No hooks, no binary, no change to `settings.json` | Safety guards, status line, hooks, usage, and every step that needs the `playbook` binary |
| Skills plus binary (recommended) | Pick one. Install script: `curl -fsSL https://raw.githubusercontent.com/pragmatic-engineer/playbook/main/install.sh \| bash` (installs both plugins). Homebrew: `brew install pragmatic-engineer/tap/playbook`, then `playbook init`. Plugins: `claude plugin install playbook@pragmatic-engineer` and `claude plugin install playbook-hooks@pragmatic-engineer`, then `/playbook:setup` | Everything: slash commands, guards, hooks, status line, memory, usage | Nothing |

There are two plugins. `playbook` holds the skills, commands and agents and has no hooks. `playbook-hooks` holds the worktree hooks and the installer shim, and needs the binary. If you installed `playbook` before the split, add the second one with `claude plugin install playbook-hooks@pragmatic-engineer`. `playbook init` and `/playbook:doctor` tell you when it is missing, and playbook never installs a plugin for you.

Which one? Choose skills only if you want to read the skills and nothing else runs on your machine. Choose skills plus binary for everything else. The binary is a hard dependency of the commands, hooks, guards, status line and usage. Only the skill text works without it.

Some commands call the `playbook` binary. With only the `playbook` plugin they fall back to plain git, gh and file steps where they can. With `playbook-hooks` installed, the first `playbook` call installs the binary once and prints a notice first. See [Plugin only](#plugin-only-and-the-binary).

## Requirements

| Tool | Status | Why |
|---|---|---|
| `claude` v2.1.224 or later | recommended | The marketplace serves a trimmed plugin archive that older versions cannot install. Below that, or missing, `install.sh` skips the plugin step with a hint. The binary, hooks, guards and settings still install. |
| `bash` or `zsh` | required | The `playbook` shell wrapper works in both. |
| `git`, `shasum` | required | Used by hooks and the installer. |
| `gh` | optional | Status line PR and CI info, and the build attestation check. |
| `agent-browser` | optional | Browser automation `/playbook:plan` uses for web-only tickets. |

## Install

### One command

```bash
curl -fsSL https://raw.githubusercontent.com/pragmatic-engineer/playbook/main/install.sh | bash
```

It installs the `playbook` binary into `PLAYBOOK_BIN_DIR` (default `~/.local/bin`) after checking its SHA256 and a `--version` smoke test, and puts that directory on `PATH`. Then it runs `playbook init`, which wires the guards and hooks into `settings.json`, and sets the status line. It also installs the plugin, and asks before adding the launcher and system prompt. Open a new terminal afterward.

Pass `--yes` to accept every default. Flags go after `-s --` when piping:

| Flag | Effect |
|---|---|
| `--yes`, `-y` | Accept every default. |
| `--skip-plugin`, `--no-setup` | Skip the plugin. Guards, settings and shell wiring still run. |
| `--aliases` | Add the `playbook` cd wrapper to your rc file without asking. |
| `--claude-override` | Make a plain `claude` load the playbook system prompt (needs the launcher line from `--aliases`). |
| `--short-name` (`playbook init`) | Define a `pb` shell function for `playbook`, only when `pb` is free. install.sh also links `pb` to `playbook` when the name is free. |
| `--system-prompt` | Install the system prompt without asking (implies `--aliases`). |
| `--binary-only` | Binary, `PATH`, guards and settings only. No plugin, launcher, system prompt or prompts. |
| `--skills-only` | Install only the `playbook` plugin. No binary, no hooks plugin, no `settings.json` change. Cannot be combined with `--binary-only`. |
| `--ref <ref>` | Source ref, same as `PLAYBOOK_REF`. |

Pin a version with `PLAYBOOK_REF`:

```bash
curl -fsSL https://raw.githubusercontent.com/pragmatic-engineer/playbook/main/install.sh | PLAYBOOK_REF=v0.25.0 bash
```

A release tag installs that release and its binary. A branch or commit pins only the source and installs no binary.

### Homebrew

```bash
brew install pragmatic-engineer/tap/playbook
playbook init
```

`playbook init` wires the hooks and the `PATH` block, so hook shells find the binary even when Homebrew's directory is not on their `PATH`. Homebrew ships only the binary, so `init` seeds the settings template and the system prompt from copies compiled into it (written to `~/.config/playbook/embedded/`) when no Claude Code plugin is installed. An installed plugin, even an older version, takes precedence. Upgrade with `brew upgrade playbook`.

### Optional: the standalone telemetry tool

You do not need this if you have `playbook`. `playbook usage` runs the same code, because playbook embeds the `playbook-telemetry` crate. Install the standalone tool only on a machine that has no `playbook`, or when you want the short `pbtop` command.

```bash
brew install pragmatic-engineer/tap/playbook-telemetry
pbtop
```

The formula installs two commands that are the same program: the short `pbtop`, which is the one to type (`pbtop`, `pbtop --summary`, `pbtop --json`, `pbtop --range 30d`), and the long `playbook-telemetry`, which suits scripts. Release archives for macOS, Linux and Windows are on the [playbook releases page](https://github.com/pragmatic-engineer/playbook/releases) (the telemetry ones are tagged `telemetry-vX.Y.Z`, starting at 0.2.0), and the [crate README](../../crates/telemetry/README.md) covers `cargo` builds. Everything stays on your machine.

The tool shares its data with playbook: the database is `~/.config/playbook/data/usage.db` and the settings are the `usage.*` keys in `~/.config/playbook/data/config.db`. See [Config keys](04-config-keys.md).

### Coming: the standalone code index

`playbook code` will also get a standalone tool, [playbook-code](../../crates/code), with the short alias `pbcode`. It has no release yet, so there is nothing to install. Use `playbook code` today.

### Without `curl | bash`

**Plugin, then `/playbook:setup`.**

```bash
claude plugin marketplace add pragmatic-engineer/marketplace
claude plugin install playbook@pragmatic-engineer
claude plugin install playbook-hooks@pragmatic-engineer
```

Then run `/playbook:setup` in a Claude Code session. It installs the release binary (checksum verified) when none is on `PATH`, wires the guards and settings, and offers the launchers and system prompt.

The `playbook` plugin alone gives you skills, commands and subagents, and nothing else. Without the binary every hook is dead, so `/playbook:doctor` reports the binary, guards and status line as missing. The `playbook-hooks` plugin ships a `bin/playbook` shim, and the first `playbook` call installs the binary once (binary and `PATH` only).

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

   In a terminal, `init` asks about hooks, settings, the status line, `PATH`, the launcher and the system prompt. The status line question reads "Use the playbook status line?" and has `--statusline` and `--no-statusline`. Answering no leaves `statusLine` in `settings.json` as it is, and an existing playbook status line stays until `playbook uninstall`. Each part has `--x` and `--no-x` flags. With `--yes`, or without a terminal, hooks, settings, the status line and `PATH` are on and the launcher and system prompt are off. Every step is safe to repeat.

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
- **Started with `playbook cc` or `playbook ccd`:** the launcher adds the binary's directory when it is missing.
- **Started from a desktop app or IDE:** hooks find `playbook` only if its directory is already on that `PATH`. Use `playbook cc`, or install with `PLAYBOOK_BIN_DIR` set to a directory that environment has.

## Plugin only and the binary

`playbook init` has one flag pair per part: `--hooks`, `--statusline`, `--security`, `--path`, `--aliases`, `--system-prompt`, `--telemetry`, `--claude-override` and `--short-name`, each with a `--no-` form. Hooks, status line and `PATH` are on by default, the permissions rules (`--security`), launcher and system prompt are off.

With the plugins alone, the guards and status line are not wired, and `/playbook:doctor` reports them as missing. Run `/playbook:setup` to finish. The first session after a plugin-only install also prints one line: "playbook is installed as a plugin only. Run /playbook:setup to install the binary and guards." It goes away once the hooks are wired, and Homebrew and install script users never see it after `playbook init`.

When no binary exists and `playbook-hooks` is installed, the first bare `playbook` call from a command runs that plugin's installer once. It prints what it does first: it downloads the release, checks its SHA256, adds the install directory to your shell `PATH` and wires hooks in `settings.json`. To stop that, set `PLAYBOOK_NO_AUTO_INSTALL=1`, or create the file `~/.config/playbook/no-auto-install`. The shim then exits with the manual install command and installs nothing.

## Settings merge

`playbook init` merges the shipped template into your `settings.json` and never overwrites it. The template carries three things, each behind its own part: the `hooks` block, the `statusLine` and, only when you opt in, the `permissions` block. It ships no display options, notification toggles or `env` entries, because no hook or launcher reads them. Keys you customised stay as you set them. Keys an older template shipped (`cleanupPeriodDays`, `outputStyle`, `env` and the like) stay in your file too, and init never removes them.

The retired `--settings` and `--no-settings` flags are gone. Use `--hooks`, `--statusline` and `--security` and their `--no-` forms instead.

It tracks a baseline in `~/.claude/.settings.base.json` and compares it with the new template and your live file. After each run, `$CLAUDE_HOME/settings-merge-skipped.<epoch>.json` lists every key where your value won, as `{"key":"...", "template_had":..., "yours":...}`. Review it and adopt template values by hand if you want them.

The security defaults, the `permissions` block, are opt-in. A plain `playbook init` does not add them and never removes ones you have. Update control (the Claude Code autoupdater) is left to you, `playbook update` and Homebrew. Pass `--security` or set `security.defaults` to true to merge them. `permissions` is one top-level key. If you customised it, the whole block counts as contested and the template's version is withheld, so your rules win. See [Security](../../README.md#security).

If a run is interrupted between writing `settings.json` and the baseline, delete `.settings.base.json`. The next run treats the missing baseline as empty and only adds keys.

## Disabling the plugin

Disabling `playbook@pragmatic-engineer` in Claude Code pauses playbook. Every hook, including the safety guards, exits as a silent success and the status line prints nothing, while the entries `playbook init` wrote stay in `settings.json`. Enable the plugin again to resume. Only an explicit `false` pauses: with no entry (Homebrew only), `true`, or a `settings.json` that cannot be read, playbook runs as usual. To run hooks anyway, set `PLAYBOOK_IGNORE_PLUGIN_DISABLED=1` in the environment Claude Code starts in. See [Launcher and hooks](../internals/01-launcher-and-hooks.md#pausing-when-the-plugin-is-disabled).

## When the plugin was removed

`playbook init` notes in `~/.config/playbook/plugin-seen` that it saw the plugin installed. If the plugin later leaves Claude Code's `installed_plugins.json` (removed, which differs from disabled), the status line shows `playbook plugin was removed, run \`playbook uninstall --yes\` to clean up` and the SessionStart hook adds the same line once per session start. Nothing is deleted for you. Homebrew-only users never had the plugin and never see the line. Set `hints.pluginRemoved` to `false` to hide it (see [config keys](04-config-keys.md)).

## Uninstall

```bash
playbook uninstall --dry-run   # list what would go, change nothing
playbook uninstall --yes       # remove it
```

It removes only what playbook placed: the hook entries and status line in `settings.json`, the launcher block in your rc files, and the files under `~/.config/playbook` that playbook copied (such as the system prompt). Without `--yes` it lists the plan and exits with status 1.

Everything else stays: the rest of `settings.json` and your rc files, `.settings.base.json`, `backups/`, memory, the [data folder](../internals/11-data-folder.md) with every database, and all runtime state. Each changed file is backed up first (`settings.json.bak.<epoch>`). A status line script or system prompt you edited by hand is kept. If `settings.json` cannot be edited, such as invalid JSON, the placed files and the binary stay too.

At the end it lists the manual steps that still apply, because playbook never changes Claude Code's plugins: `claude plugin uninstall playbook@pragmatic-engineer` and `claude plugin uninstall playbook-hooks@pragmatic-engineer` while each is installed, and `claude plugin marketplace remove pragmatic-engineer` while the marketplace is listed.

`--remove-binary` also removes the binary, its `.bak` backups and the `PATH` block the installer added.

## Notes

Edits to `settings.json` or hooks take effect in a fresh session only. Run `playbook cc fresh` or plain `claude` after changing them. `playbook cc` warns when a resumed session runs on stale config.

## See also

- [Launcher and hooks](../internals/01-launcher-and-hooks.md)
- [The system prompt](../concepts/01-system-prompt.md)
- [Docs index](../index.md)
