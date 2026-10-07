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

## Step 0: Read the run mode

Run `playbook mode status --json` and note the mode. This command behaves the same in either mode, so carry on if it fails.

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
wired_status="OK"
if ! command -v playbook >/dev/null 2>&1; then
  wired_status="UNKNOWN"
  hc_out=""; post_out=""
else
  hc_out=$(playbook doctor hook-commands-for-event ~/.claude/settings.json PreToolUse \
    rm-workspace-guard bg-await-guard no-slop-guard precommit-check \
    commit-message-sanitizer 2>/dev/null) || wired_status="UNKNOWN"
  # The sanitizer's backstop is wired separately, on PostToolUse.
  post_out=$(playbook doctor hook-commands-for-event ~/.claude/settings.json PostToolUse \
    commit-message-sanitizer 2>/dev/null) || wired_status="UNKNOWN"
fi
if [ "$wired_status" = "UNKNOWN" ]; then
  echo "UNKNOWN"
else
  wired=0; total=0; problems=""
  while IFS= read -r line; do
    [ -n "$line" ] || continue
    guard=${line%%=*}
    count=${line#*=}
    total=$((total + 1))
    if [ "${count:-0}" -gt 0 ]; then
      wired=$((wired + 1))
    else
      problems="$problems $guard:NOT_WIRED"
    fi
  done <<< "$hc_out"
  while IFS= read -r line; do
    [ -n "$line" ] || continue
    guard=${line%%=*}
    count=${line#*=}
    total=$((total + 1))
    if [ "${count:-0}" -gt 0 ]; then
      wired=$((wired + 1))
    else
      problems="$problems $guard(PostToolUse):NOT_WIRED"
    fi
  done <<< "$post_out"
  echo "wired=$wired/$total$problems"
fi
```

Report:

- `wired=6/6` → PASS: the five PreToolUse guards and the
  `commit-message-sanitizer` backstop on PostToolUse.
- Any `NOT_WIRED` → **FAIL.** The guard is either still on its legacy
  `~/.claude/hooks/<name>.sh` command from before this change shipped, or
  missing from `settings.json` entirely; either way it is not running from
  the binary. Remediation: `playbook init`, which rewrites every guard's
  command to its bare form unconditionally, or `/playbook:setup` on a
  machine without the binary.
- `UNKNOWN` → INFO, not a false PASS or FAIL: `playbook` is missing entirely
  or predates the `hook-commands-for-event` subcommand, so this layer
  cannot count anything. Layer 6 names which.

## Layer 3: Launcher (opt-in)

Detect the current shell:

```bash
basename "${SHELL:-}"
```

For zsh: pass if BOTH conditions hold:
1. `grep -qF 'shell/zsh/cc.zsh' ~/.zshrc 2>/dev/null`
2. `test -f ~/.config/playbook/shell/zsh/cc.zsh`

For bash: pass if BOTH conditions hold:
1. `grep -qF 'shell/bash/cc.sh' ~/.bashrc 2>/dev/null`
2. `test -f ~/.config/playbook/shell/bash/cc.sh`

For any other shell: report "shell not detected" and skip this check.

This layer is opt-in. Report "not installed (opt-in; run /playbook:setup)" rather than
a hard fail when either condition is false.

Remediation hint when not installed: "run /playbook:setup and choose Yes for the launcher question"

## Layer 4: System prompt (opt-in)

```bash
test -f ~/.config/playbook/prompts/SYSTEM_PROMPT.md
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
sl_cmd=$(playbook doctor statusline-command ~/.claude/settings.json 2>/dev/null)
sl_status=$?
if [ $sl_status -ne 0 ]; then
  echo "UNKNOWN, playbook too old or missing, see Layer 6"
elif [ -z "$sl_cmd" ]; then
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
- `UNKNOWN` → INFO, not the same thing as `NOT_CONFIGURED`. `playbook` failed
  to answer, either it is missing entirely or it predates the `doctor`
  subcommand, so this layer genuinely does not know whether a status line is
  configured. Layer 6 names which.

**Do not label a difference "stale" without checking direction.** Verified on
2026-08-18: a locally fixed `statusline.sh` reported as differing from the 0.9.1
plugin cache while the older, buggy backup reported `MATCH`, because the baseline
is the RELEASED copy. Calling that "stale" would have told the user to overwrite
a good file with a broken one.

## Layer 6: Binary resolves

`settings.json` invokes every ported hook as a bare `playbook hook <name>`, with
no path (`src/init/wire.rs`, and the reasoning in its module doc). So the binary
has to resolve on PATH or all 20 ported hooks silently do nothing: the command
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
  # Guarded: an empty $manifest means no candidate path was ever found, not a
  # subcommand failure, and clap reads a genuinely empty argument as a
  # missing one, so calling through with "" would misreport TOO_OLD.
  if [ -n "$manifest" ]; then
    man_ver=$(playbook doctor plugin-version "$manifest" 2>/dev/null)
    man_status=$?
  else
    man_ver=""
    man_status=0
  fi
  if [ -z "$bin_ver" ]; then echo "NO_VERSION"
  elif [ $man_status -ne 0 ]; then echo "TOO_OLD $bin_ver"
  elif [ -z "$man_ver" ]; then echo "PRESENT_NO_BASELINE $bin_ver"
  elif [ "$bin_ver" = "$man_ver" ]; then echo "MATCH $bin_ver"
  else echo "SKEW binary=$bin_ver plugin=$man_ver"
  fi
  if playbook gate record --help 2>/dev/null | grep -q -- '--source'; then
    echo "GATE_SOURCE=OK"
  else
    echo "GATE_SOURCE=MISSING"
  fi
  # Every distinct playbook on PATH, in order. An old first entry may predate
  # any subcommand, so this reads `--version` from each instead of asking it.
  first_ver=""; stale=0; n=0; entries=""
  seen="|"
  while IFS= read -r bin_path; do
    case "$seen" in *"|$bin_path|"*) continue ;; esac
    seen="$seen$bin_path|"
    v=$("$bin_path" --version 2>/dev/null | awk '{print $NF}')
    entries="$entries\nPATH_SHADOW_ENTRY $bin_path ${v:-?}"
    if [ "$n" -eq 0 ]; then first_ver="$v"
    elif [ -n "$first_ver" ] && [ -n "$v" ] && [ "$v" != "$first_ver" ] \
      && [ "$(printf '%s\n%s\n' "$first_ver" "$v" | sort -V | head -1)" = "$first_ver" ]; then stale=1
    fi
    n=$((n + 1))
  done < <(which -a playbook 2>/dev/null | grep '^/')
  if [ "$n" -gt 1 ]; then
    printf '%b\n' "${entries#\\n}"
    if [ "$stale" -eq 1 ]; then echo "PATH_SHADOW STALE_FIRST"; else echo "PATH_SHADOW MULTIPLE"; fi
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
- `TOO_OLD` → INFO, not FAIL. The binary resolved and reports a version, but it
  predates the `doctor plugin-version` subcommand, so this layer cannot check
  it against the plugin manifest. Not the same as `PRESENT_NO_BASELINE`: here a
  manifest may well exist, the binary is just too old to read it this way.
  Remediation: update `playbook`.
- `GATE_SOURCE=MISSING` → **FAIL, independent of the SKEW verdict above.**
  `gate record`/`gate check` both require a `--source` flag as of the gate
  staleness enforcement change; a binary built before that ships with neither
  flag, so every `/playbook:plan`, `/playbook:adr`, and `/playbook:implement`
  quality gate call fails outright, not just reads stale. Unlike `SKEW`, this
  is not a direction-unknown comparison: a binary missing the flag is broken
  for this purpose regardless of whether it is otherwise ahead of or behind
  the plugin manifest. Remediation: update `playbook` to a version that
  supports gate staleness enforcement.
- `PATH_SHADOW STALE_FIRST` → **WARN.** More than one `playbook` is on PATH and
  the first one, the one every hook runs, is older than a later one. The
  `PATH_SHADOW_ENTRY <path> <version>` lines name each binary in PATH order.
  Remediation: remove the stale one (for Homebrew, `brew uninstall
  playbook`) or put the newer one's directory first on PATH. `MULTIPLE` is
  INFO (several binaries, the first is not older). A single entry prints no
  `PATH_SHADOW` verdict and needs no report line.
- `GATE_SOURCE=OK` → no separate report line; folded into the `MATCH`/`SKEW`/
  etc. verdict above, since the installed binary already supports the flag
  every current caller passes.

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
hooks_status="OK"
if [ ! -f ~/.claude/settings.json ]; then
  hook_cmds=""
  hooks_status="UNKNOWN"
elif command -v playbook >/dev/null 2>&1; then
  hook_cmds=$(playbook doctor hook-commands ~/.claude/settings.json 2>/dev/null) || hooks_status="UNKNOWN"
else
  hook_cmds=""
  hooks_status="UNKNOWN"
fi
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
done <<< "$hook_cmds"
dangling=$(printf '%s' "${dangling#|}" | tr '|' '\n' | sort -u | tr '\n' '|')
dangling=${dangling%|}
echo "status=$hooks_status checked=$checked dangling=$dangling"
```

A bare `playbook hook <name>` command never reaches the check: it names no
path, and Layer 6 already covers whether the binary itself resolves. Only a
command whose last whitespace-separated token looks like a path (contains a
`/`) is checked, the same convention Layer 5 already uses for
`statusLine.command`. A token that still contains an unexpanded `$VARIABLE`
after `~`/`$HOME` substitution is skipped rather than guessed at, so this
layer only ever reports a path it actually resolved and actually checked.

The command list itself comes from `playbook doctor hook-commands`, not
`jq`: it walks the exact same shape (`.hooks | to_entries[]? | .value[]? |
.hooks[]?.command`) directly against the compiled binary, so a host with no
`jq` installed no longer reads as "zero commands, all healthy." `hooks_status`
is set to `UNKNOWN`, rather than letting an empty command list masquerade as
PASS, in every case this layer cannot actually answer the question: no
`~/.claude/settings.json` to read at all, `playbook` itself missing, or a
`playbook` build old enough to lack the `hook-commands` subcommand. A
`settings.json` that exists but fails to parse as JSON is not separately
distinguished (the subcommand reads that the same way as a legitimately
empty `.hooks`, matching Layer 5 and 6's existing string-field reads), so it
still reports `status=OK checked=0`; a corrupt `settings.json` is caught by
other means (the file would not have parsed for any other tool either).

Report:

- `hooks_status=UNKNOWN` → INFO, not FAIL: either `~/.claude/settings.json`
  itself is missing, or `playbook` is missing or too old to run this check
  (the latter two already reported by Layer 6), so this layer only needs to
  say it could not check rather than repeat that diagnosis.
- `dangling` empty (and `hooks_status=OK`) → PASS. Say how many commands
  were checked (`checked`); `checked=0` on a fully-ported install is
  expected and healthy, not a gap.
- `dangling` non-empty → **FAIL**, one line per entry. Remediation: the
  file is missing, so this hook does nothing every time it fires; `playbook
  init` will not remove a stray entry like this on its own, since it only
  manages the entries it recognises, so delete the entry from
  `~/.claude/settings.json` by hand, or fix the path if the file moved.

## Effective review, merge and sign-off config

This is informational, not one of the seven layers above: a non-default but
validly-configured `autoReview.*`, `autoMerge.enabled`, `commit.signOff` or `pr.draft`
value is nothing to "fix", so it carries no remediation hint and never fails
the check on its own.

```bash
if ! command -v playbook >/dev/null 2>&1; then
  echo "UNKNOWN"
else
  # One line per key, so a key that fails does not hide the ones that resolve.
  for key in autoReview.enabled autoReview.type autoReview.fix autoMerge.enabled commit.signOff pr.draft; do
    key_status=0
    key_out=$(playbook config get "$key" 2>&1) || key_status=$?
    if [ $key_status -eq 0 ]; then
      printf 'OK\t%s\n' "$key_out"
    elif printf '%s\n' "$key_out" | grep -q 'unknown config key'; then
      printf 'UPGRADE\t%s\n' "$key"
    elif printf '%s\n' "$key_out" | grep -q 'config file is not a valid JSON object'; then
      printf 'MALFORMED\t%s\n' "$(printf '%s\n' "$key_out" | grep -m1 'config file is not a valid JSON object')"
    else
      printf 'ERROR\t%s\t%s\n' "$key" "$(printf '%s\n' "$key_out" | head -n 1)"
    fi
  done
fi
```

`playbook config get <key>` prints `<key>: <value> (source: <tier>)` on
success, where tier is `repo`, `org`, `global`, or `default`; `default` means
no file at any tier set the key, so the value shown is the built-in default,
not one read off disk. It also prints `(source: default, <tier> value ignored)`
when a tier set an invalid value for the key: the built-in default is shown
and a warning goes to stderr. On a malformed tier file it instead prints an error
naming the file to stderr and exits non-zero; the `||` after the assignment
above exists so that failure is captured into `key_status`
rather than aborting this block (and, if this file's shell blocks share a
`set -e` context, the rest of the script), the same guard style Layer 7 uses
around `playbook doctor hook-commands`.

Output is `UNKNOWN`, or one tab-separated line per key. Report each line on
its own, as INFO, so a failing key never hides the keys that resolve:

- `OK<TAB><line>` → the line exactly as `playbook config get` printed it,
  for example `INFO  autoReview.enabled: true (source: default)`.
- `UPGRADE<TAB><key>` → `config get` said "unknown config key", so the
  installed playbook binary is older than this plugin and needs an upgrade
  for that key. Say `INFO  <key>: playbook binary needs an upgrade for this
  key`.
- `MALFORMED<TAB><error>` → print the error plainly; it already names the
  file that failed to parse.
- `ERROR<TAB><key><TAB><message>` → `INFO  <key>: could not check: <message>`.
- `UNKNOWN` (no `playbook` on `PATH`) → INFO, "could not check: playbook
  config unavailable", the same condition Layer 6 and Layer 7 already report
  as `MISSING`/`UNKNOWN` for their own checks.

## Stale worktrees

This is informational, not one of the seven layers above: a worktree `sweep
--dry-run` would remove is nothing to "fix" here and now, only to be aware of.
It never fails the check on its own.

```bash
if ! command -v playbook >/dev/null 2>&1; then
  echo "UNKNOWN"
else
  sweep_status=0
  sweep_out=$(playbook worktree sweep --dry-run 2>&1) || sweep_status=$?
  if [ $sweep_status -ne 0 ]; then
    echo "ERROR"
    printf '%s\n' "$sweep_out"
  else
    echo "OK"
    printf '%s\n' "$sweep_out"
  fi
fi
```

`playbook worktree sweep --dry-run` prints one line per registered worktree it
would act on, naming the path and the reason: `would remove (dry run)` for one
that has landed and is safe to remove, or a skip reason (`not landed`, `has
uncommitted changes`, `locked by a live process`, and so on) for one it leaves
alone. A worktree with nothing to report (for example one with no matching
convention) prints no line at all, so an empty `sweep_out` is a normal, healthy
result, not a sign the command failed. On a hard failure (`git worktree list`
itself failing, or a malformed config tier file), it exits non-zero and prints
an error naming what went wrong instead.

The reported path itself already names the convention: a path under
`.claude/worktrees/agent-*` is the Agent-tool convention, one under
`.worktrees/<repo>/` is the `cc worktree` launcher convention, one under
`review-worktrees/` is the PR review convention, and one under a
`repos/<owner>/<repo>/*/worktrees/` tree is the `/playbook:implement`
Work-Unit convention. When relaying a line, read the convention off the path
this way rather than treating the line as convention-less.

Report:

- `OK` with output → INFO, one line per worktree the sweep reported, exactly
  as `playbook worktree sweep --dry-run` printed it, for example `INFO  worktree
  /path/to/worktree: would remove (dry run)`.
- `OK` with no output → INFO, "no stale worktrees found".
- `ERROR` → INFO, print the captured error line(s) plainly; they already name
  what failed.
- `UNKNOWN` → INFO, "could not check: playbook worktree unavailable". Covers
  both `playbook` missing entirely and a `playbook` too old to have the
  `worktree` subcommand, the same underlying condition Layer 6, Layer 7, and
  the review, merge and sign-off config check already report as `MISSING`/`UNKNOWN` for their
  own checks.

## Pending migration notices

This is informational, not one of the seven layers above: a file you edited
after playbook placed it is kept as is, so there is nothing to "fix". Print a
row only when something is pending; print nothing when the output is empty.

```bash
if command -v playbook >/dev/null 2>&1; then
  playbook doctor pending-migrations 2>/dev/null
fi
```

Each output line is `<migration id>: <message>`. Report one `INFO  migration
<id>: <message>` row per line. No output, a missing `playbook`, or a
`playbook` too old to know the subcommand all mean no row.

## Output format

Print a table with one row per layer. Use a clear status marker and a brief
label. For opt-in layers that are not installed, use a neutral marker (for
example INFO or SKIP) rather than FAIL. For each failing or missing item add a
one-line remediation hint. Example shape:

```
PASS  plugin enabled
PASS  safety guards wired (6 of 6)
INFO  launcher not installed (opt-in; run /playbook:setup)    -- run /playbook:setup and choose Yes for the launcher question
INFO  system prompt not installed (opt-in, recommended) -- run /playbook:setup and choose Yes for the system prompt question
INFO  status line differs from the shipped copy -- stale, or a local fix ahead of the release; a plugin install will overwrite it either way
FAIL  playbook binary not on PATH -- every ported hook is dead; install the release asset or cargo build --release, then ensure its directory is on PATH
FAIL  hook command points at a missing file: python3 ~/.claude/hooks/memory_context.py -- this hook does nothing every time it fires; playbook init will not remove it, delete the entry from ~/.claude/settings.json by hand
INFO  autoReview.enabled: true (source: default)
INFO  autoReview.type: auto (source: default)
INFO  autoReview.fix: false (source: default)
INFO  autoMerge.enabled: false (source: default)
INFO  commit.signOff: true (source: default)
INFO  pr.draft: true (source: default)
INFO  no stale worktrees found
INFO  migration 0004-skills-edited: skills edited after install: demo; a plugin update replaces them, so move your edits into your own skill
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
