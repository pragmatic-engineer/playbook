# Changelog

Release notes for playbook: the binary and the plugin in `plugins/playbook`. From 0.32.0 they ship together as one `vX.Y.Z` release with one version ([ADR-0038](docs/adr/0038-one-release-line.md)). Each release has a section named `## playbook <version>`, newest first, which the release pull request adds from the notes that `scripts/release-notes.sh` builds. The sections below 0.32.0 are plugin releases from before the two lines joined. The binary skips 0.27 to 0.31: it goes from 0.26 to 0.32.0.

## playbook 0.33.0

- `engineering-standards-rust` gets eight more references: async and concurrency, iterators and collections, performance, unsafe and FFI, security and input, testing, tooling and CI, and project structure. The rules index in `SKILL.md` lists each one with a load-when line.
- The tooling reference ends with the Rust setup of pragmatic-engineer/playbook as a worked example.
- `systematic-debugging` and its condition-based-waiting guide point to the Rust skill, the same way they point to the JavaScript one.
- Secrets in `Debug` and error responses for network services now have one home, in the security reference.

## playbook 0.32.0

- A new skill, `engineering-standards-rust`, holds the Rust rules for code and reviews. It sits next to `engineering-standards-javascript` and adds only the Rust specifics. Its first references cover ownership and borrowing, error handling, types and traits, API design, style and naming, and a checklist of antipatterns. Most rules name the Clippy lint or command that checks them.
- `engineering-standards` points to the new skill for Rust, the same way it points to the JavaScript one.
- A skills lint test now checks that each `references/<name>.md` named in a `SKILL.md` exists.

## playbook 0.31.3

- The doctor command text matches the binary again: the upgrade record row hints `run: playbook migrate`, and the next session start or `playbook migrate` brings the record up to date.
- A short note after layer 6 says what to run after an upgrade: `playbook migrate`, or `playbook doctor --fix`, which runs the same step. It also says where each update channel shows breaking changes.

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
