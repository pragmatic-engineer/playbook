# Internals: Codex skills

`playbook skills sync` copies the playbook skills and commands into the skill folder Codex reads. The code is in `crates/cli/src/skills_sync/`. The reasoning for multi-agent support is in [ADR 0015](../adr/0015-multi-agent-support.md).

Status: shipped for Codex. The planner and renderer, the apply step, the `playbook skills sync` command, the refresh at upgrade time and the uninstall step all exist.

## Where the copies go

The target is `~/.agents/skills`, the user skill path in the [Codex skills docs](https://learn.chatgpt.com/docs/build-skills) (read 2026-10-19) and in Codex source (`codex-rs/ext/skills/src/host_roots.rs`, which calls `$CODEX_HOME/skills` deprecated). It is not `~/.codex/skills`. Codex counts as present when `~/.codex` is a folder or `codex` is on `PATH`. `CODEX_HOME` is not read, because the skills path does not depend on it.

The first install needs an explicit `playbook skills sync`, because `~/.agents/skills` is your folder. After that, playbook refreshes only the folders it wrote and recorded.

## Names

Every skill `plugins/playbook/skills/<s>/` becomes `playbook-<s>/`. Every command `commands/<c>.md` becomes `playbook-<c>/SKILL.md`. The prefix keeps the shared folder free of clashes with your own skills and with other agents that read it. A name over 64 characters, or a skill and a command with the same name, is an error, and the planner has a test that runs on the real plugin folder.

## Copies, not links

Each folder is a real copy. A symlink would break when the plugin cache is replaced by an update, and Codex would see a dead skill.

## Rendering rules

- A skill keeps every file, including `references/`. The `name:` in `SKILL.md` becomes `playbook-<s>`.
- A command keeps only `name` and `description` in its frontmatter. The body gets a short preamble saying this is the playbook command for Codex, how to run it as `$playbook-<c>`, and what to do when a step names a tool Codex lacks.
- In every copied `.md`, `/playbook:<n>` becomes `$playbook-<n>`, `playbook:<n>` becomes `playbook-<n>`, and `../skills/<n>/` becomes `../playbook-<n>/`. This applies only when `<n>` is a name the plugin ships and the match ends at a word boundary. Other text, such as a URL or the key `update.plugin`, stays as written.
- Codex truncates a skill's main prompt at 8,000 bytes on one read path. A rendered `SKILL.md` over that keeps its frontmatter, the preamble and one line that tells the agent to read `references/playbook-body.md`. The full body goes in that file.

## Ownership

Playbook records, per folder, the plugin version and a hash of every file it wrote. A folder it did not write, or one you edited, or one with a file the record does not list, is left alone and reported. A folder whose record matches is replaced. A recorded folder the plugin no longer ships is removed when it still matches.

Each replacement writes to a temporary folder, records the change as pending, swaps the folders, then records it as done, so a crash at any step leaves a folder the next run can replace. One run at a time holds a lock.

## The swap window

A replacement renames the old folder aside and then the new one into place. For a moment one skill folder is missing. A Codex session that scans the folder at that exact moment does not see that one skill until its next scan.

## What does not work in Codex

Codex has no subagents and no `AskUserQuestion` tool, so a command step that dispatches an agent or asks a question through a tool does not run as written. The preamble tells the agent to do that step itself or to ask in chat.

## Open question

Whether a Codex agent follows the one line in a split stub that points at `references/playbook-body.md` is not yet verified in a running Codex. If it does not, skills and commands over 8,000 bytes run with their stub only.
