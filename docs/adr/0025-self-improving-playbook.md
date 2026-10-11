# ADR-0025: A Self Improving Playbook Proposes Config Patches Behind a Staged Trust Ladder

- **Status:** Proposed
- **Date created:** 2026-10-11
- **Date modified:** 2026-10-11

## Context

Issue #532 asks whether playbook can get better at its own job without someone driving each change, and asks for a design decision before any build. This record answers its six questions and picks the autonomy model.

### What exists today (checked against the code)

| Piece | Where | What it does | Gap for learning |
| --- | --- | --- | --- |
| Memory signals | `src/hooks/memory_signals.rs` | Per node hit count, a node is `promoted` at 3 hits inside 7 days, a cached `stale` flag with a verified hash, a consolidation cursor. Ranking reads `ranking_signals`. | Promotion only flags a node. Nothing expires, supersedes or adds a fact from the counters. A hit is a read, so a fact is reinforced by being shown. |
| Capture and anchors hooks | `src/hooks/memory_capture.rs`, `src/hooks/memory_anchors.rs` | Capture blocks the Stop turn up to a bounded re-block count and asks the model to write durable facts. Anchors warn on edits to anchored files, capped at 600 characters and once per file per session. | No record of whether a captured fact was later used, corrected or ignored. |
| Staged learning | `commands/learn-project.md` (`--stage`, `--from-staged`) | Collects candidates without a human, writes them to a staging area, a human confirms later. Auto mode behaves as `--stage`. | Promotion out of staging is always a human step. |
| Usage store | `src/usage/db.rs`, `src/usage/query.rs` | `usage_events` (one row per assistant message: model, effort, repo, branch, session, tokens, cost) and `tool_invocation_events` (one row per Skill or Agent call). | A row has no component, so cost per agent comes only from telemetry. Linked PRs are counted, merged PRs are not. |
| Telemetry | `src/usage/otlp.rs`, `src/usage/telemetry_db.rs`, #787 to #794 | A local OTLP receiver keeps only an allowlist of keys (event name, session, request id, model, effort, agent name, token counts, cost) in `telemetry_requests`, reconciled per session against transcripts. | Prompts, tool content and paths are dropped by design. Claude Code also emits hook, tool result and retry events, and the allowlist drops them, so denials and retries are not stored. |
| Gate database | `src/gate/db.rs` | `gate_phases` keeps one verdict per plan and phase (`PASS`, `FAIL`, `WARN`, `INCONCLUSIVE`) with evidence. | Per finding outcomes (kept or dropped by verification) are not recorded. |
| Effort tuning | `src/effort/floor.rs`, `suggest.rs`, `patch.rs`, `auto.rs`, ADR-0022 | `playbook effort suggest` reports, `--patch` and `apply` write ledgered config keys with a backup, `reset` undoes them, `effort.auto` (`off`, `suggest`, `on`) picks a variant per dispatch. A floor table is the only thing that can grant a lower effort. | Covers effort only. Every floor equals the shipped effort today, so the honest output is often "no change". |
| Benches | `src/eval/bench.rs`, `src/eval/wu_bench.rs`, `playbook eval review-triage` | Single turn benches per agent (pass rate, cost, seeded finding recall), a Work Unit bench, a triage regression eval. | Single turn and no tools for most roles. The reviewer rests on 7 seeded bugs. Commands, the auditor and skill text have no bench. |
| Config store | `src/config/keys.rs` | Known keys with defaults and allowed values, for example `effort.auto`. | No `learn.*` key exists. |
| Hook denials | the guard hooks under `src/hooks/` | Print a deny or advisory message. | No durable count of denials, retries or overrides exists in `src/usage` or `src/gate`. |

ADR-0022 already set the pattern for one surface: rules and thresholds, permission from a benchmark floor, trigger from usage, a ledger, one command to reset, three stages. It also says that if #532 adds a shared autonomy key, effort tuning reads it. This record generalises that pattern and does not replace it.

### What this month's manual sweeps found

The sweeps ran from the cost and context reports of 2026-10-10 and 2026-10-11 (bench campaign, skills and agents bench, playbook versus plain, haiku first, quick wins, savings). Counting the changes they produced by what it would take to automate each:

| Class | Items | Example and size |
| --- | --- | --- |
| A. A config patch from a benchmark rule | 2 | The effort campaign moved critic and implementer to Sonnet medium and analyst and fact-checker to Haiku medium (bench campaign, about $16.5 of bench spend, critic about 40% cheaper per call). The Read size limit moved from 1000 to 500 lines and from 200 KB to 60,000 bytes after measuring 5,145 real reads: over 1000 lines caught 0, over 500 caught 9.6% of reads and 29% of tokens. |
| B. Detectable by a rule, fixed by an edit to code or prompt text | 4 | `agent: git` and `agent: auditor` did not resolve, so 536 runs forked into a general purpose agent on the session model, about $148 actual against about $64 at Haiku rates over 30 days (about $84, 2.4% of $3,558). Two guard messages and the anchors message were rewritten or capped. |
| C. Judgment | 5 | Description trimming (36 files, about 32% fewer description tokens, routing spot check), fresh subagents instead of forks, inline review briefs (reviewer run $0.0447 to $0.0395 at equal recall), the context hygiene loop, a launcher default of medium, keeping the reviewer on Opus. |

This is my count of eleven items from the reports, and a different grouping would move a row by one. The pattern is stable: only class A fits a config patch, and class A found its savings in one campaign. After that campaign the skills and agents report found a cost weighted saving of zero from further config changes, because no setting met the decision rule on a fixture that could separate it. The larger part of the month's saving came from class B and C, which edit code or prompts. Those are the surfaces the issue lists as riskiest, and this ADR keeps them out of automatic change.

The same reports name what cannot be measured today: cost per merged PR (the store counts linked PRs, 549 linked in 30 days, about $7.6 each, and these include open ones), per component cost without telemetry, quality of commands, and reviewer recall beyond 7 seeded bugs.

## Decision

### 1. One loop, one change surface, three autonomy levels

The loop is: observe, propose, evaluate, approve, apply, measure, roll back. It runs on demand and from a scheduled routine. It never runs inside a session hook, because a hook adds latency to every turn and must never fail a session.

The change surface is playbook's own config and its own local stores. In scope:

1. Memory metadata: promote a staged fact, mark a fact stale, expire or supersede a fact whose anchor is gone, change its rank weight.
2. Effort and tier choices, through the existing ADR-0022 stages.
3. Thresholds with a named constant (recall thresholds, hygiene threshold, Read size limit), as config keys where a key exists.
4. Guard message wording and skill description wording, as a proposal only (see 3).

Out of scope for any automatic change: hook code, agent and command prompt bodies, skill bodies, Claude Code settings and Claude Code memory. A learned change never edits these silently. At most it opens a draft pull request with a diff for a human to review like any other change.

The autonomy key is `learn.autonomy` with values `off`, `propose` and `auto`, default `off`. It is a ceiling shared by every learner. `effort.auto` keeps its own values (`off`, `suggest`, `on`) because it shipped, and the lower of the two decides: `learn.autonomy=off` forces `effort.auto` to behave as `off`, and `propose` caps it at `suggest`. A per surface key (`learn.memory.autonomy`, `learn.effort.autonomy`) may lower the ceiling further and never raise it.

| Level | Behaviour |
| --- | --- |
| `off` | Observe and report only. Nothing is written except local counters that already exist. |
| `propose` | Write a proposal file (and for repo-tracked changes, a draft pull request). Nothing applies until a human runs `playbook learn apply <id>` or merges the pull request. |
| `auto` | Apply a proposal without asking, only if it is in the allowed class for that surface (4) and passes every gate (5). Everything else stays a proposal. |

### 2. Signals

Safe means local, content free, and already allowed by the privacy design of the OTel receiver.

| Signal | Source | Status | Use | Weakness |
| --- | --- | --- | --- | --- |
| Cost, tokens, model, effort per session | `usage_events` | Exists | Cost per session, trend, budget | Shared by main session and agents |
| Cost per agent | `telemetry_requests.agent_name` | Exists (#794) | Cost per component | Needs telemetry on |
| Dispatch counts, reruns, overrides | `tool_invocation_events` | Exists | ADR-0022 rules | Orchestrator and user choices look alike |
| Memory hits, promotion, staleness | `memory_signals` | Exists | Memory rules | A hit is a display, not a use |
| Review phase verdicts | `gate_phases` | Exists | Weak outcome signal | One row per phase, no findings |
| Merged PR, review rounds, fix after merge | local git and optional `gh` read | Not collected | Cost per merged PR, rework | `gh` is opt in, fix after merge is noisy |
| Finding kept or dropped | `deep-review` verification | Not recorded | Reviewer precision | Needs a new table |
| Hook denial and retry counts | hook outcome | Not recorded | Guard message tuning | Needs a counter with no content, only guard name and a count |
| User override of a choice | a dispatch variant or a config edit differing from the proposal | Partly (variant names) | Veto signal | Cannot tell a user from the orchestrator |
| Corrections typed in chat | transcripts | Rejected | None | Free text, can hold secrets and names. Not read. |
| Memory fact later contradicted | a newer fact on the same anchor | Not collected | Memory expiry | Needs the contradiction check from consolidation |

Two rules bind every signal:

- Only counters, names from a closed set (guard name, agent name, fact id, command) and numbers are stored. Free text from prompts, tool inputs, outputs and transcripts is never stored in a learning record, and never copied into a fact by the loop. Memory facts written by the existing capture flow keep their existing evidence check.
- Absence of a bad signal is not evidence of quality. As in ADR-0022, usage evidence can trigger or veto a change, and only a benchmark or a deterministic check grants permission.

### 3. What it may learn, and how

| Surface | Learned how | Permission comes from | Max autonomy |
| --- | --- | --- | --- |
| Memory: promote staged fact | Rule: at least 3 hits across at least 2 sessions, anchor still verifies, no contradicting fact | The deterministic anchor check | `auto` |
| Memory: expire a fact | Rule: every anchor is gone and the fact has not been read in 60 days. It is moved to an archive, never deleted | The deterministic anchor check | `auto` |
| Memory: supersede | A newer fact on the same anchor with a different claim | A human, because it needs judgment | `propose` |
| Memory ranking weights | Boost facts followed by a kept edit, decay unread ones | Replay on stored sessions, ranking must not lose a fact that was used | `propose` |
| Effort and tier | ADR-0022 stages 1 to 3 | The floor table | Stage 3 of ADR-0022 |
| Recall and hygiene thresholds | Choose from the measured distribution, as the Read size limit was | Replay over local history shows the new value catches at least as many of the cases it exists for | `propose` |
| Guard message wording | Candidates from denial counts: a guard denied often and followed by a retry of the same call | A human | `propose` only |
| Skill description and routing | Candidates from the routing spot check method used in #776 | The routing eval must not drop any unit, plus a human | `propose` only, never `auto` |

A tier of Haiku, Sonnet or Opus is part of effort and model routing and follows the ADR-0022 floor table and the #575 approval gates. This ADR adds no second router.

### 4. The loop

1. **Observe.** `playbook learn observe` reads the stores listed above and writes one local summary per window. It makes no network call. The `gh` reads are opt in.
2. **Propose.** Each learner emits zero or more proposals. A proposal file holds an id, the surface, the exact change (a config key and value, or a memory operation on a fact id), the evidence counts, the gate results and the rollback command. Cold start (under the ADR-0022 minimums) emits nothing.
3. **Evaluate.** The proposal must pass the gates for its surface (5) before it is shown as applicable. Effort uses the quality floor and the existing benches. Skill text uses the routing eval. Memory uses a replay.
4. **Approve.** At `propose` a human applies it. At `auto` the loop applies it only for the classes marked `auto` above. Anything touching a costly to miss component, a protected path, or code and prompt text always needs a human.
5. **Apply.** The same mechanism as `playbook effort apply`: a backup of the previous values, a ledger entry per key or fact id, a line in a local append only change log.
6. **Measure.** After a change, the next two windows are compared with the window before it on the success metric and the veto signals (8). The result is appended to the change log.
7. **Roll back.** `playbook learn reset` undoes every ledgered change in one command, `playbook learn reset <id>` undoes one. A watchdog rolls back by itself, and says so in the next session start block, when a veto signal fires after a change (a rise in reruns or overrides for the touched component, a rise in rework, or cost above the weekly budget).

Where it runs: on demand (`playbook learn run`) and from a scheduled routine the user opts into. A SessionEnd hook may only append counters that exist already. A learner is never launched from a hook.

### 5. Guardrails

1. **Quality floor.** Any change that could lower quality needs a benchmark entry that allows it. Memory and threshold changes need the replay described above. No floor is lowered by this ADR.
2. **Never lower effort where a miss is costly.** The reviewer, the auditor and anything marked costly in `src/effort/floor.rs` are raise only. Protected paths (auth, secrets, crypto, migrations, permissions, CI, hooks) never get a lower effort.
3. **Human approval by default.** `learn.autonomy` defaults to `off`. Moving to `propose` and then to `auto` is a user config change, per surface.
4. **Staged trust ladder.** A surface moves up one level only after a track record: at `propose`, at least 10 applied proposals over at least 4 weeks with no rollback and no veto signal. The check prints the record. It never raises the level itself. The user does.
5. **Every change is logged and reversible.** An append only change log, a ledger, a backup of what was replaced. A key the user later edits by hand leaves the ledger and is not touched by reset.
6. **A budget and a kill switch.** A weekly cap on learning spend (benchmark and routing evals call a model) with a default of zero, so nothing runs a model unless the user sets a cap. `learn.autonomy=off` stops everything at the next run.
7. **Local only.** No network call from the loop apart from the opt in `gh` read of the user's own PRs. Nothing is sent anywhere. A learning record holds counters and ids only, so a record holds no PII.
8. **No silent code or prompt change.** A proposal that edits code or a prompt is a draft pull request, which goes through the normal signed commit and review path.
9. **Config and Claude Code state.** The loop writes only playbook's own config and stores. It never writes Claude Code settings or Claude Code memory.

### 6. Avoiding overfitting and feedback loops

- **Reinforcement by display.** A hit counts only when the fact was injected and a later edit touched one of its anchored files in the same session. A fact shown and ignored decays. Hits caused by the loop's own proposals (a test run, a replay) are tagged and excluded.
- **Self confirmation.** A fact written by capture is not allowed to count its own capture session as a hit. Promotion needs at least 2 distinct sessions.
- **Small windows.** Minimum evidence before any proposal (ADR-0022: 20 dispatches, 5 sessions, 7 days). A single heavy session cannot move a threshold.
- **Held out check.** A threshold or ranking change is tuned on the older 70% of the window and must hold on the newest 30%.
- **One change at a time per surface.** A second change waits until the measurement window of the first closes, so effects can be attributed.
- **Ratchet limits.** A learned value may move at most one step per change (one effort level, one threshold step of 20%) and may never cross the shipped default by more than that, except through a benchmark floor.
- **Recurring bench as the control.** When a proxy moves, the response is a bench re-run with new seeded cases, as in ADR-0022 section 5, never a change on the proxy alone.

### 7. Failure modes

| Failure | Effect | Response |
| --- | --- | --- |
| Memory poisoning (a wrong fact captured, then promoted) | A bad fact steers later sessions | Promotion needs a passing anchor check and 2 sessions. Facts carry their evidence. `reset` archives, never deletes. |
| Stale signals | A change made on old data | Windows are bounded and each record carries its window |
| Prompt drift | Wording changes slowly lose behavior | Text is never changed automatically. A routing eval gates the proposal. |
| Cost creep | A loop that spends to save | Weekly budget defaults to zero. The change log shows spend against savings. |
| Quality loss that nobody sees | A missed finding is silent | Floor table, costly to miss list, recurring bench, never a change on absence of signals |
| Oscillation | A value flips between two settings | One change per window, a minimum dwell, ratchet limits |
| Gaming the metric | Cost falls because work was skipped | The metric is paired with a quality floor (8) |
| Telemetry or store missing | No evidence | Say "not enough data" and propose nothing |
| Loop crash | A session is affected | The loop never runs in a hook. A failure writes a log line and exits. |

### 8. Success metric

The primary metric is **cost per merged PR at equal review quality**, measured per repo over a 30 day window:

- Cost: the sum of `usage_events` cost for sessions whose branch ended in a merged PR (merge state from local git history, or `gh` when the user opts in).
- Equal review quality: review rounds per PR (count of review passes before merge) and fix after merge within 14 days must not rise by more than 5 percentage points against the 30 days before the change, and the seeded bench recall on touched components must hold within 3 points.

Secondary: stale facts in the active set, share of promoted facts that a later session used, hook denials per 100 sessions, rollbacks per applied change.

Today this metric cannot be computed. The store counts only linked PRs, so a first slice must add merge state to the store or read it from git before any claim of improvement. Until then the loop reports "no baseline". A change that cannot be measured stays at `propose`.

## Rollout

Each stage ships on its own and is judged on its own record. Stage 0 is the smallest and needs no model call.

0. **Observe and baseline.** `playbook learn observe` reports the signals that exist today, lists which are missing, and prints cost per merged PR as "no baseline" until merge state is read. Adds `learn.autonomy` (default `off`) to the known keys. Writes nothing else. This is the first slice.
1. **Close the loop for memory (issue #532 step 2).** Rule based promotion, expiry by dead anchors into an archive, ledger, change log, `learn reset`. Behind `learn.autonomy`. At `propose` it writes proposals, at `auto` it applies promotion and expiry only. No model call.
2. **Signals that are missing.** Record kept or dropped findings in `deep-review`, a content free hook denial counter, merge state for the metric.
3. **Share the key with effort.** Make `effort.auto` obey the `learn.autonomy` ceiling, and add the `playbook learn` entry that lists effort proposals from ADR-0022.
4. **Thresholds and routing proposals.** Replay based proposals for recall and hygiene thresholds, guard and description wording as draft pull requests.
5. **Raise autonomy per surface** after a recorded track record (guardrail 4). The default stays `off` for new installs.

## Alternatives considered

- **Do nothing.** Cheapest, and the manual route keeps working. Rejected as the end state because the sweeps found real money once (about $84 per 30 days from one prefix bug, 29% of full read tokens above the old limit) and the checks that would have found the class B items are cheap rules. Kept as the default behavior: with `learn.autonomy=off` playbook behaves as today.
- **Manual sweeps every quarter.** What we ran this month. They found class C items that no rule would, and they cost a coordinator and bench quota each time (the campaign alone was about $16.5 of bench spend plus the work to write the reports). Kept as the way to handle judgment items. The loop is meant to catch class A and B between sweeps, so the sweep can concentrate on class C.
- **A learned model that tunes routing, memory ranking and text.** Rejected for now, for the reasons in ADR-0022: there are no labelled outcomes, a model is hard to explain and test, and its failures are silent. Reopen with a new ADR when the loop has at least a quarter of merged PR outcomes to hold out against.
- **Autonomous edits to prompts, skills and hooks.** Rejected. The change is hard to evaluate (commands and the auditor have no bench) and a prompt regression is silent. Allowed only as a draft pull request.
- **Remote or shared learning across users.** Rejected. It conflicts with local only data, and one user's habits are not another's.
- **A separate autonomy key per learner with no shared ceiling.** Rejected. A single ceiling gives the user one switch to turn everything off.

## Consequences

- The first two stages add no model calls and no new cost. The loop is a small amount of Rust over stores that exist.
- Most of this month's savings fall in classes the loop will not change by itself. Expect the loop to mainly keep memory tidy and to flag candidates, and expect the larger savings to keep coming from reviewed changes.
- Cost per merged PR needs new data. Stage 0 states the gap and stage 2 closes it.
- Memory expiry moves facts to an archive, so a wrong expiry is recoverable.
- Every learner shares the `learn.autonomy` vocabulary. A change to a level name later is a breaking change, so the names are fixed here.

## Follow up issues this implies (not created)

1. `feat(learn): add learn.autonomy key and playbook learn observe report` (stage 0).
2. `feat(memory): rule based promotion and expiry by dead anchors with ledger and reset` (stage 1, issue #532 step 2).
3. `feat(gate): record kept and dropped findings in deep-review` (needed for reviewer precision).
4. `feat(hooks): content free denial and retry counters per guard` (needed for guard tuning).
5. `feat(usage): store merge state per branch to compute cost per merged PR` (needed for the metric).
6. `feat(effort): obey the learn.autonomy ceiling` (stage 3).
7. `feat(learn): replay harness for ranking and threshold proposals` (stage 4).
8. `chore(eval): a larger seeded defect set for the reviewer and a routing eval for descriptions` (raises the floor evidence).
9. `feat(learn): scheduled routine and weekly learning budget` (opt in).
10. `feat(learn): trust ladder check that prints the record before a level change`.
