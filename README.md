# Playbook

An opinionated toolkit for Claude Code. It gives you slash commands, skills and subagents for planning, building, reviewing and remembering, plus a small Rust binary that runs the safety guards, hooks, launcher and status line.

The skills ship as a Claude Code plugin and work on their own. The binary, which adds the guards, hooks, status line and launcher, is optional.

## Quick start

Skills, the part you can use alone:

```bash
claude plugin marketplace add pragmatic-engineer/skills
claude plugin install playbook@pragmatic-engineer
```

The binary, optional. The install script installs it and then asks whether to run `playbook init`:

```bash
curl -fsSL https://raw.githubusercontent.com/pragmatic-engineer/playbook/main/install.sh | bash
```

On macOS or Linux you can use Homebrew instead, then run `playbook init` once:

```bash
brew install pragmatic-engineer/tap/playbook
playbook init
```

`playbook init` adds the hook entries to `~/.claude/settings.json`. It asks first, keeps your own entries, and marks the ones it owns so `playbook uninstall` removes only those. Then open a Claude Code session and run `/playbook:doctor`. It checks every layer and tells you how to fix a miss.

The installer only installs the `playbook` binary (checksum verified) and puts it on `PATH`. It never installs or removes a plugin. `playbook init` also sets the status line and asks before adding the launcher and system prompt. Pass `--yes` to the installer to say yes to every question.

To see what `playbook init` changes before you agree, read [What init installs](docs/guides/06-what-init-installs.md) and the [hook reference](docs/reference/hooks.md).

Optional: the standalone usage dashboard for machines without playbook, `brew install pragmatic-engineer/tap/playbook-telemetry`, then run `pbtop`. `pbtop` is the short alias of `playbook-telemetry`. With playbook installed you do not need it, because `playbook usage` runs the same code. The code index gets the same treatment: `pbcode` is the planned alias of the standalone `playbook-code` tool, which has no release yet, so use `playbook code` today.

Pin a version with `PLAYBOOK_REF=v0.25.0`. Update later with `playbook update`. Everything else about installing, including a route with no `curl | bash`, is in the [install guide](docs/guides/00-install.md).

| Mode | What you get | Commands |
|---|---|---|
| Skills only | Skills, commands and subagents. No guards, hooks or status line. | The two `claude plugin` commands above |
| Binary only | The `playbook` command, hooks, guards, status line and usage | The install script, or Homebrew then `playbook init` |
| Both (recommended) | Everything | Both of the above |

The hooks, guards, status line and usage need the binary. Only the skill text works without it. See [Pick an install mode](docs/guides/00-install.md#pick-an-install-mode).

## A first session

```text
/playbook:plan          turn an idea into a verified plan
/playbook:implement     build it with subagents and TDD, one commit per work unit
/playbook:create-pull-request
/playbook:quick-review  or /playbook:deep-review for a bigger change
```

For a small bug, `/playbook:fix` goes from a failing test to one pull request. For a big choice, `/playbook:adr` records the decision. The [guides](https://github.com/pragmatic-engineer/skills/blob/main/docs/guides/01-plan-and-implement.md) walk through each flow.

## The launcher

`playbook cc` starts Claude Code with the playbook system prompt, the right model chain and your effort ceiling. `playbook ccd` is the same with permission prompts skipped. `pb` is a short alias for `playbook` (`pb cc`, `pb ccd`), linked next to the binary by install.sh and Homebrew when the name is free. The old `ccc` and `ccd` shell functions are gone; `playbook doctor` finds leftovers and names the replacement. Add the cd wrapper, which lets `playbook cc launch worktree` leave you in the new worktree, with `playbook init --aliases`.

```bash
playbook cc                     # resume this directory's last session, or start fresh
playbook cc fresh               # new session, no history
playbook cc list                # recent sessions for this directory
playbook cc launch worktree <branch>   # create or enter a git worktree, then start a session there
playbook cc clean               # resume with /model, /effort and similar settings stripped
playbook cc raw [id]            # resume verbatim, no fork or cleanup
playbook cc prune               # prune old transcripts now
```

The work happens in the binary, so bash and zsh behave the same. `playbook shell-init` prints one small `playbook` shell function that only adds the `cd` into a worktree after a session. Worktrees land in `<repo-parent>/.worktrees/<repo>/<folder>`. See [worktrees](docs/internals/03-worktree.md) and [the launcher](docs/internals/01-launcher-and-hooks.md).

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

Skills load on demand from the `playbook` plugin in [pragmatic-engineer/skills](https://github.com/pragmatic-engineer/skills): review and research discipline, engineering standards, writing style, subagent delegation and debugging. To add your own, see [Commands, skills and hooks](https://github.com/pragmatic-engineer/skills/blob/main/docs/authoring/01-commands-skills-hooks.md).

## Settings

Settings live in one SQLite store, `~/.config/playbook/data/config.db`, next to every other database playbook keeps (see [the data folder](docs/internals/11-data-folder.md)). Settings resolve from repo, then org, then global, then the default.

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

`playbook mode auto` lets commands take the recommended answer instead of asking, list each choice under Assumptions, and keep going. A spend cap per agent (`auto.budgetUsd` for the main session, `auto.agentBudgetUsd` for each subagent) parks a runaway agent while it can still deliver finished work. `playbook mode ask` turns it off, and `playbook mode status` shows where the mode came from. The default is `ask`.

Auto never force-pushes and never approves a plan design unless you pass `--auto-design`. Auto does not skip the review post question: pass `--self` or `--no-post` for that. Details and limits are in the [auto mode guide](docs/guides/05-auto-mode.md).

## Models and effort

Each command and agent names its model and effort in its own frontmatter. Spend on judgment, save on routine work.

| Tier | Model | Falls back to | Used for |
|---|---|---|---|
| Opus | `claude-opus-5-5` | `claude-opus-5` | Design, every reviewer, the repo auditor |
| Sonnet | `claude-sonnet-5-5` | `claude-sonnet-5` | Session default, implementer, critic |
| Haiku | `claude-haiku-5-5` | `claude-haiku-4-5` | Mechanical agents: git, collector, fact-checker, triage |

Plugin files name only the alias, and `playbook cc` passes the fallback chain. Run `playbook doctor models` to see the table, your overrides and the chain in force.

- **Effort ceiling.** `playbook effort <auto|low|medium|high|xhigh|max>` sets `maxEffortLevel`. The lower of this and Claude Code's own `maxEffortLevel` wins, and playbook never changes Claude Code's setting. Override one component with `effort.agents.<name>`, `effort.commands.<name>` or `effort.skills.<name>`.
- **Variants.** The `Agent` tool has no per-call effort, so `playbook cc` renders effort variants of the agents for each session (`reviewer-low`, `reviewer-xhigh`). Nothing is committed. `agents.variants` is `auto`, `all` or `off`, and `playbook agents variants` shows what a session gets.
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
| 3. Shell wrapper | A `playbook` function in your shell rc file that follows `playbook cc` into a worktree. Opt in. |
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

They are the `permissions` block of the shipped `settings.shared.json`. The permissions default is conservative. It removes bare `Bash` and keychain commands from auto-allow, and moves twelve interpreters (`node`, `python3`, `npx`, `npm`, `make`, `awk`, `go`, `source`, `xargs`, `sqlite3`, `psql`, `docker`) from allow to ask. It also adds `Read(**/.env)` deny rules Running it again changes nothing. It follows the usual settings merge, so a `permissions` block you customised wins.

Without the opt-in, `init` leaves the `permissions` block in your `settings.json` as it is. It stops adding it and never removes one you already have. To turn the defaults off, run `playbook init --no-security` or set `playbook config set --global security.defaults false`, then delete the `permissions` entries from `~/.claude/settings.json` by hand.

It is not a sandbox. `git`, `gh`, `find -exec` and the `sed` e-command still run without a prompt. The `Read(**/.env)` deny rules cover the `Read` tool only, so `cat .env` still runs.

To report a vulnerability, see [SECURITY.md](SECURITY.md).

## Docs

Start at the [docs index](docs/index.md).

- **Guides:** [install](docs/guides/00-install.md), [plan and implement](https://github.com/pragmatic-engineer/skills/blob/main/docs/guides/01-plan-and-implement.md), [review and PR flow](https://github.com/pragmatic-engineer/skills/blob/main/docs/guides/02-review-and-pr-flow.md), [decisions and memory](docs/guides/03-decisions-and-memory.md), [config keys](docs/guides/04-config-keys.md), [auto mode](docs/guides/05-auto-mode.md).
- **Concepts:** [system prompt](docs/concepts/01-system-prompt.md), [memory](docs/concepts/02-memory-system.md), [design rationale](docs/concepts/03-why-the-pieces-are-shaped-this-way.md).
- **Authoring:** [commands, skills and hooks](https://github.com/pragmatic-engineer/skills/blob/main/docs/authoring/01-commands-skills-hooks.md), [agents](https://github.com/pragmatic-engineer/skills/blob/main/docs/authoring/02-authoring-agents.md).
- **Internals:** [launcher and hooks](docs/internals/01-launcher-and-hooks.md), [model routing](docs/internals/02-model-routing-and-memory.md), [worktrees](docs/internals/03-worktree.md), [usage dashboard](docs/internals/04-usage-dashboard.md), [headless mode](docs/internals/05-headless-mode.md), [release channels](docs/internals/06-release-channels.md), [migrations](docs/internals/07-migrations.md), [state store](docs/internals/08-state-store.md), [sessions](docs/internals/09-sessions.md), [document conversion benchmark](docs/internals/10-document-conversion-benchmark.md).

## Repository layout

This repository holds the `playbook` binary and `install.sh`. The skills, commands and agents live in [pragmatic-engineer/skills](https://github.com/pragmatic-engineer/skills). The Rust code is a Cargo workspace under `crates/`: `cli` (the `playbook` binary), `common`, `code`, `telemetry` (`playbook-telemetry` and `pbtop`) and `data`. See [CONTRIBUTING.md](CONTRIBUTING.md#repository-layout).

## License

Apache License 2.0. See `LICENSE`.
