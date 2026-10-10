# Playbook

An opinionated toolkit for Claude Code. It gives you slash commands, skills and subagents for planning, building, reviewing and remembering, plus a small Rust binary that runs the safety guards, hooks, launcher and status line.

It ships as a Claude Code plugin. The local layer (guards, settings, launcher, system prompt) is optional.

## Quick start

```bash
curl -fsSL https://raw.githubusercontent.com/pragmatic-engineer/playbook/main/install.sh | bash
```

On macOS or Linux you can use Homebrew instead, then run `playbook init` once:

```bash
brew install pragmatic-engineer/tap/playbook
playbook init
```

Then open a Claude Code session and run `/playbook:doctor`. It checks every layer and tells you how to fix a miss.

The installer adds the plugin, installs the `playbook` binary (checksum verified), wires the guards and hooks, merges `settings.json` and installs the status line. It asks before adding the launcher and system prompt. Pass `--yes` to accept every default.

Pin a version with `PLAYBOOK_REF=v0.22.0`. Update later with `playbook update`. Everything else about installing, including a route with no `curl | bash`, is in the [install guide](docs/guides/00-install.md).

**Plugin only.** `claude plugin install playbook@pragmatic-engineer` (after `claude plugin marketplace add pragmatic-engineer/marketplace`) gives you the skills, commands and subagents. Hooks need the binary, so the first `playbook` call installs it, and `/playbook:setup` finishes the wiring. The plugin needs Claude Code 2.1.224 or later.

## A first session

```text
/playbook:plan          turn an idea into a verified plan
/playbook:implement     build it with subagents and TDD, one commit per work unit
/playbook:create-pull-request
/playbook:quick-review  or /playbook:deep-review for a bigger change
```

For a small bug, `/playbook:fix` goes from a failing test to one pull request. For a big choice, `/playbook:adr` records the decision. The [guides](docs/guides/01-plan-and-implement.md) walk through each flow.

## The launcher

`ccc` starts Claude Code with the playbook system prompt, the right model chain and your effort ceiling. `ccd` is the same with permission prompts skipped. The name is `ccc` because `cc` is the C compiler. Install both with `playbook init --aliases`.

```bash
ccc                     # resume this directory's last session, or start fresh
ccc fresh               # new session, no history
ccc list                # recent sessions for this directory
ccc worktree <branch>   # create or enter a git worktree, then start a session there
ccc clean               # resume with /model, /effort and similar settings stripped
ccc raw [id]            # resume verbatim, no fork or cleanup
ccc prune               # prune old transcripts now
```

`ccc` and `ccd` are two small shell functions printed by `playbook shell-init`. The work happens in the binary (`playbook cc launch`), so bash and zsh behave the same. Worktrees land in `<repo-parent>/.worktrees/<repo>/<folder>`. See [worktrees](docs/internals/03-worktree.md) and [the launcher](docs/internals/01-launcher-and-hooks.md).

## Commands

| Command | What it does |
|---|---|
| `/playbook:setup` | Wires the guards, seeds `settings.json`, installs what you choose. Safe to repeat. |
| `/playbook:doctor` | Checks the seven layers and prints a hint for each miss. |
| `/playbook:plan` | Idea to a verified, parallel-safe plan for `/playbook:implement`. |
| `/playbook:implement` | Runs a plan or ADR blueprint with subagents and TDD. `--auto` opens a PR. |
| `/playbook:fix` | One small bug: failing test, smallest fix, one PR. Hands off to `plan` if it grows. |
| `/playbook:adr` | Writes a fact-checked Architecture Decision Record to `docs/adr/`. |
| `/playbook:commit-and-push` | Writes the message from the staged diff, commits signed, pushes. |
| `/playbook:create-pull-request` | Opens a PR with pre-flight checks and the team template. |
| `/playbook:quick-review` | One-pass PR review, posted as a pending GitHub review. |
| `/playbook:deep-review` | Parallel specialist reviewers, consolidated and fact-checked. |
| `/playbook:address-pr-comments` | Walks unresolved review threads, fixes or drafts replies. |
| `/playbook:learn-project` | Reads the repo, PRs and tickets, then writes distilled facts to memory. |
| `/playbook:repo-audit` | Read-only audit with severity-rated findings and a task plan. |
| `/playbook:session-handoff` | Writes a handoff note (state, decisions, next steps, open questions) and saves it for the next session. Use it before `/clear` or when you stop. |
| `/playbook:session-start` | Loads the last handoff for this directory. |

Reviews are stack aware. When a PR sits in a stack, `/playbook:quick-review` and `/playbook:deep-review` always ask whether to review the whole stack or one PR (`playbook pr stack` finds it, see [ADR-0019](docs/adr/0019-pr-stack-review.md)).

Skills load on demand from `skills/`: review and research discipline, engineering standards, writing style, subagent delegation and debugging. To add your own, see [Commands, skills and hooks](docs/authoring/01-commands-skills-hooks.md).

## Settings

Everything lives in one SQLite store, `~/.config/playbook/playbook.db`. Settings resolve from repo, then org, then global, then the default.

```bash
playbook config list                                  # every key, value and source tier
playbook config set --global autoReview.fix true      # fix confirmed review findings, then push
playbook config set --global autoMerge.enabled true   # mark ready, wait for green checks, merge
playbook config set --global pr.draft false           # open PRs ready for review
playbook config export > playbook-config.json         # back up; load with `config import`
playbook state list                                   # inspect playbook's own bookkeeping
```

Every key is in [Config keys](docs/guides/04-config-keys.md), and every command and flag is in the [CLI and config reference](docs/reference/cli-and-config.md).

## Auto mode

`playbook mode auto` lets commands take the recommended answer instead of asking, list each choice under Assumptions, and keep going. A spend cap (`auto.budgetUsd`) stops a runaway session. `playbook mode ask` turns it off, and `playbook mode status` shows where the mode came from. The default is `ask`.

Auto never force-pushes, never posts a review to GitHub, and never approves a plan design unless you pass `--auto-design`. Details and limits are in the [auto mode guide](docs/guides/05-auto-mode.md).

## Models and effort

Each command and agent names its model and effort in its own frontmatter. Spend on judgment, save on routine work.

| Tier | Model | Falls back to | Used for |
|---|---|---|---|
| Opus | `claude-opus-5-5` | `claude-opus-5` | Design, every reviewer, the repo auditor |
| Sonnet | `claude-sonnet-5-5` | `claude-sonnet-5` | Session default, implementer, critic |
| Haiku | `claude-haiku-5-5` | `claude-haiku-4-5` | Mechanical agents: git, collector, fact-checker, triage |

Plugin files name only the alias, and `ccc` passes the fallback chain. Run `playbook doctor models` to see the table, your overrides and the chain in force.

- **Effort ceiling.** `playbook effort <auto|low|medium|high|xhigh|max>` sets `maxEffortLevel`. The lower of this and Claude Code's own `maxEffortLevel` wins, and playbook never changes Claude Code's setting. Override one component with `effort.agents.<name>`, `effort.commands.<name>` or `effort.skills.<name>`.
- **Variants.** The `Agent` tool has no per-call effort, so `ccc` renders effort variants of the agents for each session (`reviewer-low`, `reviewer-xhigh`). Nothing is committed. `agents.variants` is `auto`, `all` or `off`, and `playbook agents variants` shows what a session gets.
- **Routing.** `playbook route <kind>` says which model, effort and agent a task goes to, and when you must approve first.
- **Measure.** `playbook eval bench` compares roles across models and efforts on real cases, with a cost cap.

Reasoning and the full policy table are in [Why the pieces are shaped this way](docs/concepts/03-why-the-pieces-are-shaped-this-way.md) and [Model routing](docs/internals/02-model-routing-and-memory.md).

## Memory

One markdown store at `~/.config/playbook/memory/`, global and per project, local only and never committed. Playbook never writes Claude Code's own memory or settings. `memory.source` is `both` (default) or `playbook`, and `playbook memory import-claude` copies Claude Code's notes for a project into playbook memory, read-only on the Claude side. See [the memory system](docs/concepts/02-memory-system.md).

## Layers

`/playbook:doctor` reports seven layers. A working install needs 1, 2 and 6.

| Layer | What it is |
|---|---|
| 1. Plugin content | Skills, commands and subagents. Writes nothing to `~/.claude`. |
| 2. Guards and settings | Safety guards and hooks wired into `settings.json`. |
| 3. Launchers | `ccc` and `ccd` in your shell rc file. Opt in. |
| 4. System prompt | `prompts/SYSTEM_PROMPT.md`, copied to `~/.config/playbook/prompts/`. Opt in. |
| 5. Status line | `playbook statusline`, pointed at by `statusLine.command`. |
| 6. The binary | Every hook is a bare `playbook hook <name>`, so without it the hooks do nothing. |
| 7. No dangling hooks | Flags hook commands that point at a file that no longer exists. |

## Security

The security defaults are opt-in. A plain `playbook init` does not add them. Turn them on with either of:

```bash
playbook init --security
playbook config set --global security.defaults true   # then every `playbook init` applies them
```

They are the `permissions` block and `env.DISABLE_AUTOUPDATER` of the shipped `settings.shared.json`. The permissions default is conservative. It removes bare `Bash` and keychain commands from auto-allow, and moves twelve interpreters (`node`, `python3`, `npx`, `npm`, `make`, `awk`, `go`, `source`, `xargs`, `sqlite3`, `psql`, `docker`) from allow to ask. It also adds `Read(**/.env)` deny rules and ships autoupdates disabled through `DISABLE_AUTOUPDATER`. Running it again changes nothing. It follows the usual settings merge, so a `permissions` block you customised wins.

Without the opt-in, `init` leaves the `permissions` block and `DISABLE_AUTOUPDATER` in your `settings.json` as they are. It stops adding them and never removes ones you already have. To turn the defaults off, run `playbook init --no-security` or set `playbook config set --global security.defaults false`, then delete the `permissions` entries and `DISABLE_AUTOUPDATER` from `~/.claude/settings.json` by hand.

It is not a sandbox. `git`, `gh`, `find -exec` and the `sed` e-command still run without a prompt. The `Read(**/.env)` deny rules cover the `Read` tool only, so `cat .env` still runs.

To report a vulnerability, see [SECURITY.md](SECURITY.md).

## Docs

Start at the [docs index](docs/index.md).

- **Guides:** [install](docs/guides/00-install.md), [plan and implement](docs/guides/01-plan-and-implement.md), [review and PR flow](docs/guides/02-review-and-pr-flow.md), [decisions and memory](docs/guides/03-decisions-and-memory.md), [config keys](docs/guides/04-config-keys.md), [auto mode](docs/guides/05-auto-mode.md).
- **Concepts:** [system prompt](docs/concepts/01-system-prompt.md), [memory](docs/concepts/02-memory-system.md), [design rationale](docs/concepts/03-why-the-pieces-are-shaped-this-way.md).
- **Authoring:** [commands, skills and hooks](docs/authoring/01-commands-skills-hooks.md), [agents](docs/authoring/02-authoring-agents.md).
- **Internals:** [launcher and hooks](docs/internals/01-launcher-and-hooks.md), [model routing](docs/internals/02-model-routing-and-memory.md), [worktrees](docs/internals/03-worktree.md), [usage dashboard](docs/internals/04-usage-dashboard.md), [headless mode](docs/internals/05-headless-mode.md), [release channels](docs/internals/06-release-channels.md), [migrations](docs/internals/07-migrations.md), [state store](docs/internals/08-state-store.md).

## License

Apache License 2.0. See `LICENSE`.
