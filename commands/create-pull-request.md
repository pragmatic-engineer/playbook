---
description: Use when a branch is ready to open as a pull request. Runs pre-flight checks, writes a conventional-commit title and the team PR template, and opens it as a draft.
allowed-tools: Bash, Read, Skill
argument-hint: "[--help] [--ready] [--base <branch>] [--ticket <ID>] [--dir <path>] [--auto] [--ask]"
context: fork
agent: git
---

# Create Pull Request

Push the current branch and open a pull request. The title is a conventional-commit summary, the body follows the team template, and both obey `playbook:engineering-standards` (readiness, size) and `playbook:writing-style` (voice, banned words, no dashes). A PR opens as a **draft** by default, controlled by the `pr.draft` setting (`true`, the default, or `false`): `--ready` does not publish it immediately, it marks it for promotion to ready once `/playbook:finish-pull-request`'s self-review passes, since a human should never be the first reviewer of unreviewed code. With `pr.draft` set to `false`, the PR opens ready for review instead.

This creates a **new** PR. If one already exists for the branch, this stops and points you at `/playbook:address-pr-comments` or `/playbook:quick-review`.

## Run this now

Execute the steps below immediately, end to end, running every command for real. Do **not** narrate a plan, summarize `git status`, offer a numbered menu, or ask "what would you like me to do?" / "proceed? [Y/n]". There is **no confirmation gate**.

The mechanical work (base and ticket detection, pre-flight checks, diff gathering, push, PR creation, base check) lives in two compiled commands, `playbook pr prepare` and `playbook pr create`. Your job is the judgment in between: read the diff, then write the title and the body. Readiness problems (uncommitted work, a diff over the soft or enforced size limit, no tests) print as warnings and never pause. Only the hard aborts (on the base branch, nothing ahead of base, an existing PR, a diff over the 1500-line hard size limit) stop the run.

This command is built to run in an isolated subagent (`context: fork`) so the diff and drafting stay out of the main context. When it forks, your final message is the only thing the main conversation sees, so end with a concise outcome summary (the PR URL, title, base, and draft state). If you are instead reading this in the main conversation, run it here exactly the same way; do not wait for a fork and do not defer to the user.

## Argument flags

Read these from `$ARGUMENTS` once and remember them. `--base`, `--ticket`, and `--dir` go straight onto the two `playbook pr` calls, and `--ready` is read by `/playbook:finish-pull-request`.

- `--ready` → promote the PR to ready once `/playbook:finish-pull-request`'s self-review passes, instead of leaving it a draft. Does NOT change how the PR opens: that is decided only by `pr.draft`.
- `--base <branch>` → override the base branch. Pass it to both `playbook pr` calls.
- `--ticket <ID>` → force the ticket, skipping branch auto-detect (`none` omits the line). Pass it to `playbook pr prepare`.
- `--dir <path>` → publish the branch checked out in this directory. A caller that names a directory or branch outside the shell's current directory (for example `/playbook:implement` publishing a Segment that lives in a git worktree) passes it here. Pass it to both `playbook pr` calls. A forked shell can reset to the main checkout between calls, so without `--dir` the command acts on whatever repo the shell is in.
- `--auto` or `--ask` → set the run mode for this run. Step 0 reads it.
- `--help` → print the argument-hint line from this file's frontmatter and stop.

There is no confirmation flag or gate: the command always runs end to end.

## Execution rules

1. Run every command for real. Do not simulate output; use the real result to drive the next step.
2. Do not assume git state, diff contents, or `gh` output. Check them.
3. Copy what `playbook pr prepare` and `playbook pr create` print. Never recompute or restate a value one of them already computed.
4. Never run destructive git commands (`reset --hard`, `push --force`, `clean -f`) or skip hooks (`--no-verify`).
5. Derive the title and body from the actual diff and commit log, never from the branch name alone or from memory.
6. Pass the PR body with `--body-file`, never inline, to preserve formatting.

## Step 0: Read the run mode and load the skill rules (MUST run before drafting title or body)

**Read the run mode first.** Run:

```bash
playbook mode status --json
```

If the arguments contain `--auto`, add `--flag auto`. If they contain `--ask`, add `--flag ask`. If both are present, stop with one line: "--auto and --ask conflict; pass one." The JSON has four keys: `mode` (`ask` or `auto`), `source` (where it came from), `hook_mode` and `warning`. If `warning` is not empty, print it once. If the command fails, do not stop and do not retry. Run in ask mode and print one line. When the error is `unrecognized subcommand 'mode'`, the installed binary is older than this plugin: print "playbook binary is older than the plugin, run `playbook update`."

- **`ask` mode:** behave exactly as this file describes.
- **`auto` mode:** follow the Auto path below.

### Auto path

In auto mode, `/playbook:finish-pull-request` skips the `/clear` self-review. When `/playbook:implement` opened this PR, its review already covered the code. Any other caller gets no review there, and the finish skill's final report says that no self-review ran. If `--ready` was passed, it still promotes the draft. The `autoReview.fix` and `autoMerge.enabled` settings still apply in auto mode, except that the finish skill never merges a PR whose code no review covered.

**Then load the skill rules.** This step needs four things: `playbook:writing-style`'s voice/banned-words/dash rules, its "When creating PRs" guidance, its "Prohibited GitHub Content" rules (the PR title and body are posted to GitHub, so these apply), and `playbook:engineering-standards`' PR readiness criteria and size limits (used in Step 2). Reading each full skill file to get a subset that small is most of Step 0's own cost, so extract only those sections with `sed` instead of invoking the Skill tool. A guard checks each extracted block for a marker string, and for the one range whose end-marker is exact heading text rather than a heading *level* (engineering-standards' Readiness+Size slice), also checks that a later section's heading is ABSENT, so a rename of the end-marker heading fails loudly instead of silently pulling everything through end of file:

```bash
WS="${CLAUDE_PLUGIN_ROOT}/skills/writing-style/SKILL.md"
ES="${CLAUDE_PLUGIN_ROOT}/skills/engineering-standards/SKILL.md"
[ -r "$WS" ] || { echo "ERROR: $WS not found under \$CLAUDE_PLUGIN_ROOT/skills/. Read the full skill via the Skill tool instead." >&2; exit 1; }
[ -r "$ES" ] || { echo "ERROR: $ES not found under \$CLAUDE_PLUGIN_ROOT/skills/. Read the full skill via the Skill tool instead." >&2; exit 1; }

# Process-scoped names: two runs (even against different repos) never share
# a fixed /tmp path and overwrite each other's extracts.
EXTRACT_DIR="/tmp/create-pr-step0-$$"
mkdir -p "$EXTRACT_DIR"
CORE="$EXTRACT_DIR/writing-style-core.md"
PRS="$EXTRACT_DIR/writing-style-prs.md"
GH="$EXTRACT_DIR/writing-style-github.md"
ENG="$EXTRACT_DIR/eng-standards.md"

sed -n '1,/^# GitHub-Specific Rules/p' "$WS" | sed '$d' > "$CORE"
sed -n '/^### When creating PRs/,/^## /p' "$WS" | sed '$d' > "$PRS"
sed -n '/^## Prohibited GitHub Content/,/^## Examples/p' "$WS" | sed '$d' > "$GH"
sed -n '/^### Readiness/,/^### Review Comments/p' "$ES" | sed '$d' > "$ENG"

for f in "$CORE" "$PRS" "$GH" "$ENG"; do
  if [ ! -s "$f" ]; then
    echo "ERROR: $f extracted empty; the source skill's heading text likely changed. Read the full skill via the Skill tool instead before continuing." >&2
    exit 1
  fi
done
grep -q "IRON RULE" "$CORE" && grep -q "Banned Words" "$CORE" \
  || { echo "ERROR: writing-style core extraction is missing an expected rule; read the full skill via the Skill tool instead." >&2; exit 1; }
grep -q "Readiness" "$ENG" && grep -q "Size" "$ENG" \
  || { echo "ERROR: engineering-standards extraction is missing Readiness or Size; read the full skill via the Skill tool instead." >&2; exit 1; }
grep -q "Automated Testing" "$ENG" \
  && { echo "ERROR: engineering-standards extraction ran past Review Comments into Automated Testing; the end-marker heading likely changed. Read the full skill via the Skill tool instead." >&2; exit 1; }

echo "Skill sections extracted and verified:"
echo "  $CORE"
echo "  $PRS"
echo "  $GH"
echo "  $ENG"
```

Read all four printed paths with the Read tool now. Together they carry the same rules Step 0 has always needed: voice, banned words, the dash rule, "When creating PRs" guidance, the Prohibited GitHub Content rules, and the PR readiness/size limits enforced in Step 2. "Natural Imperfections" (the other subsection under writing-style's GitHub-Specific Rules) is deliberately not extracted: it governs injecting casual typos/imperfections into posted *review* comments, not PR titles/bodies, so it doesn't apply here. If any guard above fails, fall back to invoking the full skill via the Skill tool for that one file rather than proceeding without its rules.

The PR title and body are read by another engineer, so they use the humane `playbook:writing-style` register (warm, contractions, active voice), NOT the terse operator voice. Where they conflict, `playbook:writing-style` wins for anything posted to GitHub.

## Step 1: Prepare (version check, pre-flight, diff, ticket)

One command does it all. It resolves the branch and the base (flag, then the repo default, then `main`), refuses to run on the base branch, with nothing ahead of it, or over 1500 changed lines, prints the readiness lines, writes the diff and commit log to a file, and detects the ticket. Pass only the flags the caller gave:

```bash
playbook pr prepare --help >/dev/null 2>&1 || { echo "ERROR: this command needs a newer playbook binary; run the playbook installer again to upgrade it." >&2; exit 1; }
playbook pr prepare [--base <branch>] [--ticket <ID>] [--dir <path>]
```

If the first line fails, stop and tell the user to upgrade the playbook binary. There is no fallback.

What it prints:

- `branch=`, `state_dir=`, `base=<branch> (source: ...)`, and `commits_ahead= changed_lines= dirty_files= test_files_touched=`.
- A `VERDICT` line each for dirty files, size, and tests. **Copy every `VERDICT` line verbatim into the readiness block.** Do not recompute or paraphrase them. Every `VERDICT` is non-blocking: print them and move on without pausing.
- The diff stat and commit log, then `diff_file=<path>` and `ticket=<ID or empty>`.

If it prints `A PR already exists: <url>` it exited 0: report that and stop. Any other non-zero exit is a hard stop: report its message and stop.

Read the file named by `diff_file=` with the Read tool. This is the source of truth for the title and body. If it is large, read it in chunks; do not skip it.

If `ticket=` holds a real ID (not empty, not `none`), the body's first line is `Ticket: <ID>`. If empty or `none`, omit the line without asking.

## Step 2: Generate the title (conventional commits)

Derive the title from the diff and commit log gathered in Step 1.

- Format: `type(scope): summary`, e.g. `feat(auth): add SSO retry logic`. Scope is optional.
- Types: `feat`, `fix`, `refactor`, `perf`, `docs`, `test`, `build`, `ci`, `chore`.
- Imperative mood ("add", not "added" or "adds"), no trailing period.
- **Length: 72 characters maximum**, counting the entire line including the `type(scope):` prefix. This is a hard limit, not a target. If the draft exceeds it, tighten the summary (drop the scope, cut filler, shorten wording) until it fits; never open a PR with a title over 72 characters. `playbook pr create` re-checks the length and refuses a longer title before it pushes anything.
- The summary states the **effect** of the change, not a list of files.
- The ticket goes in the body, not the title.

## Step 3: Generate the body (MANDATORY template)

Fill this template exactly. Keep the section order. Follow `playbook:writing-style` throughout: active voice, contractions, no banned words, no em or en dashes, no "This PR..." filler.

```markdown
Ticket: PROJECT-1234

## Summary

<Why are we doing this? 1-2 sentences, active voice. Focus on the why, not the what. The bug being fixed, the requirement, the motivation. Do not echo the title.>

## What Changed

- <Bullets in plain terms. Describe concepts and context, not files. Give the reviewer what they need to follow the change. 3-8 bullets, grouped logically.>

## Notes for reviewers

- <Optional. Oddities, trade-offs, intentional tech debt, anything that needs human context. Drop this whole section if there's nothing to add.>

## Related work

- <Optional. Cross-references to other PRs or tickets. Drop this whole section if there's nothing to add.>
```

Rules for filling it:

1. **`Ticket:` line**: include only when Step 1 printed a real `ticket=` ID; otherwise delete the line so the body starts at `## Summary`.
2. **Summary**: the why, not the what. One or two sentences. If the title is `fix(cache): stop stale reads after invalidation`, the Summary explains why stale reads mattered, not that you changed the cache.
3. **What Changed**: every bullet maps to something real in the diff. Group by concept, don't enumerate files. Use the same terms the code uses (if it's a "handler", don't call it a "controller").
4. **Notes for reviewers**: drop the heading entirely if empty. Don't leave "N/A".
5. **Related work**: drop the heading entirely if empty.
6. No trailing "generated by" footer. No test-count noise. If CI covers it, the reviewer sees CI.
7. The body must carry no AI attribution (no `Claude-Session:` trailer, no `claude.ai/code/session` link, no co-author line naming Claude or Anthropic, no "generated with" footer) and no em or en dashes outside code. `playbook pr create` checks the title, the body, and every commit message, and refuses to push if it finds any.

## Step 4: Push and create

`playbook pr create` reads the `pr.draft` setting itself (`playbook config get pr.draft`, default `true`) and passes `--draft` to `gh` only when it is `true`, so do not add a draft flag yourself. With the default, the PR opens as a draft. With `pr.draft` set to `false`, it opens ready for review. `--ready` is not used here: `/playbook:finish-pull-request` reads it, after the self-review, to decide whether to promote a draft.

Write the finished body to `<state_dir>/pr-body.md` (the `state_dir=` value from Step 1) and create the PR in one command:

```bash
cat > "<state_dir>/pr-body.md" <<'PRBODY_EOF'
<the filled template goes here>
PRBODY_EOF
playbook pr create --title "<title>" --body-file "<state_dir>/pr-body.md" [--base <branch>] [--dir <path>]
```

It checks the title length and the attribution and dash rules, pushes (a rejected push stops the run before any PR exists), confirms the remote carries your HEAD, opens the PR (as a draft unless `pr.draft` is `false`), and checks that the PR's base matches the one resolved in Step 1, correcting it if not. It prints `PR: <url>` and a one-line summary.

If it refuses with a list of problems, nothing was pushed or created: fix the title or body (or amend the named commit) and run it again. If a run fails after the PR already exists, running it again reuses the open PR and finishes the base check.

Report the PR URL and a one-line summary (title, base, draft state), and say whether `--ready` was passed. End the report with the line "Next: invoke playbook:finish-pull-request for this PR." and add "--ready" to it when `--ready` was passed. This forked agent cannot run the self-review, fix, promote and merge work: the orchestrating session that receives this report does, through that skill.
