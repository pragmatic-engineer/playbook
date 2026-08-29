---
description: Walk unresolved PR review comments one at a time, apply fixes or draft replies, then commit-and-push and post replies with the new SHA.
allowed-tools: Bash, Read, Edit, Write, Grep, Glob, Agent, Skill
argument-hint: "[PR number] [--bots] [--dry-run] [-y|--yes]"
model: opus
effort: high
---

# Address PR Comments

Iterate through unresolved review-thread comments and PR-level comments on a pull request. For each one: read the code, propose a fix or reply, get user approval, apply the edit (or post the reply), and move on. At the end, hand off to `/playbook:commit-and-push -A` and then post any queued thread replies that cite the resulting commit SHA.

## Discipline: receiving review feedback

Code review is technical evaluation, not emotional performance. Apply this loop to every comment before deciding fix/reply/skip:

1. **Read** the comment without reacting.
2. **Understand** the ask. Restate it in your head; if unclear, surface that to the user before acting.
3. **Verify** the claim against the actual code (use `Read`). Reviewers and bots can be wrong.
4. **Evaluate** whether the suggestion is right for THIS codebase, not in the abstract. Check for legacy reasons, YAGNI (grep for callers if the suggestion is "add feature X"), and conflicts with prior architectural decisions.
5. **Decide**: fix, reply explaining, push back with technical reasoning, or ask the user.
6. **Apply** one item at a time. Don't batch-edit and discover regressions later.

Push back when the suggestion breaks existing functionality, the reviewer lacks context, it violates YAGNI, or it's technically wrong for the stack. Push back with technical reasoning, not defensiveness. If you can't verify a claim without extra work, say so in the proposed action ("I can't verify this without running X; investigate, ask, or skip?").

Forbidden internal monologue and forbidden in draft reply text:

- "You're absolutely right" / "Great point" / "Good catch" / "Nice catch" / "Thanks for catching that"
- "Let me implement that now" (before verification)
- Any gratitude expression. The diff is the acknowledgement.

## Discipline: writing the reply

Invoke the `playbook:writing-style` skill before drafting any reply. It governs voice, contractions, banned words, banned openers, reply brevity (one sentence ideal, two max, a paragraph is never acceptable), and the GitHub-specific rules (no commit hashes in reply text, no markdown emphasis in inline comments, no em or en dashes). Load it once at the start of Step 4 and apply it to every reply in the loop; do not re-derive these rules from memory.

Shell-quoting gotcha for `gh api -f body=`: apostrophes in double quotes are literal (`"I don't"` is correct, NEVER escape to `"I don\'t"` which posts as `don'''t`). If the body contains complex quoting, write it to a temp file and use `-F body=@/tmp/reply.txt`.

## Argument parsing

Parse `$ARGUMENTS` token-by-token:

- Integer or `#<integer>` -> explicit `PR_NUMBER`.
- `--bots` -> `INCLUDE_BOTS=true` (default: skip CodeRabbit, Copilot review, Greptile, github-actions, etc).
- `--dry-run` -> `DRY_RUN=true` (list everything but never edit files, never post replies, never commit).
- `--yes` or `-y` -> `AUTO_COMMIT=true` (skip the final "proceed with commit?" gate; per-comment gates still apply).
- Empty or unmatched -> resolve PR from the current branch.

## Execution rules

1. Run every bash block for real. Don't simulate.
2. Read the file at the PR's head SHA before proposing a fix. The reviewer's `line` field refers to the post-diff line number.
3. Combine independent bash calls into a single tool call.
4. **NEVER resolve threads.** That's the reviewer's call. Reply only.
5. Never auto-commit until every queued comment has been triaged (or skipped) and the user has reviewed the staged diff.
6. If `--dry-run`, stop after Step 3 with a printed plan.
7. Skip bot authors unless `--bots`. Bot login list: `coderabbitai`, `coderabbitai[bot]`, `copilot-pull-request-reviewer[bot]`, `greptile-apps[bot]`, `github-actions[bot]`, `sonarqubecloud[bot]`, `codecov[bot]`, `dependabot[bot]`, `renovate[bot]`.

## Step 1: Resolve PR and capture context

```bash
ARGS="$ARGUMENTS"
INCLUDE_BOTS=false
DRY_RUN=false
AUTO_COMMIT=false
PR_NUMBER=""

for tok in $ARGS; do
  case "$tok" in
    --bots) INCLUDE_BOTS=true ;;
    --dry-run) DRY_RUN=true ;;
    -y|--yes) AUTO_COMMIT=true ;;
    \#[0-9]*) PR_NUMBER="${tok#\#}" ;;
    [0-9]*) PR_NUMBER="$tok" ;;
    *) echo "warning: ignoring unknown arg '$tok'" >&2 ;;
  esac
done

if [ -z "$PR_NUMBER" ]; then
  PR_NUMBER=$(gh pr view --json number -q .number 2>/dev/null) || { echo "error: no PR for current branch; pass a PR number" >&2; exit 1; }
fi

REPO=$(gh repo view --json nameWithOwner -q .nameWithOwner)
OWNER="${REPO%/*}"
NAME="${REPO#*/}"
HEAD_SHA=$(gh pr view "$PR_NUMBER" --json headRefOid -q .headRefOid)
ME=$(gh api /user -q .login)

echo "PR: $REPO#$PR_NUMBER"
echo "Head SHA: $HEAD_SHA"
echo "Me: $ME"
echo "Flags: bots=$INCLUDE_BOTS dry-run=$DRY_RUN auto-commit=$AUTO_COMMIT"
```

Capture `PR_NUMBER`, `OWNER`, `NAME`, `HEAD_SHA`, `ME`, and the three flag values. You need them for every later step.

## Step 2: Fetch unresolved review threads and PR-level comments

Two sources of comments: inline review threads (have a `path` and `line`) and PR-level issue comments (top of the PR page).

```bash
# Review threads. databaseId on the first comment is needed for REST replies.
gh api graphql -f query='
  query($owner: String!, $name: String!, $pr: Int!) {
    repository(owner: $owner, name: $name) {
      pullRequest(number: $pr) {
        reviewThreads(first: 100) {
          nodes {
            id
            isResolved
            isOutdated
            path
            line
            originalLine
            comments(first: 50) {
              nodes {
                id
                databaseId
                author { login }
                body
                url
                createdAt
              }
            }
          }
        }
      }
    }
  }' -F owner="$OWNER" -F name="$NAME" -F pr="$PR_NUMBER" \
  > /tmp/pr-comments-$PR_NUMBER-threads.json

# PR-level issue comments (no path/line)
gh api "/repos/$OWNER/$NAME/issues/$PR_NUMBER/comments" --paginate \
  > /tmp/pr-comments-$PR_NUMBER-issues.json
```

Now filter in Claude (not bash) so you can reason about each thread:

- Skip threads where `isResolved == true`.
- If `INCLUDE_BOTS=false`, skip comments whose author login is in the bot list above.
- Outdated threads (`isOutdated == true`, or `line == null`) MAY have valid feedback that no longer maps to a line. Surface them with an `(outdated)` tag and let the user decide.
- For each remaining thread, the **last** comment is usually the most recent ask. Show the whole thread but treat the latest comment as the prompt.

## Step 3: Display triage summary

Print one block summarising what was found, then list each thread/comment with an index. Keep it compact:

```
Found N unresolved review threads, M PR-level comments (K bot comments suppressed).

Threads:
  [1] src/foo.ts:42  alice: "this allocates on every call, can we cache?"
  [2] src/bar.ts:103 bob (outdated): "rename to fooBar?"
  [3] src/baz.ts:7   alice: "missing null check"

PR-level:
  [4] charlie: "tests please"
```

If `DRY_RUN=true`, stop here. Print "dry-run: no edits applied, no replies posted."

## Step 4: Iterate one at a time

For each indexed item, do this loop:

1. **Show context.** Print the file path, the comment author, the full body, and the URL. Then `Read` the file around the line (10 lines either side).
2. **Verify the claim.** Does the code actually do what the reviewer says? Read enough to be sure. If not, you're going to draft a reply rather than a fix.
3. **Choose an action and present it to the user:**

   - **Fix**: propose a concrete diff. Show the diff snippet before applying.
   - **Reply**: draft a one-or-two-sentence reply (no fix needed). Show the reply text.
   - **Both**: apply a fix AND queue a reply that will say "addressed in `<SHA>`" once we commit.
   - **Skip**: neither fix nor reply. Use sparingly. Skipped items get listed in the final summary so nothing slips through silently.

4. **Get user approval.** Ask `[F]ix / [R]eply / [B]oth / [S]kip / [Q]uit / [E]dit-then-fix`. Wait for the answer.

   - `Edit-then-fix` means: user wants to write a different fix than what you proposed. Wait for them to describe it, then apply.
   - `Quit` means: stop iterating, jump straight to Step 5 with what you have so far.

5. **Apply.** Everything above this sub-step (show context, verify the claim, choose the action, draft the exact diff or reply text, get user approval) stays in the main session unchanged. Once an action is approved and its content is fully decided, dispatch execution to `patch-applier` (`subagent_type: playbook:patch-applier`) rather than applying it directly. `patch-applier` holds `Edit` and `Bash`, so per `playbook:delegating-subagents` it delivers its outcome by file, not by return value alone: every dispatch below names a report file path at `/tmp/$REPO/address-pr-comments-$PR_NUMBER-item-<N>.report.md` (`<N>` is this item's index), and the main session reads that file the moment the dispatch returns or goes idle, before trusting any outcome.

   - **For Both, dispatch the fix half now and queue the reply text for Step 6.** The reply half of a **Both** action is NEVER dispatched here: only its content is decided now. Step 6 dispatches it after commit, once `<SHA>` is known. This deferred timing is the primary rule for Both, not a trailing exception to it.
   - For **Fix**, or the fix half of **Both**: dispatch `patch-applier` with the exact, already-approved diff and the report file path. Read the report file; it names the exact hunk applied, or a failure. Print the applied hunk to the user immediately, before advancing to the next indexed item.
   - For **Reply only** (no fix): dispatch `patch-applier` with the exact reply text, the exact command shape to run, and the report file path, matching the two existing shapes below:
     ```bash
     # Inline review-thread reply (use databaseId of the first comment in the thread)
     gh api -X POST "/repos/$OWNER/$NAME/pulls/$PR_NUMBER/comments/$DATABASE_ID/replies" \
       -f body="<reply text>"
     # PR-level issue comment reply
     gh pr comment "$PR_NUMBER" --body "<reply text>"
     ```
     Read the report file; it names the exact body posted, or a failure. Print the posted body to the user immediately, before advancing to the next indexed item.
   - One dispatch per approved item, applied immediately. Do not batch multiple items into one `patch-applier` call.
   - If the report file is missing, or names a failure (the diff did not apply, the `gh api` call failed), surface it to the user plainly and mark the item `failed` in the tracked state below, never `fixed`/`replied`. A missing report file is its own distinct outcome from a reported failure: say which one happened, don't guess.

6. **Track state.** Keep a running markdown list, one row per indexed item, with status `fixed | replied | both-queued | failed | skipped`. This is the audit trail.

## Step 5: Pre-commit confirmation

Print a summary:

```
Triaged N items: A fixed, B replied, C both-queued, D skipped, E failed.
Staged changes:
  <git diff --stat output>
```

Run `git diff --stat` to show what changed locally. If `AUTO_COMMIT=false`, ask the user `Proceed with commit-and-push? [Y/n]`. If the user says no, exit with the changes left in the working tree (do not stage, do not commit). Print the list of queued replies they'll need to post manually.

## Step 6: Commit, push, post queued replies

If approved (or `AUTO_COMMIT=true`):

1. Invoke the `commit-and-push` skill with the `-A` flag and an extra hint that the commit message should reference the PR (e.g. "address review comments on #<PR_NUMBER>"). The skill handles staging, formatting, message generation, rebase, and push. Capture the resulting commit SHA from the skill's output.

2. For each `both-queued` reply, finalise the body by substituting `<SHA>`, then dispatch `patch-applier` (`subagent_type: playbook:patch-applier`) with the exact finalised body, the command shape to run, and a report file path at `/tmp/$REPO/address-pr-comments-$PR_NUMBER-item-<N>.report.md`, the same convention Step 4 uses:
   ```bash
   gh api -X POST "/repos/$OWNER/$NAME/pulls/$PR_NUMBER/comments/$DATABASE_ID/replies" \
     -f body="$REPLY_TEXT_WITH_SHA"
   ```
   Read the report file the moment the dispatch returns or goes idle; it names the exact body posted, or a failure. Print the posted body before moving to the next queued reply. On a missing report file or a reported failure, apply the SAME rule Step 4 sub-step 5 uses: surface it to the user plainly, mark the item `failed` (not `both-queued` anymore), and do not count it toward "posted P queued replies" below. This is the same delegation Step 4 uses for an immediate reply, applied here to the deferred post: Step 4's "who executes the post changes, not when" claim depends on this step actually dispatching `patch-applier` too, not the main session running `gh api` directly.

3. Print a final summary:
   ```
   Done. Committed <SHA>, posted P queued replies, skipped D items, F failed posts.
   Skipped items needing follow-up:
     - <thread URL> author: comment-summary
   Failed posts needing follow-up:
     - <thread URL> author: reason
   ```

## Notes

- **Threads with multiple comments.** A thread can have a back-and-forth. Display all comments in chronological order but treat the latest non-author comment as the prompt. If the latest comment is from `$ME` (you replied earlier), surface that and let the user decide if there's anything still pending.
- **No silent edits.** Every file change must be visible to the user before the next iteration. If a file is edited (directly, or via `patch-applier`'s dispatch), print the resulting hunk.
- **Failure handling.** If a reply POST fails (404 on the thread, 403 if you're not a collaborator), don't retry blindly. Print the error, leave the reply in the queue, and continue with the rest. Surface the failures in the final summary so the user can post them manually.
- **Resuming after Quit.** If the user quits mid-iteration, print the remaining indexed items with their URLs so they can re-run `/playbook:address-pr-comments` later (the resolved/unresolved state on GitHub is still the source of truth).
