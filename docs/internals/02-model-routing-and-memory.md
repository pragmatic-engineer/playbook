# Model routing and memory

The session model defaults to Sonnet. Four things shape which model and effort a task gets: the tier table, the policy in each file's frontmatter, `playbook route`, and a hook that nudges design prompts toward Opus. Memory is a typed graph in plain markdown files.

## Model routing

The policy lives in `prompts/SYSTEM_PROMPT.md` and in each file's frontmatter.

- **Sonnet** is the session default and covers most coding: `/playbook:implement`, `/playbook:fix`, the implementer and the critic.
- **Haiku** takes mechanical, formatting and search work: `git`, `patch-applier`, `collector`, `cheap-checker`, `review-triage`, `fact-checker`, `test-reviewer`, `analyst`, and the `doctor`, `session-start` and `setup` commands.
- **Opus** takes design (`/playbook:plan`, `/playbook:adr`), every reviewer (`/playbook:quick-review` at medium effort, `/playbook:deep-review` and `/playbook:implement` Step 9 at high), the auditor behind `/playbook:repo-audit`, `/playbook:address-pr-comments` and `/playbook:learn-project`.

Design work uses `/playbook:plan` and `/playbook:adr`, not the built-in plan mode.

### The routing table

`playbook route <kind>` is one table for every command that dispatches agents (`src/routing.rs`). It says which model, effort and agent a kind of task goes to, and whether you must approve first.

| Kind | Model and effort | Agent |
|---|---|---|
| `mechanical` | haiku, low | `patch-applier` |
| `classify` | haiku, medium | `review-triage` |
| `check` | haiku, medium | `cheap-checker` |
| `review` | opus, high | `reviewer` |
| `implement` (`--tier low\|medium\|high`) | sonnet, low, medium or high | `implementer` |
| `design` | opus, xhigh | `critic` |

Approval is needed for the top model, the high implement tier, and a task that has failed twice (`--failures 2`). `routing.escalate` decides what happens then: `ask` (default) tells the orchestrator to ask you, `auto` proceeds (auto mode and `auto.budgetUsd` still cap the spend), `deny` returns the next cheaper route. Your effort ceiling always wins. `--json` prints the decision for scripts.

### auto-model-detect

The `auto-model-detect` hook (invoked as `playbook hook auto-model-detect`, `src/hooks/auto_model_detect.rs`) runs on every `UserPromptSubmit` event, wired in `settings.json` under the `UserPromptSubmit` hook list. It can't flip the session model mid-stream (Claude Code doesn't support that). Instead it detects design intent and injects a context message nudging Claude toward an Opus subagent.

The hook skips slash commands and prompts under 20 characters. For natural-prose prompts it matches a hand-expanded set of phrases (case-insensitive, word-boundary matched, ported from the original regex) covering:

- Design nouns: `design`, `architecture`, `ADR`, `schema`, `tradeoffs`, `migration`, `data model`, `interface design`, and related terms.
- Decision verbs: `evaluate`, `compare`, `brainstorm`, `propose`, `critique`, `review the approach`.
- Design-shaped questions: `should we`, `how would/should we/you/I`, `what's the best`, `which approach/design/pattern`, `pros and cons`.

On a match, the hook emits a prompt context message reminding Claude the session runs on Sonnet and recommending one of two delegation paths: the `Plan` agent (via `Agent` tool with `model: "opus"`) for implementation planning with codebase grounding, or `/playbook:plan` for design work before any code. For narrow prompts (a quick choice between two named options) staying inline on Sonnet is fine. No match: the hook exits silently.

### Effort policy

The reasoning is in [Why the pieces are shaped this way](../concepts/03-why-the-pieces-are-shaped-this-way.md).

Effort is a second dial next to the model tier. Agents, commands and skills accept an `effort` key (`low`, `medium`, `high`, `xhigh`, `max`). The `Agent` tool has no per-call effort, only `model`, so an agent's effort is fixed by its file. Where one role needs two efforts, the launcher renders tier variants for the session (see [Authoring agents](../authoring/02-authoring-agents.md)) and the orchestrator picks by name. To compare settings on real inputs, run `playbook eval bench`.

The rule: lower effort where the work is mechanical or already decided, and never where a missed finding is costly. A wrong model costs a rerun. A reviewer that stops looking costs a bug in production.

| File | Model | Effort | Reasoning |
| --- | --- | --- | --- |
| `agents/git` | haiku | `xhigh` (was low) | Drafts the commit message and PR title and body, and runs fixed git and `gh` steps. The Haiku evaluation found the conventional commit type was right 42% of the time at `low` and 100% at `xhigh`; at about $0.0006 a call the extra effort costs less than a cent. This is the one Haiku role allowed above `medium` (see the effort cap below), and `playbook commit run` also refuses a type that contradicts the files. |
| `agents/patch-applier` | haiku | `low` (was medium) | Applies a diff someone else approved, verbatim, with no judgment. |
| `agents/collector` | haiku | `low` (was medium) | Gathers and compacts raw history; the analyst does the thinking later. |
| `agents/cheap-checker` | haiku | `low` (was medium) | One narrow concern from a named reference file. A full lens covers the rest. |
| `agents/review-triage` | haiku | `low` (was medium) | A three-way classifier; any bad or missing answer already falls back to `full-lens`, so a wrong call fails safe. |
| `commands/quick-review` | sonnet (orchestrator); its reviewer runs on opus | `medium` for the command and for its reviewer (the reviewer's base is `high`) | One pass over a diff the user chose not to deep review; `/playbook:deep-review` is the thorough path. The reviewer is spawned as `reviewer-medium` through `playbook effort resolve agents reviewer --cap medium`, and falls back to the base reviewer in a session without variants. The Opus medium A/B (issue #660) is now done: see the `agents/reviewer` row. |
| `agents/reviewer` | opus | `high` (kept) | A missed finding is the cost. This includes the security lens, so nothing here is lowered; the `-low` variants exist only for small diffs and are an orchestrator choice. Benched on 10 real diffs (7 with a seeded real finding, 3 clean), 20 runs per cell (200 calls): Opus high 200/200, Opus medium 199/200 (the one miss was `rd-lower`, a seeded finding, 139/140 recall), Sonnet medium and high 200/200, Opus low 49/50 in a 5 run round. Cost per call: Opus high $0.0155 to $0.0168, Opus medium $0.0125 to $0.0136 (19% less), Sonnet high $0.0066 to $0.0072, Sonnet medium $0.0050 to $0.0054. Opus medium is within 1 point of high, which backs `quick-review`'s `reviewer-medium`, but the 7 seeded findings are a small set of distinct bugs, so 140 runs do not prove recall on harder ones, and a missed review finding is the costly error. Sonnet matched Opus on this set at about 60% less, and moving the reviewer to Sonnet would also change the tier rule above; both stay unapplied until a larger set of seeded diffs exists. |
| `agents/critic` | sonnet | `medium` (was high) | Round 2 bench (2026-10-10, 90 calls per cell on 6 plans, 4 with a seeded flaw and 2 clean): Sonnet medium 89/90 (98.9%) at $0.0088, Sonnet low 89/90 at $0.0078, Sonnet high 71/90 (78.9%) at $0.0145. Every miss at `high` was a false blocking finding on a clean plan; the only missed flaw in 180 medium and low calls was one `cr-destructive` run. Medium is 39% cheaper and no worse on seeded flaws. Haiku scored 67 to 73% on the same cases (false blocks on clean plans), so the critic stays on Sonnet. |
| `agents/fact-checker` | haiku | `medium` (was high) | The `high` in the file was already cut to `medium` by the Haiku effort cap, so the file now says what runs. Measured with `playbook eval bench` on real playbook excerpts over three rounds (2026-10-09 and 2026-10-10, 6 cases): Haiku high 137/138 (99.3%) at $0.00027 a call, Haiku medium 134/138 (97.1%) at $0.00021, Haiku low 86/90 (95.6%, four misses on one case) at $0.00015, Sonnet high 78/78. Medium is within 3 points of high, so it is the setting. |
| `agents/test-reviewer` | haiku | `low` (kept) | Pilot, same benchmark (18 calls per cell, then 90 per Haiku cell in round 2 on real tests with weakened assertions and clean controls): Haiku low 86/90 (95.6%), medium 86/90, high 90/90, Sonnet high 48/48, Sonnet low 42/48. Every Haiku low miss was a false alarm on a clean control, not a missed weak test, so low stays; it is the lowest setting there is. |
| `agents/analyst` | haiku (was sonnet) | `medium` (was high) | Round 2 bench (2026-10-10, 3 cluster cases checked for well formed facts and for anchors drawn only from the allowed set): Haiku low 45/45 at $0.00035, Haiku medium 45/45 at $0.00049, Haiku high 44/45 at $0.00072, Sonnet medium and high 15/15 at $0.005 and $0.007. About 14 times cheaper than Sonnet high at the same pass rate. The bench checks structure and grounding, not how useful a fact is; the `learn-project` Phase 3 confirmation by the user is still the quality gate, and the analyst never writes to the store. Medium rather than low keeps one step of margin on a bench of only three cases. |

To undo the Haiku moves, set `model: sonnet` and `effort: high` in the agent file and re-run `playbook eval bench --role <role> --model haiku,sonnet --effort medium,high` to compare. Tier overrides (`models.haiku`) stay inside one tier, so no config key moves a single agent to Sonnet.

The `implementer` stays on Sonnet and moves from `high` to `medium`. Round 1 (2026-10-10, 8 cases, 3 runs) and round 2 (20 repo tasks in `tests/fixtures/bench/implementer-repo-tasks.json` and `-2.json`: a stub to fill, a refactor, multi file changes, a bug whose root is in a helper, existing patterns to follow, a tempting hardcode, a test that must not be edited, scope and input mutation traps) apply each reply in a temp dir, run visible and hidden tests, and fail any edit to a test file or to a file outside the allowed list, with no judge model. Round 2 with 5 runs per case (100 calls per cell): Haiku low 89%, medium 88%, high 91% at $0.0003 to $0.0004 a call; Sonnet low 99%, medium 96%, high 95% at $0.0035 to $0.0037. With one fixture bug fixed (a hidden input the spec never asked for) and 10 runs per case (200 calls per cell): Sonnet medium 200/200 at $0.0027, Sonnet low 199/200 at $0.0027, Sonnet high 198/200 at $0.0029. Haiku never reaches 95% on 20 tasks (it drops the file sections or misses a multi file thread), so it stays out. On Sonnet, effort barely changes the cost of a single reply, so the saving here is about 7%, and `medium` is a no-loss setting rather than a big saving. The bench is a single turn with no tools, so it does not measure reading a real repo, running the verify command or committing, which is why `low` is not taken even though it scored the same.
| `agents/implementer` | sonnet | `medium` (was high) | See the paragraph above. |
| `agents/auditor` | opus | `high` (kept) | A whole repository audit whose findings people act on; no headless bench exists for it. |
| `commands/deep-review`, `implement`, `plan`, `adr`, `learn-project`, `fix`, `address-pr-comments` | opus, except implement and fix (sonnet) | `high` (kept) | Judgment and orchestration; mistakes propagate to every spawned agent. |
| `commands/commit-and-push`, `create-pull-request`, `repo-audit` | fork into git (haiku) and auditor (opus) | none | They fork into `git` (`low`) and `auditor` (`high`), which set the effort. |
| `commands/doctor`, `session-start`, `setup` | haiku (was sonnet) | `low` (kept) | Run compiled helpers and print the result, or load a saved handoff. Mechanical, so the cheapest tier fits (about 95% less per call, 20x cheaper per token). Checked live (see PR). |
| per-session variants such as `reviewer-low` | same as base | any tier | Not files. Rendered from the base agent by `ccc` and `ccd` into a throwaway session plugin (`--plugin-dir`), spawned as `playbook-variants:<name>`. The orchestrator picks by ceiling, then diff size and risk; see `playbook:delegating-subagents`. |
| `skills/*` | none | none | A skill is knowledge loaded into whoever uses it. An `effort` key would override the caller's choice, so none sets one. |

### Policy table: every component

This table lists every agent, command and skill with the model and effort in its file. The test `tests/policy_table.rs` reads the frontmatter of `agents/*.md`, `commands/*.md` and `skills/*/SKILL.md` and fails when this table is missing a component, lists one that does not exist, or shows a model or effort that differs from the file. A dash means the file sets none. Change a file and this table in the same PR. To compare a setting on real inputs, run `playbook eval bench` (see [Effort cap per model](#effort-cap-per-model)).

<!-- policy-table:start -->
| Kind | Name | Model | Effort | Reason |
| :- | :- | :- | :- | :- |
| agent | `analyst` | `haiku` | `medium` | Distills collector findings into memory facts. Round 2 bench: Haiku 45/45 at medium against Sonnet 15/15, at about a fourteenth of the cost; the user confirms facts in learn-project Phase 3. |
| agent | `auditor` | `opus` | `high` | A read-only repo audit where a missed finding is the cost. |
| agent | `cheap-checker` | `haiku` | `low` | One narrow concern from a named reference file. A full lens covers the rest. |
| agent | `collector` | `haiku` | `low` | Gathers and compacts raw history. The analyst does the thinking. |
| agent | `critic` | `sonnet` | `medium` | Adversarial review of plans and ADRs. Round 2 bench: medium 89/90, high 71/90 (false blocks on clean plans), so medium finds the seeded flaws at 39% lower cost. |
| agent | `fact-checker` | `haiku` | `medium` | Three bench rounds: Haiku medium 134/138, high 137/138, at about a tenth of Sonnet's cost. The Haiku effort cap already held it at medium. |
| agent | `git` | `haiku` | `xhigh` | Drafts commit and PR text. The commit type was right 42% of the time at low and 100% at xhigh. The one Haiku role allowed above medium (OPT_INS). |
| agent | `implementer` | `sonnet` | `medium` | Writes and commits production logic. Round 2 bench on 20 repo tasks: Sonnet medium 200/200, high 198/200; Haiku stays under 92%. |
| agent | `patch-applier` | `haiku` | `low` | Applies an approved diff verbatim, with no judgment. |
| agent | `review-triage` | `haiku` | `low` | A three-way classifier. A bad or missing answer falls back to a full lens. |
| agent | `reviewer` | `opus` | `high` | Review is where a missed finding costs most. quick-review runs it at medium through the variant mechanism (--cap medium). |
| agent | `test-reviewer` | `haiku` | `low` | Pilot (18 calls per cell): Haiku low, medium and high 18/18, Sonnet high 18/18, Sonnet low 16/18. |
| command | `address-pr-comments` | `opus` | `high` | Judgment on review threads and replies that go public. |
| command | `adr` | `opus` | `high` | A hard-to-reverse decision record. |
| command | `commit-and-push` | - | - | Forks into the git agent, which sets the model and effort. |
| command | `create-pull-request` | - | - | Forks into the git agent, which sets the model and effort. |
| command | `deep-review` | `opus` | `high` | Orchestrates the reviewer swarm. A missed finding is the cost. |
| command | `doctor` | `haiku` | `low` | Runs compiled helpers and prints the result. Mechanical. |
| command | `fix` | `sonnet` | `high` | Small, well understood bugs, with code changes. |
| command | `implement` | `sonnet` | `high` | Executes an approved plan and delegates edits. |
| command | `learn-project` | `opus` | `high` | Builds durable memory from a whole repo. Mistakes persist. |
| command | `plan` | `opus` | `high` | Design work. A weak plan is expensive. |
| command | `quick-review` | `sonnet` | `medium` | One pass over a diff. Its reviewer runs on Opus at medium. |
| command | `repo-audit` | - | - | Forks into the auditor agent, which sets the model and effort. |
| command | `session-start` | `haiku` | `low` | Loads a saved handoff. Mechanical. |
| command | `setup` | `haiku` | `low` | Runs compiled helpers and asks questions. Mechanical. |
| skill | `atlassian-cli` | - | - | Knowledge loaded into the caller. A skill never sets a model or effort, so the caller decides. |
| skill | `delegating-subagents` | - | - | Knowledge loaded into the caller. A skill never sets a model or effort, so the caller decides. |
| skill | `engineering-standards-javascript` | - | - | Knowledge loaded into the caller. A skill never sets a model or effort, so the caller decides. |
| skill | `engineering-standards` | - | - | Knowledge loaded into the caller. A skill never sets a model or effort, so the caller decides. |
| skill | `finish-pull-request` | - | - | Knowledge loaded into the caller. A skill never sets a model or effort, so the caller decides. |
| skill | `grounding-research` | - | - | Knowledge loaded into the caller. A skill never sets a model or effort, so the caller decides. |
| skill | `grounding-review` | - | - | Knowledge loaded into the caller. A skill never sets a model or effort, so the caller decides. |
| skill | `playbook-usage` | - | - | Knowledge loaded into the caller. A skill never sets a model or effort, so the caller decides. |
| skill | `session-handoff` | - | - | Knowledge loaded into the caller. A skill never sets a model or effort, so the caller decides. |
| skill | `systematic-debugging` | - | - | Knowledge loaded into the caller. A skill never sets a model or effort, so the caller decides. |
| skill | `writing-style` | - | - | Knowledge loaded into the caller. A skill never sets a model or effort, so the caller decides. |
<!-- policy-table:end -->

### Effort cap per model

The Haiku 5.5 evaluation (#576) priced effort on Haiku: `xhigh` costs 2x `medium` for the same pass rate, and `max` costs 13x (about 7,100 thinking tokens and 32 s per call) and scores worst (91% against 97% at `medium`). So `src/effort/model_cap.rs` puts a ceiling on every component whose frontmatter model is Haiku:

- `max` is never used on Haiku, whatever a role or a variant asks for.
- Haiku roles stop at `medium` by default.
- A role may go up to `xhigh` only when it has an entry in `OPT_INS` with a recorded reason. Today that is the `git` agent: its conventional commit type was right 42% of the time at `low` and 100% at `xhigh`, so the `git` agent itself ships at `xhigh`. Because the fork skills `commit-and-push` and `create-pull-request` name `agent: git`, the base file is the simple place for it, and a cheaper `git-low` variant stays available for a session that wants it.

The cap is a ceiling like the others. The lowest ceiling wins, your `maxEffortLevel` still beats it when lower, and it never raises anything. It applies in `playbook effort resolve`, in the variants `ccc` and `ccd` render, and it shows in `playbook effort list` and `playbook doctor models`.

Effort is fixed per agent definition: base agents run at their own effort, and an orchestrator picks a variant by ceiling, diff size and risk. Do not request xhigh or max on your own for anything else; if a deployment rejects it, use the base agent.

### Per-component ceilings

A ceiling for one component is the key `effort.<agents|commands|skills>.<name>` (see [ADR-0017](../adr/0017-per-component-effort-ceilings.md)). `playbook effort list` prints every component with its shipped, configured and effective effort. `playbook effort resolve agents <name> --json` returns, for an agent, the file to dispatch under the ceilings and every file allowed at or below them. The `Agent` tool has no per-call effort, so choosing the file is how an agent's ceiling is enforced.

### Model tiers and the 5.5 rule

Every model playbook picks is the 5.5 generation. Plugin files name a tier by alias (`haiku`, `sonnet`, `opus`) and never pin a model id. Claude Code resolves each alias to the newest model the provider offers (on the Anthropic API that is the 5.5 model).

| Tier | Alias | Preferred (5.5) | Falls back to |
|---|---|---|---|
| fast | `haiku` | `claude-haiku-5-5` | `claude-haiku-4-5` |
| balanced | `sonnet` | `claude-sonnet-5-5` | `claude-sonnet-5` |
| deep | `opus` | `claude-opus-5-5` | `claude-opus-5` |

The table lives in `src/models/mod.rs`. When the preferred model is overloaded or unavailable (including a Bedrock or Vertex account that cannot invoke it), Claude Code switches to a fallback for that turn. `ccc` and `ccd` pass the chain `claude-opus-5,claude-sonnet-5,claude-haiku-4-5` with `--fallback-model`. They add nothing when you pass `--fallback-model` yourself or set `fallbackModel` in `~/.claude/settings.json`. A session started without the launcher gets no playbook fallback.

Measured on Claude Code 2.1.293 (2026-10-09): all six ids answer, and the aliases resolve to the 5.5 ids. With an unknown primary id and `--fallback-model`, Claude Code runs the fallback. Without it the call fails with `api_error`. Claude Code accepts a chain of more than three models without a flag error, so the three-model cap is only a documented limit.

**Effort and the fallback.** The previous Sonnet and Opus reject `xhigh` and `max` with HTTP 400 `Invalid effort level`. They accept `low`, `medium` and `high`. Haiku 4.5 accepts all four. A 400 also triggers the fallback, so a session at `xhigh` would walk the chain and end at Haiku anyway. When the effort in force is `xhigh` or `max` (from `--effort`, or `effortLevel` in your user settings, lowered to the effective ceiling), `ccc` therefore passes only `claude-haiku-4-5`. When no effort is stated, the model default applies and the full chain is passed. Playbook never changes your effort to suit a fallback.

**Overrides.** `playbook config set --global models.sonnet claude-sonnet-5` (also `models.haiku` and `models.opus`) points an alias at another model of the same tier. The value is empty (no override) or `claude-<tier>-<major>[-<minor>]` for its own tier, and anything else is refused. `ccc` and `ccd` export `ANTHROPIC_DEFAULT_<ALIAS>_MODEL` for each override, so subagents and skills that name the alias follow it. A variable you exported yourself wins. Shipped plugin files still name only aliases, so the pin check in `tests/model_pins.rs` is unchanged.

`playbook doctor models` (add `--json`) prints the table, any override, the chain `ccc` passes, whether your own fallback replaces it, and whether the effort in force trimmed the chain.

`tests/model_pins.rs` fails when an agent, command, skill, prompt, output style, workflow or `settings.shared.json` names a model id outside this table, or when an agent uses anything but a tier alias.

## Memory Protocol

Memory is a typed graph. Both scopes share the same file format.

### Scopes

| Scope | Path | Coverage | Committed to git? |
|---|---|---|---|
| Global | `~/.config/playbook/memory/` | Cross-project | No, outside any repo |
| Org | `~/.config/playbook/memory/<owner>/` | Every repo under one owner | No, outside any repo |
| Project | `~/.config/playbook/memory/<owner>/<repo>/` | One repo only | No, outside any repo |

The `<owner>/<repo>` project index is injected at session start. The global index is read on demand.

### Claude Code's memory

Playbook memory and Claude Code's auto memory stay separate. Playbook never writes Claude Code's memory or settings. `memory.source` is `both` (default) or `playbook`. With `playbook`, `ccc` and `ccd` set `CLAUDE_CODE_DISABLE_AUTO_MEMORY=1` for that session only. `playbook memory import-claude` copies Claude Code's notes for a project into playbook memory, read-only on the Claude side.

### File format

One fact per file. Filenames are kebab-case (e.g. `commits-must-be-signed.md`). Every file opens with YAML frontmatter:

```yaml
---
name: fact-name
description: one-line trigger hint for when to use this fact
type: user | feedback | project | reference
links:
  supersedes: old-fact-name
  depends_on: prerequisite-name
  relates_to: neighbor-name
  contradicts: conflicting-name
anchors:           # optional; mainly used in the project store
  - src/auth/login.py#authenticate
  - src/auth/
---
```

Edge values are bare basenames with no path and no extension. For `feedback` and `project` type facts, the body follows a fixed structure: the rule first, then a **Why:** section and a **How to apply:** section.

### Index

`memory.graph.json` is the index. Each fact becomes one node:

```json
{
  "id": "acme/api/auth-flow",
  "file": "acme/api/auth-flow.md",
  "scope": "project",
  "type": "project",
  "name": "Auth Flow",
  "description": "How tokens are issued and validated in this service"
}
```

Nodes carry names and descriptions for navigation; edge declarations live only in each fact's frontmatter, not duplicated on the node.

### Edge types

| Edge | Direction | Meaning |
|---|---|---|
| `supersedes` | new → old | The authoring fact replaces the target. Act on the chain head; treat superseded facts as historical. |
| `depends_on` | authoring → prerequisite | Load the prerequisite before acting on this fact. |
| `relates_to` | symmetric | Pull the neighbor for related context. |
| `contradicts` | symmetric | Both facts are live but conflict. Surface the conflict; don't silently choose one. |

Each edge is stored once on the authoring node. Reverse links are inferred by scanning frontmatter at load time, not stored explicitly.

Traversal depth is 1 for all edge types except `supersedes`, which is followed fully (chain head wins). A project fact that contradicts a global fact wins for that repo. Dangling basenames (the target isn't in the store) are surfaced, not dropped.

### memory.graph.json

`~/.config/playbook/memory/memory.graph.json` is the single navigable export of the full memory graph: nodes are facts and referenced code locations, edges are `links:` between facts plus `anchors:` from facts to code. It covers all scopes: global, org, and project. The `rebuild-memory-graph` PostToolUse hook rebuilds it automatically after any fact file is saved. See [Decisions and Memory](../guides/03-decisions-and-memory.md) for how to query and use it day-to-day.

## See also

- [Decisions and Memory](../guides/03-decisions-and-memory.md): using memory day-to-day and the `/playbook:learn-project` command.
- [Launcher and hooks](01-launcher-and-hooks.md): the `ccc` launcher that sets the session model.
- [Docs index](../index.md)
