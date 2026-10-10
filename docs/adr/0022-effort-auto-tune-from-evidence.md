# ADR-0022: Tune Effort From Local Evidence, in Three Stages

- **Status:** Accepted
- **Date created:** 2026-10-10
- **Date modified:** 2026-10-10

## Context

Issue #578 asks for a system that recommends, and later applies, effort choices from how a project is really used. The inputs it waited on now exist:

- The effort benchmark (#576, #662) ran in a campaign merged in #712, #714 and #716. It measured pass rate and cost for implementer, critic, analyst, fact-checker, test-reviewer and reviewer at each effort level, and moved four agents down (critic and implementer to Sonnet medium, analyst and fact-checker to Haiku medium). The reviewer moved to Opus medium in [ADR-0021](0021-reviewer-at-opus-medium.md). Everything else was kept, for the reasons in `docs/internals/02-model-routing-and-memory.md`.
- The effort caps are in place: Claude Code's `maxEffortLevel`, playbook's `maxEffortLevel`, the per-component keys `effort.<agents|commands|skills>.<name>` ([ADR-0017](0017-per-component-effort-ceilings.md)), and the per-model cap in `src/effort/model_cap.rs` (Haiku stops at `medium`, never `max`).
- The routing contract (#575) is in `src/routing.rs` and the `delegating-subagents` skill: the orchestrator picks a tier by judgement, states it, and asks first for the high tier, the top model and a third attempt.
- The usage store and its query layer ([ADR-0020](0020-usage-terminal-ui.md), `src/usage/query.rs`) hold one row per assistant message (model, effort, repo, branch, session, tokens, cost) and one row per `Skill` or `Agent` tool call (kind and name, with the variant name when a variant was dispatched).

What the store cannot do today matters for the design. A usage row does not say which agent produced it, so cost cannot be split per component, and it has no wall time per dispatch. The transcripts do carry the link (subagent lines have `isSidechain` and an `agentId`), but ingest does not read it. The benchmark is single turn with no tools for most roles, and the reviewer rests on 7 distinct seeded bugs. Any design has to work with weak evidence and say so.

## Decision

### 1. Rules and thresholds, not a learned model

The first version is a fixed set of rules with named thresholds, read from one table in the code. A learned model needs labelled outcomes the project does not have (see "Measuring a missed finding"), would be hard to explain to the person it advises, and would be hard to test. Rules can be unit tested, can be printed with the evidence behind them, and a threshold can be changed in one line. A learned model stays open for a later ADR once there is enough labelled history to beat the rules on a held out set.

### 2. Permission comes from the benchmark, the trigger comes from usage

This is the one rule that settles most conflicts. Evidence from real usage can raise a flag or veto a change. It can never grant permission to lower effort. Only the benchmark quality floor grants that.

- Lowering a component below its shipped effort requires a quality floor entry that lets it, and a floor never drops under what a benchmark measured.
- Usage data decides whether a permitted change is worth proposing (enough dispatches, meaningful cost) and whether to hold it back (reruns, overrides toward higher effort, missed finding proxies).
- Absence of bad signals is not evidence of quality. A quiet reviewer is not a good reviewer.

### 3. Signals

Each signal has a source, a stage that first uses it, and a known weakness.

| Signal | Source | First used | Weakness |
| --- | --- | --- | --- |
| Dispatch count per component and per effort variant | `tool_invocation_events` (the name `reviewer-low` or `playbook-variants:reviewer-xhigh` carries the effort) | Stage 1 | Names are as typed by the orchestrator. |
| Reruns of a component in one session | `tool_invocation_events`, grouped by session | Stage 1 | Fan out components (reviewer, implementer, test-reviewer, cheap-checker) are dispatched many times on purpose, so they are marked and excluded from rerun evidence. |
| Overrides: a variant that differs from the shipped effort | Same rows | Stage 1 | Cannot tell a user choice from an orchestrator choice or a ceiling. All three are shown as overrides. |
| Cost and tokens per model family and effort | `usage_events`, through `query::load_window` | Stage 1 | Shared with the main session, because a row has no component. Shown as a bucket, labelled as such. |
| The user's ceilings | `playbook effort` resolution (read only) | Stage 1 | None. |
| Cost, tokens and wall time per component | Needs `agentId` read at ingest and stored as a `component` column | Later change to ingest, before stage 2 | A schema migration in `src/usage`, so it gets its own ADR note or PR. |
| Outcome signals: overturned and missed findings | Gate database verdicts, the git history, optional `gh` reads | Stage 2 | See the next section. |
| Memory facts about risk (hot files, security paths, flaky areas) and ADR decisions | Memory store, read only, matched to diff paths through fact anchors | Stage 3 | Facts can be stale. A stale fact only ever raises effort. |
| Diff shape: bytes, files touched, language, risk class | The diff, at dispatch time | Stage 3 | The existing orchestrator rule (a diff over 60 KB, or auth, secrets, crypto, untrusted input, goes to `reviewer-xhigh`) is the baseline. The tuner may add to it and never removes it. |

Wall time is not used in stage 1. A session's first to last message gap says little about one agent, and a number the tool cannot defend does not belong in a report.

### 4. Guardrails

1. **Quality floor.** A table in code (`src/effort/floor.rs`) lists, per agent, the lowest effort the maintainer accepts, whether a miss is costly, and the source of the number (the benchmark run and the document that records it). A floor is the lowest level that held quality in the bench, raised by one step when the bench is thin (a single turn bench with no tools, or fewer than ten cases). A component with no bench has its shipped effort as its floor, so it is never lowered. The table is the only place that grants permission, and a test checks that every agent has a row and that no floor is above its shipped effort.
2. **Costly to miss.** The reviewer, the auditor and any component the maintainer marks never go below shipped effort, whatever the data says. A lower variant dispatched for one of them is reported as a risk and never counted as a saving.
3. **Protected work.** Diffs that touch auth, secrets, crypto, migrations, permissions, CI or hooks, or paths named by a memory fact of type risk or by an ADR, never get lower effort. The rule "never lower effort for auth code" is stored as a protected path pattern in memory or ADR text, and the tuner reads it. It does not infer it.
4. **Never above the ceiling.** The tuner works inside the lowest of Claude Code's `maxEffortLevel`, playbook's `maxEffortLevel`, the component key and the model cap. It never raises a component above its shipped effort except by choosing an existing variant that the ceiling allows, as the orchestrator already does. The #575 approvals (top model, high implement tier, a third attempt) still apply and the tuner cannot grant them.
5. **Cold start uses defaults.** Below the minimum evidence (20 dispatches of a component in the window, and 7 days of history) the tuner says "not enough data" and proposes nothing. A new install behaves exactly as today.
6. **Local only.** The tuner reads the local usage store, local config and local memory. It makes no network call of its own. Outcome signals that need `gh` are opt in and read PR data the user already has access to, and nothing is sent anywhere.
7. **One command resets.** Stage 1 writes nothing, so there is nothing to reset. From stage 2 on, every key the tuner writes is recorded in a ledger file next to the config, and `playbook effort tune reset` removes exactly those keys and restores the shipped defaults. Keys the user set by hand are never in the ledger and are never touched.
8. **Playbook code reads Claude Code settings and never writes them.** Applied changes go to playbook's own config only (`effort.agents.<name>`).

### 5. Measuring a missed finding without ground truth

There is no ground truth for a finding nobody saw. The design treats every measure as a proxy, uses them only to veto or to ask for a re-bench, and never to justify lowering.

- **Overturn by a later pass.** A review at one effort followed, on the same branch, by a review at a higher effort (for example `quick-review`, then `deep-review`, or a base reviewer then `reviewer-xhigh`) that reports findings the first did not. The count of extra findings per effort level is a lower bound on misses.
- **Late human comments.** Review comments on a PR after a self review passed (read with `gh`, opt in).
- **Fix after merge.** A commit of type `fix` that touches the same files within 14 days of the PR merging, from local git history. This is noisy (it includes unrelated fixes), so it is a rate compared across effort levels of the same component, never an absolute count.
- **False positives.** The gate database already records findings that the verification step dropped. A high drop rate at a given effort argues against raising it.
- **Benchmark as the control.** A recurring seeded bench (#576 step 5) is the only measure with ground truth. When a proxy rate rises for a component, the response is to re-run its bench with new seeded cases, not to change effort on the proxy alone.

### 6. Rollout stages

Each stage ships and is judged on its own. Stage 2 and 3 are not part of the work that accepts this ADR.

1. **Report (the first change after this ADR).** `playbook effort suggest` prints, per agent, the evidence and whether a change looks safe or needed, and `--json` prints the same for scripts. It reads the usage store, the floor table and the effort resolution. It writes nothing. It cannot lower anything it is not permitted to, and it says "no change" when that is the answer.
2. **Proposed config patch.** `playbook effort suggest --patch` shows the exact `playbook config set` commands for the user's own config, with the evidence, and applies them only on explicit approval, recording each key in the ledger. Needs per component attribution first, so that the evidence is the component's own cost.
3. **Optional automatic choice per dispatch.** A config key (default `off`) lets `playbook effort resolve` pick among the variants allowed by the ceiling, using diff shape, protected paths and the component's history. Every automatic choice is printed in the dispatch report with its one line reason, appended to a local log, and shown in `playbook effort suggest`. It is never silent, never above the ceiling, and a failure to read evidence falls back to the shipped default.

### 7. Relation to #532 and #575

- **#532 (self improving playbook)** asks what may change on its own, from safest to riskiest, and names an autonomy setting (`off`, `propose`, `auto`). Effort tuning is one concrete case of it. The stages above map to that setting (stage 1 is `off` with a report, stage 2 is `propose`, stage 3 is `auto`), and the tuner reuses its rules: every change reversible, a kill switch, nothing above the user's ceiling. If #532 adds a shared autonomy key, effort tuning reads it and does not add its own.
- **#575 (routing contract)** owns the choice of tier at dispatch and the approval gates. Stage 3 supplies an extra input to that choice and adds no second router. It plugs into `playbook effort resolve` and `src/routing.rs`, and the gates stay where they are.

## Alternatives considered

- **A learned model first.** Rejected for now (see 1). No labelled outcomes exist.
- **Letting usage evidence lower effort without a benchmark.** Rejected. Cost and rerun data say nothing about missed findings, which is the costly error (the reviewer stayed on Opus and at medium in [ADR-0021](0021-reviewer-at-opus-medium.md) because a missed finding is the costly error).
- **Tuning commands and skills too.** Rejected for stage 1. Commands have no headless bench and their cost is not separable in the store, so their floor is their shipped effort. Skills set no effort at all ([ADR-0017](0017-per-component-effort-ceilings.md)).
- **Writing the suggestion straight into settings.** Rejected. Claude Code settings are never written by playbook code, and an unreviewed change to config removes the user's chance to say no.
- **Splitting cost by component from the usage rows alone.** Rejected as unsound: a model and effort pair is shared by the main session and by every agent on that pair. Stage 1 shows the shared bucket and says so, and component attribution is an ingest change for later.

## Consequences

- Stage 1 gives a reading of dispatch counts, overrides and reruns, and a check that the user's ceilings are not below a quality floor. On the current agent set most floors equal the shipped effort (the campaign already took the available savings), so the honest output of the report is often "no change".
- The floor table has to be updated by hand when a bench changes a component. A test ties it to the agent files, and the policy table in `docs/internals/02-model-routing-and-memory.md` stays the human readable record.
- Stage 2 is blocked on per component attribution in the usage store. That change touches ingest and the schema, and is recorded here so nobody reads the shared bucket as per agent cost in the meantime.
- Reviewer recall is the thinnest evidence in the floor table (7 distinct seeded bugs). Until a larger set exists the reviewer stays costly to miss and is never proposed for lowering.
- A threshold set here (20 dispatches, 7 days, rerun share, override share) is a starting point. The code keeps them as named constants, and changing one needs a line in the PR that says which evidence moved it.
