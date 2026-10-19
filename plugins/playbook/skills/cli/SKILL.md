---
name: cli
description: Points to the list of playbook binary commands. Use when a task might have a playbook command that the session tool map does not show, or before running a playbook command you are not sure about.
---

Run `playbook commands` to list every playbook command and subcommand (an older binary without it: run `playbook --help`); run `playbook commands <group>` for one group with its flags; never run a command marked user-only (tagged `(user-only)` or a name ending in `!`); instead, ask the user to run it with `!`.
