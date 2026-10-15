---
description: Use after install or update, or when hooks or the system prompt seem not to run. Prints a status table of the seven playbook layers.
allowed-tools: Bash, Read
argument-hint: ""
model: haiku
effort: low
---

# Doctor

This command checks the optional playbook binary and what it wires. The plugin's skills and commands work without the binary, falling back to plain git, gh and file steps; the binary adds the safety guards, the hooks and the tool map printed at session start.

Run the checks with the `Bash` tool:

```bash
playbook doctor check
```

The binary runs all seven layer checks and the informational ones, and prints
one row per line as `LEVEL  message -- fix hint`, then a closing verdict.
Print its output as the status table. Add `--json` for the same rows as JSON
(`layer`, `level`, `message`, `hint`).

If the command fails with `command not found`, the binary is not on PATH: print
`INFO  plugin only, binary not installed: skills and commands work; guards, hooks and the tool map are off -- install the binary (see the Layer 6 hint), then run /playbook:setup` and the install
hint from Layer 6 below, and stop. If it fails with
`unrecognized subcommand 'check'`, the installed binary is older than this
plugin: print "playbook binary is older than the plugin, upgrade it with the install hint below."
and stop.

## What each layer checks

1. **Plugin enabled.** `claude plugin list` names playbook and does not show it
   disabled. Hint on a miss: `claude plugin marketplace add pragmatic-engineer/marketplace && claude plugin install playbook@pragmatic-engineer`.
2. **Safety guards wired.** `settings.json` runs the binary's hook command for the
   six PreToolUse guards (`rm-workspace-guard`, `bg-await-guard`,
   `no-slop-guard`, `precommit-check`, `commit-message-sanitizer`,
   `policy-guard`) and for the
   `commit-message-sanitizer` backstop on PostToolUse. A legacy `.sh` command or
   a near-miss name does not count. Fix: `playbook init`.
3. **Launcher (opt-in).** The rc file for your shell has the playbook
   shell init line and `playbook` is on PATH. An old `source .../cc.zsh` line
   reads as outdated (run `playbook init`). Not installed is INFO, never FAIL.
4. **System prompt (opt-in).** `~/.config/playbook/prompts/SYSTEM_PROMPT.md`
   exists. Absent is INFO.
5. **Status line command is current.** The binary's status line command passes. The retired
   `bash ~/.config/playbook/statusline.sh` fails (run `playbook init`). A custom
   command whose file is missing fails. A custom command that exists, or none at
   all, is INFO.
6. **Binary resolves.** `settings.json` runs every ported hook as a bare
   binary hook command, so the binary must be on PATH or all of them silently
   do nothing. Layer 6 prints these rows:
   - **Version row.** A different binary and plugin version is normal: the row
     still passes and names both versions. It passes while the plugin is inside
     the supported range (minimum 0.30.1, up to major 0) and the binary meets
     the plugin's `minBinaryVersion` from `plugin.json` metadata. Otherwise it
     warns with one fix command. A binary below `minBinaryVersion` or a plugin
     on a newer major gets the binary upgrade (`brew upgrade playbook` when the
     binary path resolves to Homebrew, else `playbook update`). A plugin older
     than 0.30.1 gets `claude plugin update playbook@pragmatic-engineer`. The
     plugin is found the same way as `playbook path plugin-root`, with
     `PLAYBOOK_PLUGIN_DIR` first.
   - **Upgrade record row.** Reads the version record in `config.db` read-only
     and compares it with the `playbook` first on PATH. It warns when the last
     heal failed, when upgrade items are pending, or when the record cannot be
     read, with the hint `run: playbook migrate`. No record yet is INFO (the
     next playbook command or session start writes it), an older record is INFO
     (the next playbook command or session start updates it), and both carry
     the same hint. A record newer than the binary (a downgrade) is INFO because
     nothing runs after a downgrade.
   - **Fixing the upgrade record.** `playbook migrate` runs the migrations that
     write only playbook's own files and records the binary that ran them. It
     does not write Claude Code settings: a change there prints the `playbook
     init` command that applies it. `--status` lists each migration and its
     state, and `--dry-run` shows what a run would start. Neither writes
     anything. Add `--json` for JSON. `playbook doctor --fix` runs the same
     step. After an upgrade, `playbook update` runs it, and so does the install
     script with a terminal or `--yes`. Both print the breaking changes the
     release lists. After `brew upgrade`, the caveats list them, and the next
     playbook command or session start runs the migrations.
   - It also requires the quality gate flags this plugin expects, and warns when an older `playbook` earlier on
     PATH hides a newer one.

   Install hint: `curl -fsSL https://raw.githubusercontent.com/pragmatic-engineer/playbook/main/install.sh | bash`
   or `brew install pragmatic-engineer/tap/playbook`, with its directory on PATH.
7. **No hook command points at a missing file.** A hook that names a path
   which does not exist fails open: it never runs and nothing says so. Bare
   binary hook commands are skipped, as is any path with an
   unresolved `$VARIABLE`. `playbook init --hooks` removes a dead entry that runs
   a missing script under `~/.claude/hooks` (it makes a backup first). Live
   scripts and commands with shell operators stay, so delete any other stray
   entry from `~/.claude/settings.json` by hand.

Layer numbers stay as they are. Do not renumber them: users and docs refer to
them.

## Informational rows

These never fail the check and carry no fix hint:

- The effective `autoReview.*`, `autoMerge.enabled`, `commit.signOff` and
  `pr.draft` values, each as `key: value (source: tier)`.
- Stale worktrees the binary's sweep dry run reports (read the
  convention from the path: `.claude/worktrees/agent-*` is the Agent tool,
  `.worktrees/<repo>/` is the session launcher's worktree, `review-worktrees/` is
  the PR review, `repos/<owner>/<repo>/worktrees/<plan-slug>/<wu-id>` is a `/playbook:implement`
  Work Unit), or "no stale worktrees found".
- Files you edited after playbook placed them, as `migration <id>: <message>`.

## Output format

Print the binary's rows unchanged, then its closing line. If a required layer
failed, the closing line names the right tool: `/playbook:setup` for layers 1 to
5, an install of the binary for layer 6 (setup cannot fix it), and a hand edit of
`~/.claude/settings.json` for layer 7.
