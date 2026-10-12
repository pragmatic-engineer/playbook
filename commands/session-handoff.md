---
description: Use at the end of a session, before /clear or compaction, or when asked to wrap up. Writes a handoff for the next session and saves it.
allowed-tools: Bash, Read
argument-hint: "[path]"
model: haiku
effort: low
---

# Session Handoff

Write a handoff for the next session. The binary holds the rules and the facts. You write only the judgment parts. Run every step now and do not ask for confirmation.

1. Read the rules and template: `playbook handoff template`.
2. Read the facts for "Where we are now": `playbook handoff facts` (add `--dir <path>` when step 5 will). Add the last lint, typecheck, test and build results you know of.
3. Write the handoff following the template. Decisions need a reason, next steps need a why-now, and open questions name what surfaced them.
4. Check the shape with `playbook handoff check`, feeding the document through a quoted heredoc (`<<'HANDOFF_EOF'`), never an unquoted heredoc or `echo`, because the facts hold repo text. Fix every problem it names and run it again.
5. Save it by feeding the same document through the same quoted heredoc to `playbook handoff save`. When a context-size nudge gave you a session id, add `--session <id>`. `--dir <path>` saves for another directory.
6. If `$ARGUMENTS` holds a path, also write the document there, creating parent directories, and confirm the path in one line. A bare filename goes in the current working directory. Never write to a path the user did not provide.

Print the handoff, then end with one line: run `/clear` and the next session loads the handoff automatically, or run `/playbook:session-start` to load it on demand.
