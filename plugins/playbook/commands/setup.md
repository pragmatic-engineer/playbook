---
description: Use on first install or to repair the local setup. Wires safety guards, settings.json, launchers and system prompt.
allowed-tools: Bash, Read, AskUserQuestion
argument-hint: "[--only <PART>...] [--yes] [--dry-run] [--explain [<PART>]] [--hooks|--no-hooks] [--statusline|--no-statusline] [--security|--no-security] [--path|--no-path] [--aliases|--no-aliases] [--system-prompt|--no-system-prompt] [--telemetry|--no-telemetry] [--claude-override|--no-claude-override] [--short-name|--no-short-name] [--help] [--auto] [--ask]"
model: haiku
effort: low
---

# Setup

Wire the playbook binary into Claude Code: the safety guards and hooks in settings.json, the status line, PATH, the optional launcher and system prompt, and telemetry for `playbook usage`. Each part is safe to run again and only changes what is missing.

This command is the `playbook setup` binary command in a Claude Code session. It takes the same flags, and its questions match the binary's in text, order and defaults. Only the binary writes files. Never edit `~/.claude/settings.json`, a shell file or the playbook config yourself: every change goes through `playbook setup`, and only after the user said yes to it.

The plugin's skills and commands work without the binary. This command, like /playbook:doctor, needs the binary because it is the binary's wiring step.

In the steps below, `playbook setup` means `playbook init` when Step 1 found an older binary (older versions: `playbook init`).

## Help

If `$ARGUMENTS` holds `--help` or `-h`, run `playbook setup --help`, or the old name with `--help` when that fails with "unrecognized subcommand" (older versions: `playbook init --help`), and print its output verbatim in a text block, with nothing before or after it. Then stop. The help writes nothing.

## Step 0: Read the run mode

Do this first. If `command -v playbook` finds nothing, stop with one line: "playbook is not on PATH. If you just installed it, open a new terminal and run /playbook:setup again; otherwise run `curl -fsSL https://raw.githubusercontent.com/pragmatic-engineer/playbook/main/install.sh | bash` first." Otherwise read the run mode for this run's arguments. Intent: run-mode. If the session's tool map (printed at session start) lists it, use that command, then read `skills/delegating-subagents/references/run-context.md` in this plugin and follow it for the warning, the both-flags exit, any other failure, and an outdated binary. Otherwise do this: if `$ARGUMENTS` holds both `--auto` and `--ask`, stop with one line saying they conflict; `--auto` or `--ask` alone sets the mode; with neither, use `PLAYBOOK_MODE` when it is `auto` or `ask`; else the mode is `ask`.

If the mode is `auto`, stop here. Print one line and nothing else: "/playbook:setup needs a person to run it, because it changes your global settings and shell files outside this repo." Do not run any later step.

In `ask` mode, behave exactly as this file describes.

## Step 1: Find the command name

```bash
playbook setup --help >/dev/null 2>&1 && echo setup || echo init
```

`setup` means the binary has the command. `init` means the binary predates the rename: use the old name with the same flags for the rest of this run (older versions: `playbook init`). The two names run the same code.

## Step 2: Read the arguments

Take `$ARGUMENTS` word by word:

- Drop `--auto` and `--ask`. Step 0 read them, and the binary does not take them.
- Replace `--install-aliases` with `--aliases` and `--use-system-prompt` with `--system-prompt`, and say so in one line. These are the old names this command used.
- Keep every other word as it is, in order. Do not check the flags yourself: the binary checks them. When it exits with code 2, show its error verbatim and stop.

The rest of this file calls the kept words the arguments.

## Step 3: Runs that ask nothing

These runs ask no question in a terminal either, so pass the arguments straight to the binary:

- The arguments hold `--dry-run` or `--explain`: run `playbook setup` with the arguments, print the output verbatim, and stop. Both write nothing.
- The arguments hold `--yes`, `-y`, `--defaults` or `--only`, or `--telemetry` with no other part flag (a bare `--telemetry` means `--only telemetry`): the user already chose. Go to Step 6 with the arguments as they are.

Otherwise go on to Step 4.

## Step 4: Ask the questions

`playbook setup` in a terminal asks one question per part, in the order of the Questions table below. It skips a part when the arguments already decide it with that part's `--x` or `--no-x` flag. Do the same:

1. Print the welcome text, verbatim: "This walks through each change playbook can make to your machine. Nothing is written until you confirm at the end. playbook never changes Claude Code's memory, and it edits settings.json only for the parts you accept."
2. Run `playbook setup --dry-run`. It writes nothing. When a line of its output is one of these three, print that line and do not ask the telemetry question, since setup would not ask it either:
   - `telemetry: already enabled in your environment or Claude Code settings, so nothing to ask`
   - `` telemetry: already set up by you, left as it is (`playbook usage status` shows how it is read) ``
   - `` telemetry: switched off by a setting setup will not change (`playbook usage status` shows where), so nothing to ask ``
3. Run `playbook setup --explain` once. It writes nothing. For each part it prints a `What it does:` line.
4. Ask the remaining questions with the AskUserQuestion tool, in table order, at most four questions per call. Each question uses the Header and Question columns verbatim and has two options, Yes and No. The default option comes first, and its label ends with " (Recommended)". The Yes description is the part's `What it does:` line from step 3. The No description is the If no column.

## Step 5: Summary and confirm

Print "Summary of your choices", then one line per part in table order: the part, on or off, and "will be applied" or "left as it is", plus "(set by a flag)" when the arguments decided it. Add `security: on (set by a flag)` when the arguments hold `--security`.

Build the command: `playbook setup`, then the arguments, then for each answered part its `--x` flag on a yes or its `--no-x` flag on a no. Print the command in a bash block.

Ask "Apply these changes?" with the AskUserQuestion tool, header "Apply", options "Yes (Recommended)" and "No". On No, print "Cancelled. Nothing was changed." and stop.

Every asked part now has a flag, so the binary writes what the user chose even though this session has no terminal. A part the user did not answer keeps the binary's own rule: without a terminal and without `--yes`, hooks, the status line and telemetry stay off unless a flag turns them on.

## Step 6: Run

Run the command, then check the tools the plugin needs and install only the missing ones:

```bash
playbook setup <the flags from Step 3 or Step 5>
playbook deps ensure --builtin
```

The binary carries its own settings template, system prompt and Brewfile, so neither command needs a plugin file. Setup also marks `~/.config/playbook` as trusted in an existing `~/.claude.json`, so Claude Code's trust dialog never blocks that folder.

When the setup command fails, check these cases in order:

1. It fails with "unexpected argument": the installed binary is older than the plugin. Run the same command again without that flag, and tell the user to upgrade with `curl -fsSL https://raw.githubusercontent.com/pragmatic-engineer/playbook/main/install.sh | bash`.
2. It reports the settings step skipped with "no template shipped": the binary is also older than the plugin. Give the same upgrade advice, because no guards were wired.
3. Any other non-zero exit: still run the second command, then report which setup steps failed.

If `playbook deps ensure --builtin` fails with "unrecognized subcommand" or "unexpected argument", skip it and say so.

## Step 7: Report

Report the output of both commands verbatim. Then say in one line what this run changed. `playbook setup` prints one line per step with its status (for example, `[=] statusline  already up to date` when nothing changed), then the totals and next steps.

End your report with:

"Re-running /playbook:setup is safe and only changes what is missing. Run /playbook:doctor to
verify the full status."

## Flags

The same flags as `playbook setup`, with the same text as `playbook setup --help`. A test in `crates/cli/tests/setup_command_parity.rs` keeps this table in step with the binary.

| Flag | What it does |
| --- | --- |
| `--hooks` | Wire every playbook hook into settings.json (guards, session and worktree hooks) |
| `--no-hooks` | Add no playbook hook entries, including the guards in the shared settings |
| `--statusline` | Use the playbook status line (sets statusLine.command to `playbook statusline`) |
| `--no-statusline` | Do not set or change statusLine in settings.json |
| `--security` | Also apply the security defaults (permissions rules); off unless asked or `security.defaults` is true |
| `--no-security` | Do not apply the security defaults, even when `security.defaults` is true |
| `--path` | Put the playbook binary on PATH for every shell start |
| `--no-path` | Do not edit shell files for PATH |
| `--aliases` | Add the `playbook` cd wrapper (follows `playbook cc` into a worktree) to your rc file |
| `--no-aliases` | Do not install the launcher |
| `--system-prompt` | Install the playbook system prompt (an existing copy is refreshed either way) |
| `--no-system-prompt` | Do not install the system prompt |
| `--telemetry` | Answer yes to the telemetry question: write the OpenTelemetry variables for the local usage collector into settings.json. Given alone it means `--only telemetry` |
| `--no-telemetry` | Do not write telemetry variables into settings.json |
| `--claude-override` | Define a `claude` shell function that loads the playbook system prompt for a plain session (sets `launcher.claudeOverride` in playbook's config) |
| `--no-claude-override` | Do not turn on the `claude` shell function |
| `--short-name` | Add `pb` as a short name for `playbook`, only when `pb` is free (recorded in playbook own config) |
| `--no-short-name` | Do not add the `pb` short name |
| `--only <PART>...` | Run only these parts (comma separated: hooks, statusline, security, path, aliases, system-prompt, telemetry, claude-override, short-name), ask nothing and leave everything else untouched. `playbook setup --only telemetry` turns telemetry on |
| `-y`, `--yes`, `--defaults` | Never ask: use the default for every part you did not answer with a flag |
| `--dry-run` | Print what would change for each part and change nothing |
| `--explain [<PART>]` | Describe a part, or every part when none is named, and change nothing |
| `-h`, `--help` | Print help (see a summary with '-h') |

This command also reads `--auto` and `--ask` (the run mode, Step 0), and the old names `--install-aliases` and `--use-system-prompt` (Step 2). It passes none of them to the binary.

## Questions

The questions `playbook setup` asks in a terminal, in its order, with its defaults. The same test keeps this table in step with the binary.

| Order | Part | Header | Question | Default | If no |
| --- | --- | --- | --- | --- | --- |
| 1 | `hooks` | Hooks | Wire the playbook hooks into ~/.claude/settings.json? (see them with `playbook hooks list`) | Yes | No hooks are added. The safety guards, session helpers and usage warm-up do not run. |
| 2 | `statusline` | Status line | Set the status line in ~/.claude/settings.json to `playbook statusline`? | Yes | Your status line stays as it is. |
| 3 | `path` | PATH | Add the playbook binary's directory to PATH in your shell startup files (~/.zshenv for zsh, ~/.bashrc and a login file for bash)? | Yes | Your shell files are not touched. Hooks may not find playbook unless it is already on PATH. |
| 4 | `aliases` | Launcher | Add one line to your shell rc file (~/.zshrc or ~/.bashrc) that defines a `playbook` function that follows `playbook cc` and `playbook ccd` into a worktree? | No | No line is added to your rc file, so `playbook cc` will not follow you into a new worktree. |
| 5 | `system-prompt` | Prompt | Copy the playbook system prompt to ~/.config/playbook/prompts/SYSTEM_PROMPT.md? | No | No file is copied. The launchers start without the playbook prompt. |
| 6 | `telemetry` | Telemetry | Turn on Claude Code telemetry by writing the OpenTelemetry variables for the local usage collector into the env block of ~/.claude/settings.json, so plain claude, the IDE and the desktop app feed `playbook usage`? (skipped with one line when telemetry is already on, set up by you, or switched off by a setting setup will not change) | Yes | No variables are written. `playbook usage` reads transcripts instead. |
| 7 | `claude-override` | claude | Make a plain `claude` load the playbook system prompt every time (it replaces Claude Code's built-in prompt), and pass calls that use their own flags straight to Claude Code? This sets `launcher.claudeOverride` in ~/.config/playbook. No Claude Code setting or memory is changed. | No | `claude` stays the plain Claude Code command. |
| 8 | `short-name` | Short name | Add `pb` as a short name for `playbook`? It is added only when `pb` is not already a command, alias or function on your machine. This sets `launcher.shortName` in ~/.config/playbook. | No | `pb` is not defined. Use `playbook` in full. |

The security part is never asked about, in a terminal or here. It runs only with `--security` or when `security.defaults` is true.
