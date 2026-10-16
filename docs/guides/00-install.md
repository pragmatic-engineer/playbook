# Install

To see every change `playbook setup` makes before you agree to it, read [What setup installs](06-what-init-installs.md) and the [hook reference](../reference/hooks.md).

Releases up to v0.34.0 have only the old name (older versions: `playbook init`). It takes the same flags, and it still works on newer releases ([ADR-0040](../adr/0040-rename-init-to-setup.md)).

The recommended route is the install script, one command, and `playbook update` keeps it current. Homebrew is supported too. This page covers every route, requirements, how the binary reaches `PATH`, upgrades, the settings merge and uninstall.

## Pick an install mode

| Mode | Commands, in order | You get | You lack |
|---|---|---|---|
| Skills only | `claude plugin marketplace add pragmatic-engineer/marketplace`, then `claude plugin install playbook@pragmatic-engineer` | Skills, commands and subagents. No hooks, no binary, no change to `settings.json` | Safety guards, status line, hooks, usage, and every step that needs the `playbook` binary |
| Binary only | Install script (recommended): `curl -fsSL https://raw.githubusercontent.com/pragmatic-engineer/playbook/main/install.sh \| bash`. Or Homebrew: `brew install pragmatic-engineer/tap/playbook`, then `playbook setup` | The `playbook` command, hooks, guards, status line, memory and usage | The skills and slash commands |
| Both (recommended) | The binary first, then the two skills commands | Everything | Nothing |

The skills are a Claude Code plugin and work on their own. The binary is optional. Installing one never installs the other, and playbook never installs or removes a plugin for you.

When the binary is installed, `playbook setup` adds its hook entries to `~/.claude/settings.json`. In a terminal it asks first. With no terminal and no `--yes`, it leaves your Claude Code settings alone and prints the command to run. `--hooks` or `--yes` is the explicit yes. It merges with the entries you already have. An entry counts as playbook's when its command is exactly one of the `playbook hook <name>` commands `setup` writes, and `playbook uninstall` removes only those. The old `playbook-hooks` plugin is retired. If it is still installed, remove it with `claude plugin uninstall playbook-hooks@pragmatic-engineer`, because its hooks would run next to the ones in `settings.json`.

Which one? Choose skills only if you want to read the skills and nothing else runs on your machine. Choose both for everything else. The binary is a hard dependency of the hooks, guards, status line and usage. Only the skill text works without it. See [Skills without the binary](#skills-without-the-binary).

## Requirements

| Tool | Status | Why |
|---|---|---|
| `claude` | only for the skills | The skills are a Claude Code plugin. The binary, hooks, guards and settings do not need it. |
| `bash` or `zsh` | required | The `playbook` shell wrapper works in both. |
| `git`, `shasum` | required | Used by hooks and the installer. |
| `gh` | optional | Status line PR and CI info, and the build attestation check. |
| `agent-browser` | optional | Browser automation `/playbook:plan` uses for web-only tickets. |

## Install

### Install script (recommended)

```bash
curl -fsSL https://raw.githubusercontent.com/pragmatic-engineer/playbook/main/install.sh | bash
```

It installs only the `playbook` binary, into `PLAYBOOK_BIN_DIR` (default `~/.local/bin`), after checking its SHA256, its build attestation when `gh` can check it, and a `--version` smoke test. It puts that directory on `PATH`. It does not touch Claude Code's plugins or settings.

When it replaces an older binary, it prints the breaking changes between the two versions, with the step to take for each.

Then it asks "Run playbook setup now?". Only a typed `y` or `yes` counts, so a bare Enter is a no. On yes it asks about the launcher and the system prompt (Enter means yes for these two), then runs `playbook setup --yes`, which adds the hook entries, the status line and the telemetry variables (a collector on this machine) to `settings.json`. On no, or with no terminal and no `--yes`, it skips `setup` and prints the command to run later, with any `--aliases` or `--system-prompt` you passed. Open a new terminal afterward.

Pass `--yes` to say yes to every question, including `playbook setup`. Flags go after `-s --` when piping:

| Flag | Effect |
|---|---|
| `--yes`, `-y` | Say yes to every question, including running `playbook setup`. |
| `--no-init` | Install the binary only. Do not ask about `playbook setup`. |
| `--aliases` | Add the `playbook` cd wrapper to your rc file without asking. |
| `--system-prompt` | Install the system prompt without asking (implies `--aliases`). |
| `--binary-only` | Still offers `playbook setup`, but skips the launcher and system prompt questions. |
| `--ref <tag>` | Release tag, same as `PLAYBOOK_REF`. |

`--skip-plugin`, `--no-setup`, `--skip-deps` and `--skip-shell` are accepted and ignored, so older commands keep working. `--skills-only` is gone and exits with the plugin commands. The other `setup` options (`--claude-override`, `--short-name`) belong to `playbook setup`; run it yourself to pass them, and install.sh also links `pb` to `playbook` when the name is free.

Pin a version with `PLAYBOOK_REF`:

```bash
curl -fsSL https://raw.githubusercontent.com/pragmatic-engineer/playbook/main/install.sh | PLAYBOOK_REF=v0.34.0 bash
```

A release tag installs that release. A branch or commit is refused, because it has no published binary.

### Homebrew

```bash
brew install pragmatic-engineer/tap/playbook
playbook setup
```

`playbook setup` asks before it adds the hooks to `settings.json`, and writes the `PATH` block so hook shells find the binary even when Homebrew's directory is not on their `PATH`. Homebrew ships only the binary, so `setup` seeds the settings template and the system prompt from copies compiled into it (written to `~/.config/playbook/embedded/`) when no Claude Code plugin is installed. An installed plugin, even an older version, takes precedence. Homebrew installs update with `brew upgrade playbook`, and `playbook update` refuses a Homebrew install and tells you to use brew.

### Optional: pbtop alone

You do not need this if you have `playbook`. `playbook usage` runs the same code, because playbook embeds the `playbook-telemetry` crate. Install `pbtop`, the usage dashboard, only on a machine that has no `playbook`. From release 0.35.0 each `vX.Y.Z` release has `pbtop-X.Y.Z-<target>.tar.gz`, which holds `pbtop` alone, and there are two ways to install it:

```bash
curl -fsSL https://raw.githubusercontent.com/pragmatic-engineer/playbook/main/install-pbtop.sh | sh
brew install pragmatic-engineer/tap/pbtop
```

`install-pbtop.sh` needs `bash` and `curl`. It downloads `install.sh` from `main` and runs it in its pbtop mode, which checks the archive like the `playbook` binary (its line in `SHA256SUMS`, the build attestation when `gh` can check it, and `pbtop --version`), installs `pbtop` into `PLAYBOOK_BIN_DIR` (default `~/.local/bin`) and puts that folder on `PATH`. It runs nothing else. To pin a release, set `PLAYBOOK_REF` for `sh`, as in `curl -fsSL .../install-pbtop.sh | PLAYBOOK_REF=v0.35.0 sh`. On a release before 0.35.0 it stops, because those have no `pbtop` archive. The old `install.sh --pbtop` still works for one more release and points to `install-pbtop.sh`.

Type `pbtop`, `pbtop --summary`, `pbtop --json` or `pbtop --range 30d`. The [crate README](../../crates/telemetry/README.md) covers `cargo` builds. Up to playbook-telemetry 0.2.0 the telemetry archives had their own `telemetry-vX.Y.Z` tags. Everything stays on your machine.

The tool shares its data with playbook: the database is `~/.config/playbook/data/usage.db` and the settings are the `usage.*` keys in `~/.config/playbook/data/config.db`. See [Config keys](04-config-keys.md).

### Coming: the standalone code index

`playbook code` will also get a standalone tool, [playbook-code](../../crates/code), with the short alias `pbcode`. It has no release yet, so there is nothing to install. Use `playbook code` today.

### Without `curl | bash`

1. From the [latest release](https://github.com/pragmatic-engineer/playbook/releases/latest), download the asset for your platform and `SHA256SUMS`.

   | Platform | Asset |
   |---|---|
   | macOS, Apple silicon | `playbook-<version>-aarch64-apple-darwin` |
   | macOS, Intel | `playbook-<version>-x86_64-apple-darwin` |
   | Linux, x86_64 | `playbook-<version>-x86_64-unknown-linux-musl` |
   | Linux, arm64 | `playbook-<version>-aarch64-unknown-linux-musl` |
   | Windows | `playbook-<version>-x86_64-pc-windows-msvc.exe` |

2. Verify it. `SHA256SUMS` is not signed, so it is only a corruption check. The attestation shows where the binary was built.

   ```bash
   grep "  <asset>$" SHA256SUMS | shasum -a 256 -c -
   gh attestation verify <asset> --repo pragmatic-engineer/playbook
   ```

   `install.sh` and `playbook update` run the attestation check when `gh` is available. They warn when `gh` is missing or the release predates attestations, and abort on a real mismatch. Set `PLAYBOOK_REQUIRE_ATTESTATION=1` to abort on a missing check too.

3. Put it on `PATH`:

   ```bash
   mkdir -p ~/.local/bin
   mv <asset> ~/.local/bin/playbook
   chmod 0755 ~/.local/bin/playbook
   ```

4. Wire the local config:

   ```bash
   playbook setup
   ```

   In a terminal, `setup` asks about hooks, settings, the status line, `PATH`, the launcher and the system prompt. The status line question reads "Use the playbook status line?" and has `--statusline` and `--no-statusline`. Answering no leaves `statusLine` in `settings.json` as it is, and an existing playbook status line stays until `playbook uninstall`. Each part has `--x` and `--no-x` flags. With `--yes`, hooks, settings, the status line and `PATH` are on and the launcher and system prompt are off. Without a terminal and without `--yes`, the hooks and the status line stay off unless you pass `--hooks` or `--statusline`. Every step is safe to repeat.

5. For the skills, run the two plugin commands from [Pick an install mode](#pick-an-install-mode). Then run `/playbook:doctor`, or `playbook doctor check` without the skills.

## Upgrade

| Install | Command |
|---|---|
| Install script (recommended) | `playbook update`, or run the installer again (`--check` to look, `--list` for releases, `--pre` for pre-releases, `--yes` in auto mode) |
| Homebrew | `brew upgrade playbook` |

`playbook update` verifies SHA256SUMS and the attestation, keeps the old binary as `playbook.<version>.bak` (the newest three are kept), refuses a Homebrew install, and warns when another `playbook` earlier on `PATH` hides the new one. The binary and the plugin share one version line ([ADR 0038](../adr/0038-one-release-line.md)), and they work together while they are on the same MAJOR.MINOR. After a binary update to a new minor line, update the plugin with `claude plugin update playbook@pragmatic-engineer`, and `playbook doctor` warns until you do. A binary pinned with `PLAYBOOK_REF` gets the same warning once the plugin moves to a newer minor line. To roll back, move a backup over the binary:

```bash
mv -f ~/.local/bin/playbook.0.19.0.bak ~/.local/bin/playbook
```

### Update notices and automatic updates

Playbook looks for a newer release on its own, the way Claude Code does. The `update.mode` setting picks what happens:

| `update.mode` | What happens |
|---|---|
| `notify` (default) | A command run in a terminal prints one line on stderr when a newer release exists: `playbook X.Y.Z is available (you have A.B.C). Run: playbook update`. Homebrew installs see `brew upgrade playbook`. On an install that can update itself, the line also names `playbook config set --global update.mode auto`. The line shows at most once in 24 hours for each version. |
| `auto` | Playbook installs the release in the background, with the same checks as `playbook update`, and runs the new binary's migrations. The command you ran finishes on the old binary. The next command you run in a terminal prints `playbook updated from A.B.C to X.Y.Z` and any breaking changes. |
| `off` | No check runs. |

Turn on automatic updates with:

```bash
playbook config set --global update.mode auto
```

`update.mode` and `update.checkIntervalHours` are settings for each user, so `config set` writes them to the global tier even without `--global` and refuses `--org`.

How the check works:

- A command reads the last result from `~/.config/playbook/data/config.db`. When that result is older than `update.checkIntervalHours` (24 by default), the command starts a background process that asks the GitHub releases API for the newest `vX.Y.Z` release. No command waits on the network. The request is unauthenticated, and its only identifying header is the `playbook/<version>` user agent. GitHub sees your IP address, as with any request. In `auto` mode the background process also downloads the release and checks its attestation through `gh`, the same requests `playbook update` makes. Offline, rate limited or behind a proxy, the check stays silent and runs again after the next interval.
- The check does not run for hooks, the status line, `shell-init`, `version`, `json`, `update`, the background `usage warm` and `usage collect` runs, any run with `--json`, or when a CI variable or `PLAYBOOK_HEADLESS` is set. The notice needs a terminal on stderr. An automatic update does not.
- `auto` falls back to the notice, with the reason, in these cases: a Homebrew install; a platform `playbook update` does not support, such as Windows; a development or release candidate build; a release less than an hour old, or one whose publish time is unknown; an install directory you cannot write; another update in progress; a release whose breaking changes need a step from you or cannot be read; and a build attestation that cannot be checked, for example when `gh` is missing. A failed automatic update prints one warning with the fix. When an attempt never runs, for example on an offline machine, the notice shows after 15 minutes.
- `playbook update --check` and `playbook doctor` show the mode, the last check and what it found.

### Coming from the playbook-hooks plugin

Older releases delivered the hooks in a second plugin, `playbook-hooks`. The hooks now live in `~/.claude/settings.json`. After you upgrade the binary, run these two commands once:

```bash
claude plugin uninstall playbook-hooks@pragmatic-engineer
playbook setup --yes
```

`playbook setup` adds the hook entries and asks first when you run it in a terminal. Leaving the old plugin installed makes every hook run twice, and `playbook doctor check` points it out.

See [release channels](../internals/06-release-channels.md) for how a release reaches each channel.

## How the binary gets on PATH

Every route ends with `playbook` on `PATH` for each shell start.

| Shell | File that gets the `# playbook binary` block |
|---|---|
| zsh | `~/.zshenv` (read by every zsh, including non-interactive ones) |
| bash | `~/.bashrc` and the login file (`~/.bash_profile`, `~/.bash_login` or `~/.profile`, whichever exists first) |
| fish | `~/.config/fish/conf.d/playbook.fish` |
| other | `~/.profile` |

`install.sh` and `playbook setup` write the same block and skip a file that already names the directory.

Hooks do not read these files. Measured on Claude Code 2.1.293 (2026-10-09), a hook runs in a non-interactive `bash -c` with exactly the `PATH` of the process that started Claude Code. So:

- **Started from a terminal:** the inherited `PATH` has the block's directory. Hooks find `playbook`.
- **Started with `playbook cc` or `playbook ccd`:** the launcher adds the binary's directory when it is missing.
- **Started from a desktop app or IDE:** hooks find `playbook` only if its directory is already on that `PATH`. Use `playbook cc`, or install with `PLAYBOOK_BIN_DIR` set to a directory that environment has.

## Setup flags

`playbook setup` has one flag pair per part: `--hooks`, `--statusline`, `--security`, `--path`, `--aliases`, `--system-prompt`, `--telemetry`, `--claude-override` and `--short-name`, each with a `--no-` form. With `--yes` or in a terminal, hooks, status line, `PATH` and telemetry are on by default. Without a terminal and without `--yes`, hooks, the status line and telemetry stay off unless their flag is passed. `--only <part>[,<part>]` runs just the named parts, so `playbook setup --only telemetry` turns telemetry on and touches nothing else. A bare `--telemetry` (no `--yes` and no other part flag) means the same. The permissions rules (`--security`), launcher and system prompt are off.

## Skills without the binary

With the skills plugin alone, some commands call the `playbook` binary. They fall back to plain git, gh and file steps where they can. No hook runs, so the guards and status line are not active, and `/playbook:doctor` reports the binary and the hook entries as missing. Install the binary and run `playbook setup` to add them.

## Settings merge

`playbook setup` merges the shipped template into your `settings.json` and never overwrites it. The template carries three things, each behind its own part: the `hooks` block, the `statusLine` and, only when you opt in, the `permissions` block. It ships no display options, notification toggles or `env` entries, because no hook or launcher reads them. Keys you customised stay as you set them. Keys an older template shipped (`cleanupPeriodDays`, `outputStyle`, `env` and the like) stay in your file too, and setup never removes them.

The retired `--settings` and `--no-settings` flags are gone. Use `--hooks`, `--statusline` and `--security` and their `--no-` forms instead.

It tracks a baseline in `~/.claude/.settings.base.json` and compares it with the new template and your live file. After each run, `$CLAUDE_HOME/settings-merge-skipped.<epoch>.json` lists every key where your value won, as `{"key":"...", "template_had":..., "yours":...}`. Review it and adopt template values by hand if you want them.

The security defaults, the `permissions` block, are opt-in. A plain `playbook setup` does not add them and never removes ones you have. Update control (the Claude Code autoupdater) is left to you, `playbook update` and Homebrew. Pass `--security` or set `security.defaults` to true to merge them. `permissions` is one top-level key. If you customised it, the whole block counts as contested and the template's version is withheld, so your rules win. See [Security](../../README.md#security).

If a run is interrupted between writing `settings.json` and the baseline, delete `.settings.base.json`. The next run treats the missing baseline as empty and only adds keys.

## Disabling the plugin

Disabling `playbook@pragmatic-engineer` in Claude Code pauses playbook. Every hook, including the safety guards, exits as a silent success and the status line prints nothing, while the entries `playbook setup` wrote stay in `settings.json`. Enable the plugin again to resume. Only an explicit `false` pauses: with no entry (a binary-only install), `true`, or a `settings.json` that cannot be read, playbook runs as usual. To run hooks anyway, set `PLAYBOOK_IGNORE_PLUGIN_DISABLED=1` in the environment Claude Code starts in. See [Launcher and hooks](../internals/01-launcher-and-hooks.md#pausing-when-the-plugin-is-disabled).

## Uninstall

```bash
playbook uninstall --dry-run   # list what would go, change nothing
playbook uninstall --yes       # remove it
```

It removes only what playbook placed: the hook entries whose command is exactly one `playbook hook <name>` command that `setup` writes and status line in `settings.json`, the launcher block in your rc files, and the files under `~/.config/playbook` that playbook copied (such as the system prompt). Without `--yes` it lists the plan and exits with status 1.

Everything else stays: the rest of `settings.json` and your rc files, `.settings.base.json`, `backups/`, memory, the [data folder](../internals/11-data-folder.md) with every database, and all runtime state. Each changed file is backed up first (`settings.json.bak.<epoch>`). A status line script or system prompt you edited by hand is kept. If `settings.json` cannot be edited, such as invalid JSON, the placed files and the binary stay too.

At the end it lists the manual steps that still apply, because playbook never changes Claude Code's plugins: `claude plugin uninstall playbook@pragmatic-engineer` while it is installed, `claude plugin uninstall playbook-hooks@pragmatic-engineer` if the retired hooks plugin is still there, and `claude plugin marketplace remove pragmatic-engineer` while the marketplace is listed.

`--remove-binary` also removes the binary, its `.bak` backups and the `PATH` block the installer added.

## Notes

Edits to `settings.json` or hooks take effect in a fresh session only. Run `playbook cc fresh` or plain `claude` after changing them. `playbook cc` warns when a resumed session runs on stale config.

## See also

- [Launcher and hooks](../internals/01-launcher-and-hooks.md)
- [The system prompt](../concepts/01-system-prompt.md)
- [Docs index](../index.md)
