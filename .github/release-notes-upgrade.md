## Upgrade note

If you installed the hooks as the `playbook-hooks` plugin, run this once after you upgrade:

```bash
claude plugin uninstall playbook-hooks@pragmatic-engineer
playbook init --yes
```

The hooks now live in `~/.claude/settings.json`. `playbook init` adds them, and asks first when you run it in a terminal.

From 0.32.0 the binary and the plugin share one version, so the binary goes from 0.26 to 0.32.0 and skips 0.27 to 0.31. Those numbers were plugin releases.
