# Playbook

A pragmatic Claude Code toolkit: opinionated skills, slash commands, subagents, and safety and state hooks for planning, review, memory, and guarded editing. It ships as a Claude Code plugin so it works in any shell. An optional local setup layer wires the always-on safety guards, seeds `settings.json`, and adds shell launchers and a custom system prompt.

## Quick start (1 command)

```bash
curl -fsSL https://raw.githubusercontent.com/pragmatic-engineer/playbook/main/install.sh | bash
```

That is the primary path. It adds the marketplace, installs and enables the
plugin, **installs the `playbook` binary**, wires the 5 safety guards and the 13
functional hooks, seeds or merges `settings.json`, and installs the status line.
It asks before adding the shell launchers and the custom system prompt; pass
`--yes` to accept every default, or `--system-prompt` to take both without
prompting.

Then open a Claude Code session and run `/playbook:doctor` to verify.

### Why not plugin-install on its own

`claude plugin install` gives you the skills, commands and subagents, and
nothing else. It does **not** install the `playbook` binary, and every ported
hook is a bare `playbook hook <name>` command, so without the binary all 20 are
dead and the guards stay unwired.

`/playbook:setup` closes that gap: it installs the release binary into
`~/.local/bin` when one is not already on `PATH`, verifying it against the
release's `SHA256SUMS` first. So plugin-install followed by `/playbook:setup`
also reaches a working state; the one-liner above is simply the shorter route
and does not need a Claude Code session.

If you want only the plugin content and no local layer, that is a supported
choice:

```bash
claude plugin marketplace add pragmatic-engineer/marketplace
claude plugin install playbook@pragmatic-engineer
```

Expect `/playbook:doctor` to report the binary, the guards and the status line
as missing. That is correct for this path, not a broken install.

The marketplace serves a trimmed plugin archive (plugin files only, no Rust
source, tests or docs), which needs Claude Code 2.1.224 or later. Older
versions cannot install the plugin from the marketplace.

This route (and `/playbook:setup`) has no equivalent of `install.sh`'s
minimum-claude-CLI-version check: Claude Code's plugin system has no hook for
it. See [docs/guides/00-install.md](docs/guides/00-install.md#requirements)
for the version this toolkit assumes going forward.

### Install without `curl | bash`

Piping a script into a shell is a reasonable thing to refuse. Two routes avoid
it, both fully supported.

**Route 1: plugin, then `/playbook:setup`.** Nothing is piped anywhere.

```bash
claude plugin marketplace add pragmatic-engineer/marketplace
claude plugin install playbook@pragmatic-engineer
```

Then in a Claude Code session run `/playbook:setup`. It installs the release
binary itself (checksum-verified) when one is not already on `PATH`, wires the
guards and settings, and offers the launchers and system prompt. Verify with
`/playbook:doctor`.

**Route 2: fully manual.** Every step auditable, nothing downloaded by a script
you have not read.

1. Install the plugin, so the skills, commands and subagents are present:

   ```bash
   claude plugin marketplace add pragmatic-engineer/marketplace
   claude plugin install playbook@pragmatic-engineer
   ```

2. From the [latest release](https://github.com/pragmatic-engineer/playbook/releases/latest),
   download the asset for your platform and the `SHA256SUMS` file:

   | Platform | Asset |
   |---|---|
   | macOS, Apple silicon | `playbook-<version>-aarch64-apple-darwin` |
   | macOS, Intel | `playbook-<version>-x86_64-apple-darwin` |
   | Linux, x86_64 | `playbook-<version>-x86_64-unknown-linux-musl` |
   | Linux, arm64 | `playbook-<version>-aarch64-unknown-linux-musl` |
   | Windows | `playbook-<version>-x86_64-pc-windows-msvc.exe` |

3. Verify the download. `SHA256SUMS` lists all five assets, so check only your
   own line:

   ```bash
   grep "  <asset>$" SHA256SUMS | shasum -a 256 -c -
   ```

   `SHA256SUMS` is not signed: its integrity rests on TLS and on trusting
   github.com, not on a cryptographic signature. Treat it as a corruption check.

   To verify where a release binary was built, use the GitHub CLI:

   ```bash
   gh attestation verify <asset> --repo pragmatic-engineer/playbook
   ```

   `install.sh` runs this check when `gh` is available. It only warns when
   `gh` is missing, not logged in, or the release predates attestations; a
   real mismatch always aborts. Set `PLAYBOOK_REQUIRE_ATTESTATION=1` to make a missing or failed
   check abort the install.

   Once installed, `playbook update` applies the same checks (SHA256SUMS, then the attestation,
   with the same `PLAYBOOK_REQUIRE_ATTESTATION=1` rule) to fetch a newer release. It keeps the
   old binary as `playbook.<version>.bak`, refuses a Homebrew install, and warns when another
   `playbook` earlier on `PATH` hides the new one. Use `--check` to look without installing and
   `--list` to see releases.

4. Put it on `PATH` as `playbook`:

   ```bash
   mkdir -p ~/.local/bin
   mv <asset> ~/.local/bin/playbook
   chmod 0755 ~/.local/bin/playbook
   ```

   Add `~/.local/bin` to `PATH` in your shell rc if it is not already there.

   On macOS or Linux, `brew install pragmatic-engineer/tap/playbook`
   does steps 2-4 for you: it fetches the same checksummed release binary
   and puts it on `PATH`. No separate build, no separate release pipeline.

5. Wire the local configuration. **`CLAUDE_PLUGIN_ROOT` is required**: without
   it `init` has no template to copy from and skips almost every step, reporting
   `skipped - CLAUDE_PLUGIN_ROOT is not set`.

   ```bash
   CLAUDE_PLUGIN_ROOT=~/.claude/plugins/cache/pragmatic-engineer/playbook/<version> \
     playbook init
   ```

   Add `--system-prompt` to also install the custom system prompt. Every step
   is idempotent, so re-running is safe.

6. Verify with `/playbook:doctor` in a Claude Code session.

Route 2 does not install the `cc`/`ccd` shell launchers unless you ask:
`playbook init --aliases` installs them and wires your rc file. You can also
run `/playbook:setup`.

For requirements, pinning a version, and uninstall, see
[docs/guides/00-install.md](docs/guides/00-install.md).

## Layers

Playbook has seven layers, numbered to match what `/playbook:doctor` reports.
Layers 1, 2 and 6 are what a working install needs; 3, 4, 5 and 7 are
optional or cosmetic.

| Layer | When | What it does |
|---|---|---|
| 1. Plugin content | Always, after `claude plugin install` | Skills, commands and subagents load from the plugin. No files written to `~/.claude`. Only the worktree hooks and a SessionStart migration check are registered by the plugin. The 13 functional hooks and 5 guards are written into `settings.json` by `playbook init` (Layer 2) and need the binary (Layer 6). |
| 2. Safety guards and settings | After `install.sh`, or `/playbook:setup` | Wires the guards and seeds or merges `~/.claude/settings.json`. `install.sh` wires them as `playbook hook <name>`; `/playbook:setup` still copies the legacy `~/.claude/hooks/*.sh` scripts, which `/playbook:doctor` reports as not wired. |
| 3. Shell launchers | Opt-in (recommended) | Adds `cc` and `ccd` to `~/.bashrc` or `~/.zshrc`. Both shells work; `cc clean` and `cc raw` are zsh-only (see Usage). |
| 4. Custom system prompt | Opt-in (recommended) | Copies `prompts/SYSTEM_PROMPT.md` to `~/.config/playbook/prompts/`; `cc` passes it via `--system-prompt-file`. Plugin content works without it. |
| 5. Status line | After `install.sh` | Runs from the binary (`playbook statusline`), and `playbook init` points `statusLine.command` at it. `/playbook:doctor` checks it. `playbook init` and `install.sh` also mark `~/.config/playbook` as a trusted workspace in `~/.claude.json` (through `playbook trust`), so a `claude` session started directly in that folder doesn't hit the trust dialog and silently skip the status line. Best-effort: a missing `~/.claude.json` is left alone. |
| 6. The `playbook` binary | `install.sh` or `/playbook:setup` | Installs the release binary to `~/.local/bin`, checksum-verified. **Every ported hook is a bare `playbook hook <name>` command, so without this all 20 are dead.** `claude plugin install` alone does not provide it. |
| 7. No dangling hook commands | Always | Flags any `settings.json` hook command pointing at a file that no longer exists, such as a leftover pre-migration Python hook a settings merge never removed. Fails open (silent no-op) if unchecked. |

## Usage

```bash
cc                     # resume this directory's last session, or start fresh
ccd                    # same, with --dangerously-skip-permissions
cc fresh               # new session, no history
cc list                # recent sessions for this directory
cc worktree <branch>   # create/enter a git worktree, then start a session there
cc new <branch>        # alias for cc worktree
cc prune               # prune old transcripts now
cc clean               # resume with /model, /effort, /config, /output-style, /style stripped
cc raw [id]            # resume verbatim, no fork or cleanup
```

`pb` and `pbd` are the same two commands under names that do not clash with the C compiler (`cc`). Use them everywhere you see `cc` and `ccd` below. To stop playbook defining `cc` and `ccd` at all, load the functions with `eval "$(playbook shell-init --no-cc)"`.

`cc` loads the system prompt (when installed), picks a model, and prunes old transcripts (keeps the newest 5; set `CCD_KEEP` to change, `CCD_KEEP=0` disables).

**One launcher, both shells.** `cc` and `ccd` are two small shell functions printed by `playbook shell-init`, and the work happens in the binary (`playbook cc launch`). So bash and zsh behave identically: every subcommand above, the config-drift auto-fork on the default resume, and retention all work the same in either shell. The rc file holds one line, `command -v playbook >/dev/null 2>&1 && eval "$(playbook shell-init)"`; `/playbook:setup --install-aliases` (or `playbook init --aliases`) writes it, and also rewrites an older `source .../cc.sh` line to it.

`cc worktree` (also `ccd worktree`) groups worktrees under `<repo-parent>/.worktrees/<repo>/<folder>` (set `WORKTREE_BASE_DIR` to change the base folder), names the folder after the JIRA key in the branch name, and copies `.env` into it. It also clones `node_modules`, pushes to set upstream, and offers AI-assisted rebase conflict resolution. The engine lives in the binary (`playbook cc worktree`), and the shell function `cd`s into the new worktree afterward. See [docs/internals/03-worktree.md](docs/internals/03-worktree.md) for the full behaviour.

## Commands

Slash commands live in `commands/`. See [docs/guides](docs/guides) for full usage.

| Command | What it does |
|---|---|
| `/playbook:setup` | Wires the guards, seeds `settings.json`, and installs what you choose. Safe to run repeatedly. |
| `/playbook:doctor` | Checks the seven layers and prints a pass/info table with a remediation hint for each miss. |
| `/playbook:plan` | One continuous session from a raw idea or a settled direction to a verified, parallel-safe plan saved for `/playbook:implement`. |
| `/playbook:implement` | Executes a `/playbook:plan` plan or `/playbook:adr` blueprint with subagents and TDD, committing each work unit. `--auto` opens a PR. |
| `/playbook:fix` | Fixes one small bug end to end: a failing test, the smallest fix, one pull request. Hands off to `/playbook:plan` when the fix is not small. |
| `/playbook:adr` | Creates an Architecture Decision Record through investigate, draft, quality-gate, finalise. Saves to `docs/adr/`. |
| `/playbook:commit-and-push` | Writes a commit message from the staged diff, commits signed, optionally rebases, then pushes. |
| `/playbook:create-pull-request` | Opens a PR with pre-flight checks, a conventional-commit title, and the team PR template. |
| `/playbook:quick-review` | Single-pass PR review using the `grounding-review` discipline, posted as a pending GitHub review. |
| `/playbook:deep-review` | Multi-agent PR review; spawns specialist subagents in parallel, consolidates findings, posts a pending review. |
| `/playbook:address-pr-comments` | Walks unresolved PR comments, applies fixes or drafts replies, then pushes and posts replies. |
| `/playbook:learn-project` | Analyses the repo (git history, code, PRs, JIRA/Confluence) and writes distilled facts to memory. Read-only; confirms before writing. |
| `/playbook:repo-audit` | Read-only four-phase repository audit (discovery, findings, strategy, task plan). |
| `/playbook:session-start` | Loads the handoff the last session saved for this directory and orients the session from it, with no copy and paste. |

### PR and commit settings

Four of the config keys shape how far the PR flow goes (the full list is in [Config keys](docs/guides/04-config-keys.md), or run `playbook config list`). Each user, org or repo can set them. Set one in your global config to apply it everywhere:

```bash
playbook config set --global autoReview.fix true       # fix every finding the self-review confirms, then push
playbook config set --global autoMerge.enabled true    # mark ready, wait for green checks, then merge with --auto
playbook config set --global commit.signOff false      # stop adding Signed-off-by (default: true)
playbook config set --global pr.draft false             # open PRs ready for review (default: true, draft)
```

`autoReview.fix` and `autoMerge.enabled` default to `false`. Auto-merge never uses `--admin` or `--delete-branch`, and never merges with a failing or unfinished check or, in auto mode, a PR no review covered. A PR with no checks at all merges once a two-minute look finds none. `commit.signOff` defaults to `true`. `pr.draft` defaults to `true`, so PRs open as drafts; set it to `false` to open them ready for review. See [PR and commit settings](docs/guides/02-review-and-pr-flow.md#pr-and-commit-settings) for the full rules.

## Auto mode

Auto mode lets a command run without asking you questions. It takes the recommended answer at each decision, writes it to an Assumptions list in the final output, and keeps going. The default is `ask`, where every command asks as usual.

Set the mode with the `mode` setting (`ask` or `auto`). Playbook picks the first one it finds:

1. The command flag: `--auto` or `--ask`.
2. The `PLAYBOOK_MODE` environment variable.
3. The `mode` config setting.

```bash
playbook mode auto      # turn auto on for this repo
playbook mode ask       # turn it off
playbook mode status    # show the mode and where it came from
```

`playbook mode auto` is saved in the repo config, so it stays on until you turn it off. Only you should run these. If auto comes from `PLAYBOOK_MODE`, unset the variable to leave it.

**Plan design.** Plain `--auto` never approves a design for you. `/playbook:plan --auto` answers only the Work Unit and Segment questions and stops at the design approval. Pass `--auto-design` to let it approve the design too. Every choice it makes is logged.

**What auto will not do.**

- `/playbook:setup`, `/playbook:adr`, and `/playbook:address-pr-comments` refuse to run in auto and say why.
- Reviews never post to GitHub. `/playbook:quick-review` and `/playbook:deep-review` run as `--self`.
- `/playbook:learn-project` stages its candidates and writes nothing to live memory.
- Auto never force-pushes.
- Auto doesn't run the self-review on a new pull request.
- `/playbook:implement` never picks the `land` boundary by itself. You must pass `--boundary=land`.

**What the hooks do.** Two hooks watch every session whose resolved mode is `auto`:

- The question hook (`auto-guard`) denies the question tool, so the model takes the recommended option instead. It also adds a one line reminder to each prompt.
- The spend cap hook totals what the session has cost. At `auto.warnPct` (default 70) of `auto.budgetUsd` (default 5, in US dollars) it warns once. At the cap it denies every tool except reading files, `git status`, `git diff`, `playbook mode status`, and writing a short note about where the work stopped. Cost is the larger of two numbers: the session transcript total, including subagent files, and the cost reported by the status line. If the cost can't be read, the hook treats it as over the cap: it stops the session and asks the model to report, without a park note. The cap is per session, so `/clear` or a resume starts a new count. Change it with `playbook config set auto.budgetUsd <usd>`.

While auto is on, the model cannot switch the mode, change the `mode`, `auto.*`, or `fix.*` settings, or edit the config files. Only you can, by typing the command yourself. At the cap it also cannot raise the cap. The one exception is when the cost is unreadable: then it may run `playbook mode ask` to leave auto.

**Flags do not reach hooks.** The hooks have no access to your command flags. They read only `PLAYBOOK_MODE` and the config. If you run a command with `--auto` while the resolved mode is `ask`, you get auto behavior in the command text only: no question hook and no spend cap. `playbook mode status --flag auto` prints a warning line when the flag and the hooks disagree. To get the hooks, run `playbook mode auto` or set `PLAYBOOK_MODE=auto`. The reverse also holds: `--ask` while the mode is auto still has the question tool denied.

**Limits.** The cap and the hooks are a guardrail against runaway spend and honest mistakes. They are not a security boundary against a model that tries to defeat them. Auto does not answer permission prompts, so run it in a permission mode you trust or the session can stall. `PLAYBOOK_HEADLESS` is a separate setting and does not turn auto on. See [docs/internals/05-headless-mode.md](docs/internals/05-headless-mode.md).

## Models, effort and agents

Each command and agent names its model and effort level in its own frontmatter. The rule is to spend on judgment and save on routine work.

| Model | Used for |
|---|---|
| Opus | Design (`/playbook:plan`, `/playbook:adr`), every reviewer, the repo auditor |
| Sonnet | The session default, the implementer, critic, fact-checker, test-reviewer and analyst |
| Haiku | Mechanical agents: `git`, `patch-applier`, `collector`, `cheap-checker`, `review-triage` |

Effort is a second setting next to the model. It is lowered where the work is mechanical or already decided, and never where a missed finding is costly. The `Agent` tool has no per-call effort, so a role that needs two efforts ships as variants (`reviewer-low`, `reviewer-xhigh`), generated by `playbook agents gen` and checked by `playbook agents check`. Skills never set a model or effort, because the caller decides.

Set a ceiling with `playbook effort <level>` (`auto`, `low`, `medium`, `high`, `xhigh` or `max`). `auto` keeps the shipped values. Any other level caps everything: an agent that ships at `xhigh` runs at `medium` under `medium`. The cap is written to `maxEffortLevel` in `~/.claude/settings.json`, which Claude Code 2.1.267 or later applies to command, skill and agent effort alike. Planned: per component ceilings (#573) and a 5.5 model rule with fallback (#574). For the full reasoning, see [Why the pieces are shaped this way](docs/concepts/03-why-the-pieces-are-shaped-this-way.md) and the [effort policy table](docs/internals/02-model-routing-and-memory.md#effort-policy).

## Skills

Skills live in `skills/` and load on demand. See [docs/authoring/01-commands-skills-hooks.md](docs/authoring/01-commands-skills-hooks.md).

| Skill | What it does |
|---|---|
| `grounding-review` | Review discipline; severity levels, Conventional Comments, proof ladder, verification summary. |
| `grounding-research` | Investigation discipline; citation rules (every claim sourced to `file:line`), structured findings, scope boundaries. |
| `engineering-standards` | Code design rules, PR readiness, test types, mocking rules, incremental delivery, deployment flow. |
| `engineering-standards-javascript` | JS/TS companion to `engineering-standards`; covers Zod validation and Jest/Vitest mocking. |
| `writing-style` | Voice rules for human-facing prose; spartan, active voice, contractions, no dashes. |
| `session-handoff` | Decision-first handoff, saved with `playbook handoff save`, so the next session picks up cold without rereading the thread. |
| `delegating-subagents` | When and how to dispatch subagents, which tier to pick (including the `-low` and `-xhigh` variants), and what to do when one finishes or goes quiet. |
| `systematic-debugging` | Reproduce, isolate and root-cause a bug before proposing a fix. |
| `finish-pull-request` | Runs right after `/playbook:create-pull-request`: the self-review `autoReview.*` selects, fixes, promotes the draft, and merges when `autoMerge.enabled` allows. |
| `playbook-usage` | Discovery aid: which `/playbook:*` command fits a task, for sessions without the system prompt. |
| `atlassian-cli` | Read and write Jira and Confluence through `acli`, used by `/playbook:learn-project` when no Atlassian MCP server is connected. |

## Docs

Full documentation: [`docs/index.md`](docs/index.md).

- **Concepts** (`docs/concepts/`): system prompt design, the memory system, and [why commands, skills, agents, models and effort levels are shaped the way they are](docs/concepts/03-why-the-pieces-are-shaped-this-way.md).
- **Guides** (`docs/guides/`): install, plan-and-implement, review and PR flow, decisions and memory, config keys.
- **Authoring** (`docs/authoring/`): writing commands, skills, and hooks.
- **Internals** (`docs/internals/`): launcher and hooks, model routing and memory, worktree engine, usage dashboard, headless mode, release channels.

## System prompt

`prompts/SYSTEM_PROMPT.md` sets the persona and session rules `cc` loads on every session, when installed. See [docs/concepts/01-system-prompt.md](docs/concepts/01-system-prompt.md).

## Memory

One markdown store at `~/.config/playbook/memory/`, global and per-project, local-only and never committed. See [docs/concepts/02-memory-system.md](docs/concepts/02-memory-system.md).

## Security

The shipped install seed (`settings.shared.json`) carries a conservative permissions default. It drops bare `Bash` and the keychain `security` commands from auto-allow, and moves twelve interpreters (`node`, `python3`, `npx`, `npm`, `make`, `awk`, `go`, `source`, `xargs`, `sqlite3`, `psql`, `docker`) from allow to ask, so the installer gets prompted. This closes the obvious `node -e` and `python3 -c` one-liners.

It is not a sandbox. Some commands still run without a prompt: `git`, `gh`, `find -exec`, the `sed` e-command, and anything under `Bash($HOME/.claude/**)`. The split lowers the default prompt surface, nothing more. Autoupdates ship disabled through `DISABLE_AUTOUPDATER` in the env block; remove it or set it to `0` to turn them back on.

The `deny` block's `Read(**/.env)` and `Read(**/.env.*)` rules apply to the `Read` tool only. They do not stop an agent from reading a `.env` file's content through the auto-allowed shell readers: `cat`, `grep`, `head`, `tail`, and `sed` are all bare-allowed `Bash(...)` entries, so `cat .env` runs without a prompt even though `Read(.env)` would be denied. Treat the deny rule as a guard against the `Read` tool specifically, not as protection for `.env` content in general.

To report a vulnerability, see [SECURITY.md](SECURITY.md); that page covers disclosure, not this permissions default.

## License

Apache License 2.0. See `LICENSE`.
