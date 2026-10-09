---
description: Use at the start of a session after /clear, or when the saved handoff did not load. Loads the handoff the last session saved for this directory and orients from it.
allowed-tools: Bash, Read
argument-hint: "[--all]"
model: haiku
effort: low
---

# Session Start

Pick up where the last session in this directory stopped. `/playbook:session-handoff` saves a handoff with `playbook handoff save`, and the SessionStart hook loads it on its own after `/clear`. Run this command when that automatic load did not appear, or to read the handoff again.

## Run this now

Run every step immediately and do not ask for confirmation.

1. Load the handoff. It prints the freshest one for this directory and never deletes it. Add `--all` when `$ARGUMENTS` has it.

   ```bash
   playbook handoff show
   ```

   A handoff the hook already loaded still prints, labelled "already loaded by the last session start".

2. If it says `No handoff saved for this directory`, say so in one line and stop. When it lists handoffs saved for other directories, name the one that looks closest and tell the user how to load it: `cd` into that directory, or run `playbook handoff show --dir <path>`. Then run `playbook handoff status` and report the last `clear` row. If no `clear` row follows a `/clear`, Claude Code did not run the SessionStart hook.

3. Orient in three to five lines: where we are, the decisions that still matter, and the next steps in order. Use only what the handoff says.

4. Check it against reality with `git branch --show-current` and `git status --short`. Compare the branch and the working tree with the handoff's "Where we are now". Name each difference in one line. Do not fix anything.

5. Finish with the first next step as the proposed action. Do not start it.
