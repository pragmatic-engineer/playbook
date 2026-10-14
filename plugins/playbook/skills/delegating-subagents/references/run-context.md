# Reading the run mode

Every command that reads the run mode does it in its Step 0. This file holds the rules for the result, its errors and its failures, so the commands do not each carry a copy.

Intent: run-mode. If the session's tool map (printed at session start) lists it, use that command with the command's name and `$ARGUMENTS`. It prints JSON with `mode` (`ask` or `auto`), `source`, `hook_mode`, `warning` and `ceiling`. If `warning` is not empty, print it once. `ceiling` is the highest effort the user allows for this run (`null` means no limit): hold your own work to it, and before you spawn an agent follow the `delegating-subagents` skill, which resolves the agent under that ceiling. Exit 2 means `--auto` and `--ask` were both passed: stop with the line it printed. For any other failure, do not stop and do not retry: run in ask mode with no ceiling and print one line saying so.

Otherwise do this:

1. If the arguments hold both `--auto` and `--ask` as whole words, stop with `--auto and --ask conflict; pass one.`
2. `--auto` (or `--auto-design`) means auto mode for this run, and `--ask` means ask mode.
3. With neither flag, read `PLAYBOOK_MODE` from the environment (`echo "$PLAYBOOK_MODE"`). `ask` or `auto` sets the mode.
4. Anything else means ask mode, the default.

Without the tool there is no effort ceiling to read: treat it as no limit, and spawn the base agents named in the `delegating-subagents` roster.
