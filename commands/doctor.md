---
description: Check the seven playbook layers and print a status table with a remediation hint for each miss.
allowed-tools: Bash, Read
argument-hint: ""
model: sonnet
effort: low
---

# Doctor

Run all seven checks below. Do not stop early if one fails. Then print a
status table with one row per layer.

## Layer 1: Plugin enabled

```bash
claude plugin list 2>/dev/null | grep -qi 'playbook'
```

Pass if the output contains "playbook" and the status shows it is enabled.

Remediation hint on miss: "run: claude plugin marketplace add pragmatic-engineer/marketplace && claude plugin install playbook@pragmatic-engineer"

## Layer 2: Safety guards wired

Every guard runs inside the compiled binary (`src/hooks/*_guard.rs`), so
there is no per-guard script to check presence for: `settings.json` names
`playbook hook <name>`, the same bare form every other ported hook uses, and
whether that name resolves is purely a question of whether the `playbook`
binary itself is on PATH, which Layer 6 already checks once for every ported
hook, guards included. This layer's job narrows to the one question that is
still specific to the guards: is each one actually wired to that bare form?

```bash
wired=0; problems=""
for g in rm-workspace-guard bg-await-guard no-slop-guard precommit-check; do
  n=$(jq -r --arg cmd "playbook hook $g" \
      '[.hooks.PreToolUse[]?.hooks[]?.command // ""] | map(select(. == $cmd)) | length' \
      ~/.claude/settings.json 2>/dev/null)
  if [ "${n:-0}" -gt 0 ]; then
    wired=$((wired + 1))
  else
    problems="$problems $g:NOT_WIRED"
  fi
done
echo "wired=$wired/4$problems"
```

Report:

- `wired=4/4` → PASS.
- Any `NOT_WIRED` → **FAIL.** The guard is either still on its legacy
  `~/.claude/hooks/<name>.sh` command from before this change shipped, or
  missing from `settings.json` entirely; either way it is not running from
  the binary. Remediation: `playbook init`, which rewrites every guard's
  command to its bare form unconditionally, or `/playbook:setup` on a
  machine without the binary.

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

## Layer 6: Binary resolves

`settings.json` invokes every ported hook as a bare `playbook hook <name>`, with
no path (`src/init/wire.rs`, and the reasoning in its module doc). So the binary
has to resolve on PATH or all 11 ported hooks silently do nothing: the command
is not found, the hook produces no output, and the session carries on as if
nothing were wired. That is the same fail-open shape as a guard script that is
named but absent, which is why this is a hard failure rather than an INFO.

This layer exists because the installer cannot guarantee PATH on its own. It
appends a line to the rc file, but a Claude Code already running, or one
launched from the macOS Dock, has a PATH that no rc file can retroactively
change. That residual gap is precisely what this check is for.

```bash
if ! command -v playbook >/dev/null 2>&1; then
  echo "MISSING"
else
  bin_ver=$(playbook --version 2>/dev/null | awk '{print $NF}')
  manifest="${CLAUDE_PLUGIN_ROOT:-}/.claude-plugin/plugin.json"
  if [ ! -f "$manifest" ]; then
    manifest=$(ls -d "$HOME"/.claude/plugins/cache/*/playbook/*/.claude-plugin/plugin.json 2>/dev/null | sort -V | tail -1)
  fi
  man_ver=$(jq -r '.version // ""' "$manifest" 2>/dev/null)
  if [ -z "$bin_ver" ]; then echo "NO_VERSION"
  elif [ -z "$man_ver" ]; then echo "PRESENT_NO_BASELINE $bin_ver"
  elif [ "$bin_ver" = "$man_ver" ]; then echo "MATCH $bin_ver"
  else echo "SKEW binary=$bin_ver plugin=$man_ver"
  fi
fi
```

Report:

- `MATCH` → PASS.
- `MISSING` → **FAIL.** Every ported hook is dead. Remediation: install the
  binary and make sure its directory is on PATH. Until ADR 0007 WU-11 lands the
  fetch step, `install.sh` does **not** place the binary, so the honest hint
  today is to download the asset for your platform from the latest release, or
  build it with `cargo build --release`, and put it on PATH.
- `NO_VERSION` → **FAIL.** `playbook` resolved but `--version` printed nothing,
  so the file on PATH is not the binary this plugin expects. A stale shim or a
  name collision with another tool are both live causes; report the resolved
  path from `command -v playbook` so the user can see which.
- `SKEW` → **INFO, not FAIL.** The binary and the plugin manifest disagree on
  version. Say which is which rather than assuming the binary is the stale one:
  a user who built from source is legitimately AHEAD of the released plugin, and
  a user who updated the plugin without re-running the installer is behind.
  This mirrors the direction-unknown rule Layer 5 already applies to the status
  line, and for the same reason.
- `PRESENT_NO_BASELINE` → INFO, the binary is there but no plugin manifest was
  found to compare against, so skew cannot be judged.

**Layer numbering: do not renumber.** ADR 0007's WU-12 specified this as "Layer
5" and a statusline-existence check as "Layer 6", written before PR #143 shipped
the current Layer 5. Two corrections, recorded 2026-08-20. First, the binary
check is appended as Layer 6 rather than displacing the shipped Layer 5, since
renumbering a layer users and docs already refer to costs more than it buys.
Second, the proposed "Layer 6: the statusLine command path exists" was **not
implemented, because Layer 5 already does it**: its `MISSING` branch reports a
hard FAIL when the path in `settings.json` is absent. Adding it would have been
a duplicate check under a second number.

## Layer 7: No hook command points at a missing file

A hook command that names a file path fails **open** when that path does not
exist: `settings.json` still fires it, nothing runs, and nothing is
reported. That is the same silent-failure shape Layer 2 and Layer 6 both
guard against for the guards and the `playbook` binary, but neither
covers a one-off stray entry, such as a leftover Python hook from before this
project's Rust migration that a settings merge never removed (`wire()` only
manages the entries it recognises, so an entry for a retired hook name is
left exactly as it was). This layer checks every hook command in
`settings.json`, across every event, for that specific shape.

```bash
dangling=""
checked=0
while IFS= read -r cmd; do
  [ -n "$cmd" ] || continue
  case "$cmd" in
    "playbook hook "*) continue ;;
  esac
  last=$(printf '%s\n' "$cmd" | awk '{print $NF}')
  case "$last" in
    */*) ;;
    *) continue ;;
  esac
  path=${last/#\~/$HOME}
  path=${path//\$HOME/$HOME}
  case "$path" in
    *'$'*) continue ;;
  esac
  checked=$((checked + 1))
  if [ ! -e "$path" ]; then
    dangling="$dangling|$cmd"
  fi
done < <(jq -r '[.hooks | to_entries[]? | .value[]? | .hooks[]?.command // empty] | .[]' ~/.claude/settings.json 2>/dev/null)
dangling=$(printf '%s' "${dangling#|}" | tr '|' '\n' | sort -u | tr '\n' '|')
dangling=${dangling%|}
echo "checked=$checked dangling=$dangling"
```

A bare `playbook hook <name>` command never reaches the check: it names no
path, and Layer 6 already covers whether the binary itself resolves. Only a
command whose last whitespace-separated token looks like a path (contains a
`/`) is checked, the same convention Layer 5 already uses for
`statusLine.command`. A token that still contains an unexpanded `$VARIABLE`
after `~`/`$HOME` substitution is skipped rather than guessed at, so this
layer only ever reports a path it actually resolved and actually checked.

Report:

- `dangling` empty → PASS. Say how many commands were checked (`checked`);
  `checked=0` on a fully-ported install is expected and healthy, not a gap.
- `dangling` non-empty → **FAIL**, one line per entry. Remediation: the
  file is missing, so this hook does nothing every time it fires; `playbook
  init` will not remove a stray entry like this on its own, since it only
  manages the entries it recognises, so delete the entry from
  `~/.claude/settings.json` by hand, or fix the path if the file moved.

## Output format

Print a table with one row per layer. Use a clear status marker and a brief
label. For opt-in layers that are not installed, use a neutral marker (for
example INFO or SKIP) rather than FAIL. For each failing or missing item add a
one-line remediation hint. Example shape:

```
PASS  plugin enabled
PASS  safety guards wired (4 of 4)
INFO  launcher not installed (opt-in; run /playbook:setup)    -- run /playbook:setup and choose Yes for the launcher question
INFO  system prompt not installed (opt-in, recommended) -- run /playbook:setup and choose Yes for the system prompt question
INFO  status line differs from the shipped copy -- stale, or a local fix ahead of the release; a plugin install will overwrite it either way
FAIL  playbook binary not on PATH -- every ported hook is dead; install the release asset or cargo build --release, then ensure its directory is on PATH
FAIL  hook command points at a missing file: python3 ~/.claude/hooks/memory_context.py -- this hook does nothing every time it fires; playbook init will not remove it, delete the entry from ~/.claude/settings.json by hand
```

If all required layers pass and optional layers are installed, say so in one
line.

If any required layer fails, end with a remediation line that names the right
tool for what failed, rather than always pointing at `/playbook:setup`:

- Layers 1 to 5 → "Run /playbook:setup to fix the items above."
- Layer 6 → `/playbook:setup` cannot fix it. It does not install the binary, so
  say so and give the install instruction instead. Telling a user to run a
  command that cannot repair the thing that failed is worse than saying nothing.
- Layer 7 → `/playbook:setup` cannot fix it either, for the same reason it is
  not `playbook init`'s job: the entry is a stray one, not a managed one.
  Tell the user to remove or fix the named entry in `~/.claude/settings.json`
  directly.
