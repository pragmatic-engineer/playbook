# Shell Quoting for `gh api -f body=`

When posting via `gh api -f body="..."`, the body is in **double quotes**. Inside double quotes:

- Apostrophes are literal: `"I don't"` is correct. NEVER escape them: `"I don\'t"` posts as `I don'''t`.
- Double quotes inside the body need escaping: `"he said \"hello\""`.
- Dollar signs need escaping if not a variable: `"\$100"`.

If the body contains complex quoting, write it to a file in the scratchpad and post it from the file, which needs no escaping. Intent: pr-reply. If the session's tool map (printed at session start) lists it, use that command with the PR number, the thread id and the file. Otherwise do this, with `$ID` the `databaseId` of the thread's first comment:

```bash
gh api "repos/{owner}/{repo}/pulls/$PR/comments/$ID/replies" -F body=@"$REPLY_FILE"
```

For a comment on the pull request itself, use `gh pr comment "$PR" --body-file "$REPLY_FILE"`.

Leave the file in place. The rm guard denies any `rm` whose path holds a variable other than `$HOME`.
