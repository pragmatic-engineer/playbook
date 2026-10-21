# Layers

`/playbook:doctor` reports seven layers. A working install needs 1, 2 and 6.

| Layer | What it is |
|---|---|
| 1. Plugin enabled | The skills, commands and subagents. `claude plugin list` shows playbook and not disabled. The plugin writes nothing to `~/.claude`. |
| 2. Guards and settings | Safety guards and hooks wired into `settings.json`. |
| 3. Launcher | `playbook cc` needs no shell line. It warns when an rc file still has a retired launcher line. |
| 4. System prompt | `seed/prompts/SYSTEM_PROMPT.md`, copied to `~/.config/playbook/prompts/`. Opt in. |
| 5. Status line | `playbook statusline`, pointed at by `statusLine.command`. |
| 6. The binary | Every hook is a bare `playbook hook <name>`, so without it the hooks do nothing. |
| 7. No dangling hooks | Flags hook commands that point at a file that no longer exists. |

Layer numbers stay as they are. See [Launcher and hooks](../internals/01-launcher-and-hooks.md) and the [hook reference](hooks.md).
