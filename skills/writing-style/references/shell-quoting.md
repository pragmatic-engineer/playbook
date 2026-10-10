# Shell Quoting for `gh api -f body=`

When posting via `gh api -f body="..."`, the body is in **double quotes**. Inside double quotes:

- Apostrophes are literal: `"I don't"` is correct. NEVER escape them: `"I don\'t"` posts as `I don'''t`.
- Double quotes inside the body need escaping: `"he said \"hello\""`.
- Dollar signs need escaping if not a variable: `"\$100"`.

If the body contains complex quoting, use a heredoc or temp file instead:

```bash
REPLY_FILE="/tmp/reply-body.txt"
echo "I don't think this will be a problem." > "$REPLY_FILE"
gh api "repos/$REPO/pulls/$PR/comments/$ID/replies" -X POST -F body=@"$REPLY_FILE"
rm -f "$REPLY_FILE"
```
