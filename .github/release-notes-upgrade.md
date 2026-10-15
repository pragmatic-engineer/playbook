## Upgrade note

If you installed the hooks as the `playbook-hooks` plugin, run this once after you upgrade:

```bash
claude plugin uninstall playbook-hooks@pragmatic-engineer
playbook init --yes
```

The hooks now live in `~/.claude/settings.json`. `playbook init` adds them, and asks first when you run it in a terminal.

Since 0.34.0 the binary and the plugin share one version. The binary went from 0.26 to 0.34.0, so binary versions 0.27 to 0.33 do not exist. Those numbers were plugin releases.
