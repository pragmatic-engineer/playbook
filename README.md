# Playbook

A pragmatic Claude Code toolkit: opinionated skills, slash commands, subagents, and safety and state hooks for planning, review, memory, and guarded editing. It ships as a Claude Code plugin so it works in any shell. An optional local setup layer wires the always-on safety guards, seeds `settings.json`, and adds shell launchers and a custom system prompt.

## Quick start (3 commands)

```bash
claude plugin marketplace add pragmatic-engineer/marketplace
claude plugin install playbook@pragmatic-engineer
```

Then open a Claude Code session and run `/setup --install-aliases --use-system-prompt`. Those flags install the shell launchers and the custom system prompt without asking; drop them and `/setup` asks two yes/no questions instead (both default to yes). Run `/doctor` afterwards to verify.

That is the primary path. The plugin content (skills, commands, agents, hooks) is available immediately after install; `/setup` adds the local layers on top. For the full local install (curl one-liner, requirements, uninstall), see [docs/guides/00-install.md](docs/guides/00-install.md).

## Layers

Playbook has four layers. Each is independent; stop at any level.

| Layer | When | What it does |
|---|---|---|
| 1. Plugin content | Always, after `claude plugin install` | Skills, commands, subagents, and functional hooks load from the plugin. No files written to `~/.claude`. |
| 2. Safety guards and settings | Always, after `/setup` | Copies the three guard hooks into `~/.claude/hooks/` and seeds or merges `~/.claude/settings.json`. Runs regardless of the other choices. |
| 3. Shell launchers | Opt-in (recommended) | Adds `cc` and `ccd` to `~/.bashrc` or `~/.zshrc`. `cc worktree` is zsh-only; other subcommands work in bash. |
| 4. Custom system prompt | Opt-in (recommended) | Copies `prompts/SYSTEM_PROMPT.md` to `~/.claude/prompts/`; `cc` passes it via `--system-prompt-file`. Plugin content works without it. |

## Usage

```bash
cc                     # resume this directory's last session, or start fresh
ccd                    # same, with --dangerously-skip-permissions
cc fresh               # new session, no history
cc list                # recent sessions for this directory
cc clean               # resume with /model, /effort, /config, /output-style, /style stripped
cc raw [id]            # resume verbatim, no fork or cleanup
cc worktree <branch>   # create/enter a git worktree, then start a session there (zsh only)
cc new <branch>        # alias for cc worktree (zsh only)
```

`cc` loads the system prompt (when installed), picks a model, and prunes old transcripts (keeps the newest 5; set `CCD_KEEP` to change, `CCD_KEEP=0` disables).

`cc worktree` (also `ccd worktree`) groups worktrees under `<repo-parent>/.worktrees/<repo>/<folder>` (set `WORKTREE_BASE_DIR` to change the base folder), names the folder after the JIRA key in the branch name, and copies `.env` into it. It also clones `node_modules`, pushes to set upstream, and offers AI-assisted rebase conflict resolution. Only available via `cc`/`ccd` on zsh. See [docs/internals/03-worktree.md](docs/internals/03-worktree.md) for the full behaviour.

## Commands

Slash commands live in `commands/`. See [docs/guides](docs/guides) for full usage.

| Command | What it does |
|---|---|
| `/setup` | Wires the guards, seeds `settings.json`, and installs what you choose. Safe to run repeatedly. |
| `/doctor` | Checks the four layers and prints a pass/info table with a remediation hint for each miss. |
| `/brainstorm` | Divergent discovery session; explores a raw idea and produces an approved design doc for `/scope`. |
| `/scope` | Interview-driven planning; saves a verified, parallel-safe plan to `.claude/plans/` for `/implement`. |
| `/implement` | Executes a `/scope` plan or `/adr` blueprint with subagents and TDD, committing each work unit. `--auto` opens a PR. |
| `/adr` | Creates an Architecture Decision Record through investigate, draft, quality-gate, finalise. Saves to `.claude/adr/`. |
| `/commit-and-push` | Writes a commit message from the staged diff, commits signed, optionally rebases, then pushes. |
| `/create-pull-request` | Opens a PR with pre-flight checks, a conventional-commit title, and the team PR template. |
| `/quick-review` | Single-pass PR review using the `grounding-review` discipline, posted as a pending GitHub review. |
| `/deep-review` | Multi-agent PR review; spawns specialist subagents in parallel, consolidates findings, posts a pending review. |
| `/address-pr-comments` | Walks unresolved PR comments, applies fixes or drafts replies, then pushes and posts replies. |
| `/learn-project` | Analyses the repo (git history, code, PRs, JIRA/Confluence) and writes distilled facts to memory. Read-only; confirms before writing. |
| `/repo-audit` | Read-only four-phase repository audit (discovery, findings, strategy, task plan). |

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
