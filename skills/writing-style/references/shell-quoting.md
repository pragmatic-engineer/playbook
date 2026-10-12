# Shell Quoting for `gh api -f body=`

When posting via `gh api -f body="..."`, the body is in **double quotes**. Inside double quotes:

- Apostrophes are literal: `"I don't"` is correct. NEVER escape them: `"I don\'t"` posts as `I don'''t`.
- Double quotes inside the body need escaping: `"he said \"hello\""`.
- Dollar signs need escaping if not a variable: `"\$100"`.

If the body contains complex quoting, write it to a file in the scratchpad and post it with `playbook pr reply`, which reads the body from the file and needs no escaping:

```bash
playbook pr reply --pr "$PR" --thread "$ID" --body-file "$REPLY_FILE"
```

Leave the file in place. The rm guard denies any `rm` whose path holds a variable.
