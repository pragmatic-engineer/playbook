# Playbook

A pragmatic Claude Code toolkit: opinionated skills, slash commands, subagents, and safety and state hooks for planning, review, memory, and guarded editing. It ships as a Claude Code plugin so it works in any shell. An optional local setup layer wires the always-on safety guards, seeds `settings.json`, and adds shell launchers and a custom system prompt.

## Quick start (1 command)

```bash
curl -fsSL https://raw.githubusercontent.com/pragmatic-engineer/playbook/main/install.sh | bash
```

That is the primary path. It adds the marketplace, installs and enables the
plugin, **installs the `playbook` binary**, wires the safety guards and the 16
functional hooks, seeds or merges `settings.json`, and installs the status line.
It asks before adding the shell launchers and the custom system prompt; pass
`--yes` to accept every default, or `--system-prompt` to take both without
prompting.

Then open a Claude Code session and run `/playbook:doctor` to verify.

### Why not plugin-install on its own

`claude plugin install` gives you the skills, commands and subagents, and
nothing else. It does **not** install the `playbook` binary, and every ported
hook is a bare `playbook hook <name>` command, so without the binary all 16 are
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

For requirements, pinning a version, and uninstall, see
[docs/guides/00-install.md](docs/guides/00-install.md).

## Layers

Playbook has six layers, numbered to match what `/playbook:doctor` reports.
Layers 1, 2 and 6 are what a working install needs; 3, 4 and 5 are optional or
cosmetic.

| Layer | When | What it does |
|---|---|---|
| 1. Plugin content | Always, after `claude plugin install` | Skills, commands and subagents load from the plugin. No files written to `~/.claude`. The functional hooks are registered but **need Layer 6 to run**. |
| 2. Safety guards and settings | After `install.sh`, or `/playbook:setup` | Wires the guards and seeds or merges `~/.claude/settings.json`. `install.sh` wires them as `playbook hook <name>`; `/playbook:setup` still copies the legacy `~/.claude/hooks/*.sh` scripts, which `/playbook:doctor` reports as not wired. |
| 3. Shell launchers | Opt-in (recommended) | Adds `cc` and `ccd` to `~/.bashrc` or `~/.zshrc`. Both shells work; `cc clean` and `cc raw` are zsh-only (see Usage). |
| 4. Custom system prompt | Opt-in (recommended) | Copies `prompts/SYSTEM_PROMPT.md` to `~/.claude/prompts/`; `cc` passes it via `--system-prompt-file`. Plugin content works without it. |
| 5. Status line | After `install.sh` | Installs `~/.claude/statusline.sh`. `/playbook:setup` does **not** install it; `/playbook:doctor` checks it. |
| 6. The `playbook` binary | `install.sh` or `/playbook:setup` | Installs the release binary to `~/.local/bin`, checksum-verified. **Every ported hook is a bare `playbook hook <name>` command, so without this all 16 are dead.** `claude plugin install` alone does not provide it. |

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

`cc` loads the system prompt (when installed), picks a model, and prunes old transcripts (keeps the newest 5; set `CCD_KEEP` to change, `CCD_KEEP=0` disables).

**One launcher, both shells.** `shell/zsh/cc.zsh` and `shell/bash/cc.sh` are thin entry points that both source the same modules under `shell/shared/`. So bash and zsh behave identically: every subcommand above, the config-drift auto-fork on the default resume, and retention all work the same in either shell. Source the entry for your shell (`cc.zsh` from `~/.zshrc`, `cc.sh` from `~/.bashrc`); `/playbook:setup --install-aliases` wires the right one.

`cc worktree` (also `ccd worktree`) groups worktrees under `<repo-parent>/.worktrees/<repo>/<folder>` (set `WORKTREE_BASE_DIR` to change the base folder), names the folder after the JIRA key in the branch name, and copies `.env` into it. It also clones `node_modules`, pushes to set upstream, and offers AI-assisted rebase conflict resolution. The engine (`shell/shared/worktree.sh`) is shared by both shells. See [docs/internals/03-worktree.md](docs/internals/03-worktree.md) for the full behaviour.

## Commands

Slash commands live in `commands/`. See [docs/guides](docs/guides) for full usage.

| Command | What it does |
|---|---|
| `/playbook:setup` | Wires the guards, seeds `settings.json`, and installs what you choose. Safe to run repeatedly. |
| `/playbook:doctor` | Checks the four layers and prints a pass/info table with a remediation hint for each miss. |
| `/playbook:brainstorm` | Divergent discovery session; explores a raw idea and produces an approved design doc for `/playbook:scope`. |
| `/playbook:scope` | Interview-driven planning; saves a verified, parallel-safe plan to `.claude/plans/` for `/playbook:implement`. |
| `/playbook:implement` | Executes a `/playbook:scope` plan or `/playbook:adr` blueprint with subagents and TDD, committing each work unit. `--auto` opens a PR. |
| `/playbook:adr` | Creates an Architecture Decision Record through investigate, draft, quality-gate, finalise. Saves to `.claude/adr/`. |
| `/playbook:commit-and-push` | Writes a commit message from the staged diff, commits signed, optionally rebases, then pushes. |
| `/playbook:create-pull-request` | Opens a PR with pre-flight checks, a conventional-commit title, and the team PR template. |
| `/playbook:quick-review` | Single-pass PR review using the `grounding-review` discipline, posted as a pending GitHub review. |
| `/playbook:deep-review` | Multi-agent PR review; spawns specialist subagents in parallel, consolidates findings, posts a pending review. |
| `/playbook:address-pr-comments` | Walks unresolved PR comments, applies fixes or drafts replies, then pushes and posts replies. |
| `/playbook:learn-project` | Analyses the repo (git history, code, PRs, JIRA/Confluence) and writes distilled facts to memory. Read-only; confirms before writing. |
| `/playbook:repo-audit` | Read-only four-phase repository audit (discovery, findings, strategy, task plan). |

## Skills

Skills live in `skills/` and load on demand. See [docs/authoring/01-commands-skills-hooks.md](docs/authoring/01-commands-skills-hooks.md).

| Skill | What it does |
|---|---|
| `grounding-review` | Review discipline; severity levels, Conventional Comments, proof ladder, verification summary. |
| `grounding-research` | Investigation discipline; citation rules (every claim sourced to `file:line`), structured findings, scope boundaries. |
| `engineering-standards` | PR readiness, test types, mocking rules, incremental delivery, deployment flow. |
| `engineering-standards-javascript` | JS/TS companion to `engineering-standards`; covers Zod validation and Jest/Vitest mocking. |
| `writing-style` | Voice rules for human-facing prose; spartan, active voice, contractions, no dashes. |
| `session-handoff` | Decision-first handoff so the next session picks up cold without rereading the thread. |

## Docs

Full documentation: [`docs/index.md`](docs/index.md).

- **Concepts** (`docs/concepts/`): system prompt design and the memory system.
- **Guides** (`docs/guides/`): install, plan-and-implement, review and PR flow, decisions and memory.
- **Authoring** (`docs/authoring/`): writing commands, skills, and hooks.
- **Internals** (`docs/internals/`): launcher, hooks, model routing, and memory injection.

## System prompt

`prompts/SYSTEM_PROMPT.md` sets the persona and session rules `cc` loads on every session, when installed. See [docs/concepts/01-system-prompt.md](docs/concepts/01-system-prompt.md).

## Memory

One markdown store at `~/.claude/memory/`, global and per-project, local-only and never committed. See [docs/concepts/02-memory-system.md](docs/concepts/02-memory-system.md).

## Security

The shipped install seed (`settings.shared.json`) carries a conservative permissions default. It drops bare `Bash` and the keychain `security` commands from auto-allow, and moves twelve interpreters (`node`, `python3`, `npx`, `npm`, `make`, `awk`, `go`, `source`, `xargs`, `sqlite3`, `psql`, `docker`) from allow to ask, so the installer gets prompted. This closes the obvious `node -e` and `python3 -c` one-liners.

It is not a sandbox. Some commands still run without a prompt: `git`, `gh`, `find -exec`, the `sed` e-command, and anything under `Bash(**/.claude/**)`. The split lowers the default prompt surface, nothing more. Autoupdates ship disabled through `DISABLE_AUTOUPDATER` in the env block; remove it or set it to `0` to turn them back on.

To report a vulnerability, see [SECURITY.md](SECURITY.md); that page covers disclosure, not this permissions default.

## License

MIT. See `LICENSE`.
