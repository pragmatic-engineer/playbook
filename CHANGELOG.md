# Changelog

Release notes for the playbook plugin in `plugins/playbook`. The binary has its own releases. Each plugin release has a section named `## playbook <version>`, newest first, and the top section always matches the version in `plugins/playbook/.claude-plugin/plugin.json`.

## playbook 0.31.2

- The doctor command text now matches the binary: it says a version difference still passes, describes the layer 6 upgrade record row, and names `playbook init --hooks` as the fix for dead hook entries under `~/.claude/hooks`.

## playbook 0.31.1

- The plugin now declares `minBinaryVersion` 0.26.0 in its metadata. The field is informational for now: nothing in the binary reads it yet.
- commit-and-push and create-pull-request tell the user to upgrade the binary when the rules step fails on an older binary (the error says 'skill reference not found' or 'no plugin root'), then fall back to reading the skill files.

## playbook 0.31.0

- A new command, `/playbook:review`, is the one entry point for reviews. It recommends a quick or a deep review from the diff, asks which you want with the recommendation marked first, then runs it. `--quick` and `--deep` skip the question. In auto mode it takes the recommendation.
- The recommendation comes from a fixed rule list: size, files, packages touched, risky paths (auth, crypto, secrets, CI, release, hooks, migrations, dependencies, the command surface), a security label, the test ratio, and changes that cannot alter behavior (docs, lock files, renames, tests). With the playbook binary it is `playbook review suggest`. Without it the same rules are in `skills/grounding-review/references/review-routing.md`.
- When no rule is decisive, one cheap `review-triage` pass gives a second opinion.
- `/playbook:quick-review` and `/playbook:deep-review` are unchanged and stay as direct entry points.

## playbook 0.30.1

- Fix: the commit-format reference is no longer read through a bare `$CLAUDE_PLUGIN_ROOT` path, which Claude Code does not fill in, so skills failed with "could not find the plugin root folder". The commands and the git agent now load the writing-style skill and read the file from its base directory.
- The same bare variable is gone from the fallback text in create-pull-request and commit-and-push.

## playbook 0.30.0

- A new reference, `conventional-commits`, sits in the writing-style references. It covers the commit format, the type table, scope and ticket rules, breaking changes, the description rules and good and bad examples.
- commit-and-push, create-pull-request (title only), the git agent and implement's per-Work-Unit commit step read it when they write a commit message or a PR title.
- It is based on Conventional Commits 1.0.0 (https://www.conventionalcommits.org/, CC BY 3.0).

## playbook 0.28.0

- Review comments now follow the Conventional Comments standard (https://conventionalcomments.org/): `<label> [decorations]: <subject>` with an optional discussion.
- The grounding-review skill has one definition of the nine labels and the decorations, in a new reference file with a label table and examples.
- A bare `blocking:` becomes `issue (blocking):`. Old comments in the bare form are still read.
- quick-review, deep-review, address-pr-comments, implement, finish-pull-request and the reviewer agents read that reference. The posting tiers now group comments by what must be fixed.
- The format is for code review comments only.

## playbook 0.27.3

- The install command in the doctor hint now names pragmatic-engineer/marketplace, the new name of the marketplace repository.
- The plugin manifest points at the playbook repository and no longer mentions the playbook-hooks plugin.

## playbook 0.27.2

- The plugin now lives in this repository. It moved from pragmatic-engineer/skills at commit a27dbe7c2c996b9d01008eb080edbc7dac639b3b, where its history stays readable.
- No change to the skills, commands, agents or output style.
