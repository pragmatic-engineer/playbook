---
description: Walk unresolved PR review comments one at a time, apply fixes or draft replies, then commit-and-push and post replies with the new SHA.
allowed-tools: Bash, Read, Edit, Write, Grep, Glob, Skill
argument-hint: "[PR number] [--bots] [--dry-run] [-y|--yes]"
model: opus
effort: max
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

Replies must read like a human typed them quickly in a code-review thread.

- **One sentence ideal, two max.** A paragraph is never acceptable.
- **Skip the opener.** Go straight to substance. "Fixed." not "Good catch, let me fix that."
- **Never paraphrase the reviewer.** They already said it. Restating wastes space and sounds like a chatbot confirming receipt.
- **Use contractions.** "doesn't", "won't", "there's", "it's". Uncontracted forms are an immediate AI tell.
- **Code-change replies are especially terse.** "Fixed.", "Done.", "Ok, done.", "Sorted." Add a brief locator only when it adds value ("Done, added the null check.").
- **Only explain when you deviated.** Verbatim application: no rationale. Different approach: one clause. Intentional disagreement: one clause.
- **No trailing hedges.** Don't end with "but good to be aware of", "worth keeping in mind", "we can revisit if needed." Say the thing and stop.
- **Match the reviewer's register.** Casual gets casual, formal gets formal. Australian slang is fine sparingly ("should be sweet", "too easy", "reckon that's fine").
- **No commit hashes in the reply text.** Humans don't write "fixed in abc1234" in conversation. Say "fixed in the latest push" instead. GitHub auto-renders the SHA when the reply lands.
- **No em or en dashes.** Hard rule. Use commas, colons, parentheses, or separate sentences.
- **No markdown emphasis in inline comments.** Plain text reads more human. `issue:` not `**issue:**`.

Banned words in replies (LLM vocabulary tells): `utilize`, `however`, `furthermore`, `moreover`, `hence`, `certainly`, `basically`, `actually`, `very`, `just`, `really`, `probably`, `delve`, `harness`, `pivotal`, `intricate`, `groundbreaking`, `remarkable`, `serves as`, `stands as`, `crucial`, `valuable`, `powerful`. Replace `utilize` with `use`, `however` with `but` (or a new sentence), `furthermore` and `moreover` with `and` (or a new sentence), `hence` with `so`. Drop the rest.

Banned reply openers (immediate AI tells, NEVER lead with these):

- "Good catch." / "Nice catch." / "Great catch."
- "You're right." / "Great point." / "Good point." / "Valid point."
- Any "Thanks for X" / "I appreciate" gratitude expression.

Banned reply patterns:

- Praise opener + restate the reviewer's finding + describe the fix. The diff is the description.
- Three-item lists when one or two would do. AI defaults to threes.
- Setup language: "It's worth noting", "In conclusion", "To summarise", "Let me explain".
- Persuasive authority tropes: "The real question is", "at its core", "fundamentally", "the deeper issue".
- Copula avoidance: say "is" and "are", not "serves as", "stands as", "functions as", "represents".

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

5. **Apply.**
   - For **Fix** or **Both**: use the `Edit` tool. Read the file first (you may have already done so in step 1). Verify the file compiles or at least re-`Read` the changed region to confirm the edit landed.
   - For **Reply only** (no fix): post the reply NOW via:
     ```bash
     # Inline review-thread reply (use databaseId of the first comment in the thread)
     gh api -X POST "/repos/$OWNER/$NAME/pulls/$PR_NUMBER/comments/$DATABASE_ID/replies" \
       -f body="<reply text>"
     # PR-level issue comment reply
     gh pr comment "$PR_NUMBER" --body "<reply text>"
     ```
   - For **Both**: queue the reply text and the thread ID; you'll post after commit (Step 6).

6. **Track state.** Keep a running markdown list, one row per indexed item, with status `fixed | replied | both-queued | skipped`. This is the audit trail.

## Step 5: Pre-commit confirmation

Print a summary:

```
Triaged N items: A fixed, B replied, C both-queued, D skipped.
Staged changes:
  <git diff --stat output>
```

Run `git diff --stat` to show what changed locally. If `AUTO_COMMIT=false`, ask the user `Proceed with commit-and-push? [Y/n]`. If the user says no, exit with the changes left in the working tree (do not stage, do not commit). Print the list of queued replies they'll need to post manually.

## Step 6: Commit, push, post queued replies

If approved (or `AUTO_COMMIT=true`):

1. Invoke the `commit-and-push` skill with the `-A` flag and an extra hint that the commit message should reference the PR (e.g. "address review comments on #<PR_NUMBER>"). The skill handles staging, formatting, message generation, rebase, and push. Capture the resulting commit SHA from the skill's output.

2. For each `both-queued` reply, finalise the body by substituting `<SHA>` and post it:
   ```bash
   gh api -X POST "/repos/$OWNER/$NAME/pulls/$PR_NUMBER/comments/$DATABASE_ID/replies" \
     -f body="$REPLY_TEXT_WITH_SHA"
   ```

3. Print a final summary:
   ```
   Done. Committed <SHA>, posted P queued replies, skipped D items.
   Skipped items needing follow-up:
     - <thread URL> author: comment-summary
   ```

## Notes

- **Threads with multiple comments.** A thread can have a back-and-forth. Display all comments in chronological order but treat the latest non-author comment as the prompt. If the latest comment is from `$ME` (you replied earlier), surface that and let the user decide if there's anything still pending.
- **No silent edits.** Every file change must be visible to the user before the next iteration. If you `Edit` a file, print the resulting hunk.
- **Failure handling.** If a reply POST fails (404 on the thread, 403 if you're not a collaborator), don't retry blindly. Print the error, leave the reply in the queue, and continue with the rest. Surface the failures in the final summary so the user can post them manually.
- **Resuming after Quit.** If the user quits mid-iteration, print the remaining indexed items with their URLs so they can re-run `/playbook:address-pr-comments` later (the resolved/unresolved state on GitHub is still the source of truth).
