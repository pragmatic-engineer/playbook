# Security defaults

The security defaults are opt-in. A plain `playbook setup` does not add them. Turn them on with either of:

```bash
playbook setup --security
playbook config set --global security.defaults true   # then every `playbook setup` applies them
```

They are the `permissions` block of the shipped `seed/settings.shared.json`. The permissions default is conservative. It removes bare `Bash` and keychain commands from auto-allow, and moves twelve interpreters (`node`, `python3`, `npx`, `npm`, `make`, `awk`, `go`, `source`, `xargs`, `sqlite3`, `psql`, `docker`) from allow to ask. It also adds `Read(**/.env)` deny rules. Running it again changes nothing. It follows the usual settings merge, so a `permissions` block you customised wins.

Without the opt-in, `setup` leaves the `permissions` block in your `settings.json` as it is. It stops adding it and never removes one you already have. To turn the defaults off, run `playbook setup --no-security` or set `playbook config set --global security.defaults false`, then delete the `permissions` entries from `~/.claude/settings.json` by hand.

It is not a sandbox. `git`, `gh`, `find -exec` and the `sed` e-command still run without a prompt. The `Read(**/.env)` deny rules cover the `Read` tool only, so `cat .env` still runs.

To report a vulnerability, see [SECURITY.md](../../SECURITY.md). For the merge rules, see [Settings merge](00-install.md#settings-merge).
