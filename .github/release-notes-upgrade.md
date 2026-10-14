## Upgrade note

If you installed the hooks as the `playbook-hooks` plugin, run this once after you upgrade:

```bash
claude plugin uninstall playbook-hooks@pragmatic-engineer
playbook init --yes
```

The hooks now live in `~/.claude/settings.json`. `playbook init` adds them, and asks first when you run it in a terminal.
