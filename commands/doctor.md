---
description: Check the five playbook layers and print a status table with a remediation hint for each miss.
allowed-tools: Bash, Read
argument-hint: ""
model: sonnet
effort: low
---

# Doctor

Run all five checks below. Do not stop early if one fails. Then print a
status table with one row per layer.

## Layer 1: Plugin enabled

```bash
claude plugin list 2>/dev/null | grep -qi 'playbook'
```

Pass if the output contains "playbook" and the status shows it is enabled.

Remediation hint on miss: "run: claude plugin marketplace add pragmatic-engineer/marketplace && claude plugin install playbook@pragmatic-engineer"

## Layer 2: Safety guards wired

```bash
jq '[.hooks.PreToolUse[]?.hooks[]?.command]
    | map(select(test("rm-workspace-guard|bg-await-guard|no-dash-guard")))
    | length' ~/.claude/settings.json 2>/dev/null
```

Pass if the result is 3 or more.

Remediation hint on miss: "run /playbook:setup to seed settings.json with the guard hooks"

## Layer 3: Launcher (opt-in)

Detect the current shell:

```bash
basename "${SHELL:-}"
```

For zsh: pass if BOTH conditions hold:
1. `grep -qF 'shell/zsh/cc.zsh' ~/.zshrc 2>/dev/null`
2. `test -f ~/.claude/shell/zsh/cc.zsh`

For bash: pass if BOTH conditions hold:
1. `grep -qF 'shell/bash/cc.sh' ~/.bashrc 2>/dev/null`
2. `test -f ~/.claude/shell/bash/cc.sh`

For any other shell: report "shell not detected" and skip this check.

This layer is opt-in. Report "not installed (opt-in; run /playbook:setup)" rather than
a hard fail when either condition is false.

Remediation hint when not installed: "run /playbook:setup and choose Yes for the launcher question"

## Layer 4: System prompt (opt-in)

```bash
test -f ~/.claude/prompts/SYSTEM_PROMPT.md
```

This layer is opt-in. Report "not installed (opt-in, recommended)" rather than
a hard fail when the file is absent.

Remediation hint when not installed: "run /playbook:setup and choose Yes for the system prompt question"

## Layer 5: Status line matches the shipped copy

The status line is the one product file `/playbook:setup` cannot install or
repair (see the `statusline-install-and-doctor-gap` note), and it is **not
plugin-versioned**, so a plugin update does not refresh it. That combination
means the installed copy can sit silently out of step with the shipped one for
as long as nobody looks.

```bash
sl_cmd=$(jq -r '.statusLine.command // ""' ~/.claude/settings.json 2>/dev/null)
if [ -z "$sl_cmd" ]; then
  echo "NOT_CONFIGURED"
else
  sl_path=$(printf '%s\n' "$sl_cmd" | awk '{print $NF}')
  sl_path=${sl_path/#\~/$HOME}; sl_path=${sl_path//\$HOME/$HOME}
  shipped="${CLAUDE_PLUGIN_ROOT:-}/statusline.sh"
  if [ ! -f "$shipped" ]; then
    shipped=$(ls -d "$HOME"/.claude/plugins/cache/*/playbook/*/statusline.sh 2>/dev/null | sort -V | tail -1)
  fi
  if [ ! -f "$sl_path" ]; then echo "MISSING $sl_path"
  elif [ ! -f "$shipped" ]; then echo "PRESENT_NO_BASELINE $sl_path"
  elif cmp -s "$sl_path" "$shipped"; then echo "MATCH"
  else echo "DIFFERS $sl_path vs $shipped"
  fi
fi
```

Report:

- `MATCH` → PASS.
- `MISSING` → **FAIL.** The status line renders nothing. Remediation: copy it
  from the plugin, `cp "$shipped" "$sl_path"`, since `/playbook:setup` cannot.
- `DIFFERS` → **INFO, not FAIL, and say which direction is unknown.** A
  difference has two causes and this check cannot tell them apart: the installed
  copy is stale, or it is a local fix that is AHEAD of the released plugin. Both
  are worth knowing. Say so, and give the hint for both: if stale, copy the
  shipped one over it; if it is a deliberate local fix, note that the next
  plugin install will overwrite it, so the fix needs releasing to survive.
- `NOT_CONFIGURED` → INFO, opt-in, no status line is configured.
- `PRESENT_NO_BASELINE` → INFO, the file is there but no plugin copy was found
  to compare against, so drift cannot be judged.

**Do not label a difference "stale" without checking direction.** Verified on
2026-08-18: a locally fixed `statusline.sh` reported as differing from the 0.9.1
plugin cache while the older, buggy backup reported `MATCH`, because the baseline
is the RELEASED copy. Calling that "stale" would have told the user to overwrite
a good file with a broken one.

## Output format

Print a table with one row per layer. Use a clear status marker and a brief
label. For opt-in layers that are not installed, use a neutral marker (for
example INFO or SKIP) rather than FAIL. For each failing or missing item add a
one-line remediation hint. Example shape:

```
PASS  plugin enabled
PASS  safety guards wired (3 of 3)
INFO  launcher not installed (opt-in; run /playbook:setup)    -- run /playbook:setup and choose Yes for the launcher question
INFO  system prompt not installed (opt-in, recommended) -- run /playbook:setup and choose Yes for the system prompt question
INFO  status line differs from the shipped copy -- stale, or a local fix ahead of the release; a plugin install will overwrite it either way
```

If all required layers pass and optional layers are installed, say so in one
line. If any required layer fails, end with: "Run /playbook:setup to fix the items
above."
