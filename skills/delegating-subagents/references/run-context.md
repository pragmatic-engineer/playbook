# Reading `playbook run-context`

Every command that reads the run mode runs `playbook run-context` in its Step 0. This file holds the rules for its JSON, its exit codes and its failures, so the commands do not each carry a copy.

The JSON has `mode` (`ask` or `auto`), `source`, `hook_mode`, `warning` and `ceiling`. If `warning` is not empty, print it once. `ceiling` is the highest effort the user allows for this run (`null` means no limit): hold your own work to it, and before you spawn an agent follow the `delegating-subagents` skill, which runs `playbook effort resolve agents <agent>` and uses the `subagentType` it returns. Exit 2 means `--auto` and `--ask` were both passed: stop with the line it printed. For any other failure, do not stop and do not retry: run in ask mode with no ceiling and print one line. When the error is `unrecognized subcommand 'run-context'`, the installed binary is older than this plugin: print "playbook binary is older than the plugin, run `playbook update`."

A command whose JSON has no `ceiling` key skips the ceiling rule, and its fallback on failure is to run in ask mode and print one line.
