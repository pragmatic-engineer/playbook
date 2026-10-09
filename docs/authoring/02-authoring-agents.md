# Authoring Agents

Agents are the fourth extension point alongside commands, skills, and hooks. An agent is a pinned system prompt plus a fixed model tier and a fixed tool set, auto-discovered from `agents/`, no registry to update elsewhere. This page is the sibling to [Commands, skills, and hooks](01-commands-skills-hooks.md); read that first if you have not, it covers the other three extension points.

## What an agent is, and when to add one

Drop a file at `agents/<name>.md` and it registers itself: no plugin manifest entry, no separate index to update.

Add one when the role needs a structural property an inline prompt cannot express: a restricted tool set, a pinned model tier, or behaviour that must stay identical across every run. Do not add one for a prompt you could inline in a command or a skill instead; a command's body or a skill's `SKILL.md` already gives you that, without the extra file.

## Frontmatter schema

Every agent file opens with five keys:

| Key | Allowed values |
|---|---|
| `name` | Kebab-case, must match the filename (`agents/reviewer.md` declares `name: reviewer`). |
| `description` | When the orchestrator should spawn it, what it returns, and a closing "Not for general-purpose work." line that keeps it out of the generic picker. |
| `tools` | Comma-separated allowlist, the smallest set the role needs. |
| `model` | One of `haiku`, `sonnet`, `opus`. |
| `effort` | One of `low`, `medium`, `high`, `xhigh`, `max`. |

`agents/auditor.md`, `agents/git.md`, and `agents/reviewer.md` are real, filled-in examples of this shape.

## The two binding mechanisms

A command binds to an agent one of two ways. Both are real and in use.

**Whole-command fork.** The command's own frontmatter carries `context: fork` and `agent: <name>`, so the entire command body runs inside that agent. Its final message is the only thing the main conversation sees. Pick this when the whole command is the agent's job.

```yaml
context: fork
agent: auditor
```

That is what `commands/repo-audit.md` adds to its frontmatter to route to `auditor`. `commands/commit-and-push.md` and `commands/create-pull-request.md` both add the same two lines, pointing at `agent: git`.

**Inline spawn.** The command keeps `Agent` in its `allowed-tools` and spawns the agent directly with `subagent_type: <name>`, often several at once. Pick this when the command orchestrates and the agent does one scoped piece of the work, or when you need a swarm.

```
subagent_type: playbook:reviewer
```

`commands/quick-review.md` spawns one `reviewer` for the whole diff in a single pass. `commands/deep-review.md` spawns several `reviewer` subagents in parallel, one per review lens.

## Model and tool policy

The tiers follow the session-wide policy in [Model routing and memory](../internals/02-model-routing-and-memory.md): `haiku` is the default for spawned subagents doing mechanical, formatting, or search work, it's three times cheaper. Escalate to `sonnet` when the agent does real reasoning or implementation, and to `opus` for PR review or deep architectural judgment, kept under 20 percent of total usage.

The agents on disk show the range: `git` runs on `haiku` for mechanical staging and push work, `reviewer` runs on `opus` for PR review, `auditor` runs on `opus` at effort `high` for full-repo audits.

Grant the smallest `tools` allowlist the role needs, and say so in the `description`. There are two read-only tiers, and the wording you pick decides which one the lint applies:

- **`Structurally read-only`**: no `Edit`, no `Write`, no `NotebookEdit`, and no `Bash`. The agent reads and greps, nothing else. `reviewer` is the example.
- **`read-only`**: no `Edit`, `Write`, or `NotebookEdit`, but `Bash` is allowed for non-mutating shell like `git log` or `find`. `auditor` is the example: it audits a whole repo, so it needs shell, and it still never writes.

Pick the strict wording whenever the role genuinely needs no shell. `playbook agents check` reads the tier off the `description` and checks the `tools` list against it, so the claim and the allowlist cannot drift apart.

A **write-capable** agent claims neither tier and holds `Edit`, `Write`, and `Bash` because writing code is its job. `implementer` is the example: `/playbook:implement` spawns it for the RED, GREEN, and REFACTOR steps and the `--no-tdd` and `--no-tests` paths. The lint does not forbid write tools, it only forbids them when a read-only claim contradicts them, so a write-capable agent passes as long as its description does not say read-only.

## The guardrail template

Copy `docs/authoring/agent-template.md.tmpl` into `agents/<name>.md` rather than copying an existing agent. The template lives outside `agents/` on purpose so it never registers as a live agent itself, and copying a real one risks carrying over guardrails or scope that don't fit the new role.

Every agent carries these invariants from the template:

- The `## Non-negotiable guardrails` heading itself, so the section is easy to find and to lint for.
- The grounding rule: read a file before citing it, quote exact code with `file:line`, tag anything unverified.
- The output contract: the final message must be the exact deliverable the caller asked for, nothing wrapped around it.
- The no-dash rule: no em dashes or en dashes anywhere the agent writes.

The template also carries a read-only invariant and a zero-attribution invariant. Keep the read-only one only if the agent's `tools` list is actually read-only, and drop it otherwise.

## Parametrize or split

[ADR 0003: Purpose-built subagents over generic fallbacks](../adr/0003-purpose-built-subagents.md) sets the rule: parametrize one agent with a focus parameter when the variants are near-identical, split into separate agents when the discipline genuinely diverges.

`reviewer` is the real parametrized example on disk: it takes a lens (`logic`, `test`, `security`, `data`, `types`, `perf`, or a conditional lens) and the same agent covers every review site across `quick-review` and `deep-review`.

ADR 0003 works the same rule through a second example. A `critic` takes a focus parameter (`premise`, `plan`, `decision`, `pre-exec`) and covers several commands, because the role is one adversarial pass and the focus just flips the stance. A `fact-checker` and a `test-reviewer` split apart instead, because one follows the `playbook:grounding-research` discipline and the other follows `playbook:engineering-standards`, checklists too different to share one file. Same rule, opposite answers, because the question is whether the discipline diverges, not whether the wording does.

The trade-off in one line each: parametrizing keeps behaviour consistent and the file count low. Splitting keeps each checklist honest, at the cost of another file to maintain.

## Effort tiers and generated variants

Effort is fixed per agent file because the `Agent` tool takes no per-call effort. When an orchestrator spawns one role at different difficulty (a one-line docs diff and a 200 KB security diff), keep one base agent and let `playbook agents gen` write variants named `<base>-<tier>`, such as `reviewer-low` and `reviewer-xhigh`.

- The base file is the single source. A variant differs only in `name`, a `Low-effort variant of reviewer.` style prefix on the `description`, and `effort`. A generated comment sits right after the frontmatter.
- The set lives in the `VARIANTS` table in `src/agents/variants.rs`. To add a tier or an agent, edit the table, run `playbook agents gen`, and commit the generated files. Never edit a variant: change the base and rerun.
- `gen` is idempotent and deterministic. `playbook agents check`, and so `playbook ci`, fails when a variant is missing, stale, hand-edited, or left behind after its table entry was removed. Variants still go through every normal check, including the read-only tool rules.
- Spawn a variant with its `playbook:` prefix, for example `playbook:reviewer-xhigh`. The roster in `skills/delegating-subagents/SKILL.md` lists them, and its "Pick the tier" section says when to use each.
- Variants ship in the plugin because the archive includes all of `agents/`.

Current set: `reviewer`, `critic`, `implementer`, and `analyst` get `-low`, `-medium` and `-xhigh`. `fact-checker` gets `-medium` and `-xhigh`, and no `-low`, because a verifier that misses a wrong claim defeats its purpose. There is no `-high` variant: every base agent that has variants ships at `high`, so the base file is the high one. The `-medium` files exist so a per-component ceiling of `medium` can be met (see [ADR-0017](../adr/0017-per-component-effort-ceilings.md)). `test-reviewer` gets none: it runs once per quality gate, so nothing varies. The policy for base efforts is in [Model routing and memory](../internals/02-model-routing-and-memory.md#effort-policy).

## The check

`playbook agents check` validates every `agents/*.md` and runs as part of `rust-ci` (`.github/workflows/rust-ci.yml`), which builds the binary the check now lives in. The template lives under `docs/authoring/`, so it isn't scanned at all. The validator still skips a file named `_TEMPLATE.md` inside `agents/`, which keeps a stray copy from failing the lint.

It enforces, per file:

- Real frontmatter: an opening and a closing `---`.
- The five required keys: `name`, `description`, `tools`, `model`, `effort`.
- `name` matches the filename.
- `model` is one of `haiku`, `sonnet`, `opus`.
- `effort` is one of `low`, `medium`, `high`, `xhigh`, `max`.
- Every entry in `tools` is a known tool name, regardless of read-only tier.
- If `description` says `structurally read-only`, `tools` holds none of `Edit`, `Write`, `NotebookEdit`, `Bash`. If it says plain `read-only`, `tools` holds none of `Edit`, `Write`, `NotebookEdit`, and `Bash` is allowed.
- A `## Non-negotiable guardrails` heading is present. Under that heading only, a no-dash clause, a grounding clause, and a zero AI attribution clause are each present somewhere.

Run it before committing a new or edited agent:

```bash
playbook agents check
```

It also compares every generated variant with what `playbook agents gen` would write. It reports every offending file at once rather than stopping at the first one.

## See also

- [Commands, skills, and hooks](01-commands-skills-hooks.md): the other three extension points, commands, skills, and hooks.
- [ADR 0003: Purpose-built subagents over generic fallbacks](../adr/0003-purpose-built-subagents.md): the decision this page documents.
- [Model routing and memory](../internals/02-model-routing-and-memory.md): the model tier policy in full.
- [Docs index](../index.md)
