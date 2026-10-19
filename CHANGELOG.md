# Changelog

Release notes for playbook: the binary and the plugin in `plugins/playbook`. From 0.34.0 they ship together as one `vX.Y.Z` release with one version ([ADR-0038](docs/adr/0038-one-release-line.md)). Each release has a section named `## playbook <version>`, newest first, which the release pull request adds from the notes that `scripts/release-notes.sh` builds. The sections from 0.33.0 down are plugin releases from before the two lines joined. The binary skips 0.27 to 0.33: it goes from 0.26 to 0.34.0.

## playbook 0.38.0

### Breaking changes

- **cli**: the `playbook shell-init` command is removed. Start sessions with `pb cc` or `pb ccd`, and delete the `eval "$(playbook shell-init)"` line from your shell rc file, or run `playbook setup` to remove it after a backup. Until 1.0 the old line prints one notice on stderr and the shell still starts. ([#1152](https://github.com/pragmatic-engineer/playbook/pull/1152))

### Features

- **shell**: mark commands that meet a command substitution ([#1127](https://github.com/pragmatic-engineer/playbook/pull/1127))
- **hooks**: add a command parser for the rm guard ([#1126](https://github.com/pragmatic-engineer/playbook/pull/1126))
- **hooks**: list the rm guard's built-in roots per OS ([#1131](https://github.com/pragmatic-engineer/playbook/pull/1131))
- **hooks**: judge rm targets against the built-in roots ([#1132](https://github.com/pragmatic-engineer/playbook/pull/1132))
- **hooks**: follow cd in the rm guard ([#1139](https://github.com/pragmatic-engineer/playbook/pull/1139))
- **hooks**: follow variables in the rm guard ([#1140](https://github.com/pragmatic-engineer/playbook/pull/1140))
- **hooks**: add a built-in deny set to the rm guard ([#1142](https://github.com/pragmatic-engineer/playbook/pull/1142))
- **disk**: add the free space probe and disk.\* config keys ([#1141](https://github.com/pragmatic-engineer/playbook/pull/1141))
- **hooks**: judge more deletion commands in the rm guard ([#1151](https://github.com/pragmatic-engineer/playbook/pull/1151))
- **hooks**: run the rm guard for every Bash command ([#1154](https://github.com/pragmatic-engineer/playbook/pull/1154))
- **migrate**: attach each migration to its release version ([#1137](https://github.com/pragmatic-engineer/playbook/pull/1137))
- **disk**: add disk status and a doctor row ([#1153](https://github.com/pragmatic-engineer/playbook/pull/1153))
- **disk**: add disk clean ([#1155](https://github.com/pragmatic-engineer/playbook/pull/1155))
- **disk**: add disk reserve and disk release ([#1156](https://github.com/pragmatic-engineer/playbook/pull/1156))
- **disk**: list cargo target folders of any project in disk status ([#1158](https://github.com/pragmatic-engineer/playbook/pull/1158))
- **migrate**: name migrations by slug and convert old records ([#1157](https://github.com/pragmatic-engineer/playbook/pull/1157))
- **disk**: classify which Bash commands are builds ([#1166](https://github.com/pragmatic-engineer/playbook/pull/1166))
- **disk**: add the disk-guard hook that stops builds on a full disk ([#1162](https://github.com/pragmatic-engineer/playbook/pull/1162))
- **setup**: rename attribution-off to attribution with ON and OFF ([#1163](https://github.com/pragmatic-engineer/playbook/pull/1163))
- **memory**: add memory stats and print it after a rebuild ([#1169](https://github.com/pragmatic-engineer/playbook/pull/1169))
- **disk**: wire the disk guard into setup, the session and status line ([#1167](https://github.com/pragmatic-engineer/playbook/pull/1167))
- **cli**: add playbook commands, a list built from the clap tree ([#1173](https://github.com/pragmatic-engineer/playbook/pull/1173))
- **setup**: retire the attribution-off migration and add flag aliases ([#1171](https://github.com/pragmatic-engineer/playbook/pull/1171))
- **disk**: clean up old migration backups and stop making more ([#1170](https://github.com/pragmatic-engineer/playbook/pull/1170))
- **hooks**: add playbook hooks explain-rm to show rm guard verdicts ([#1176](https://github.com/pragmatic-engineer/playbook/pull/1176))
- **migrate**: list each item that needs you with its fix ([#1175](https://github.com/pragmatic-engineer/playbook/pull/1175))
- **plugin**: add the playbook:cli skill pointing to playbook commands ([#1177](https://github.com/pragmatic-engineer/playbook/pull/1177))

### Fixes

- **release**: keep full multi-line breaking change footers ([#1125](https://github.com/pragmatic-engineer/playbook/pull/1125))
- **hooks**: let the policy guard allow reads of the config store ([#1129](https://github.com/pragmatic-engineer/playbook/pull/1129))
- **hooks**: judge rm guard targets from real command words ([#1128](https://github.com/pragmatic-engineer/playbook/pull/1128))
- **setup**: name the collector agent com.github.pragmatic-engineer ([#1134](https://github.com/pragmatic-engineer/playbook/pull/1134))
- **migrate**: move the collector agent to its new label ([#1136](https://github.com/pragmatic-engineer/playbook/pull/1136))
- **rm-guard**: allow playbook's folders, protect only irreplaceable data ([#1165](https://github.com/pragmatic-engineer/playbook/pull/1165))
- **policy-guard**: allow store reads, leave deletions to the rm guard ([#1172](https://github.com/pragmatic-engineer/playbook/pull/1172))
- **migrate**: show each pending item once, then once a day ([#1179](https://github.com/pragmatic-engineer/playbook/pull/1179))

### Other changes

- test: give every playbook test a scratch home ([#1130](https://github.com/pragmatic-engineer/playbook/pull/1130))
- docs: add the rm guard guide and ADR 0045 ([#1133](https://github.com/pragmatic-engineer/playbook/pull/1133))
- test(hooks): pin the rm guard's verdicts on a corpus of real commands ([#1138](https://github.com/pragmatic-engineer/playbook/pull/1138))
- docs: describe migrations by slug and release version ([#1160](https://github.com/pragmatic-engineer/playbook/pull/1160))
- refactor(settings): make settings.shared.json the one permissions source ([#1161](https://github.com/pragmatic-engineer/playbook/pull/1161))
- refactor(setup): look for the seed in the seed folder of a checkout ([#1164](https://github.com/pragmatic-engineer/playbook/pull/1164))
- docs(memory): point shipped text at data/memory.db and guard it ([#1174](https://github.com/pragmatic-engineer/playbook/pull/1174))
- refactor: move the shipped seed to seed/ and intents.json to plugins/ ([#1168](https://github.com/pragmatic-engineer/playbook/pull/1168))
- refactor: remove breaking-changes.json and playbook breaking ([#1178](https://github.com/pragmatic-engineer/playbook/pull/1178))
- chore(dev): tidy the justfile, the seed Brewfile and the script tests ([#1180](https://github.com/pragmatic-engineer/playbook/pull/1180))

## playbook 0.37.0

### Breaking changes

- **setup**: `playbook setup --aliases` and `--claude-override` do nothing now. Start sessions with `pb cc` or `playbook cc`, which load the playbook system prompt. A plain `claude` no longer goes through the launcher. ([#1119](https://github.com/pragmatic-engineer/playbook/pull/1119))
- **cc**: the playbook, claude and pb shell functions are gone. `playbook cc` no longer changes the shell's directory, and a plain `claude` no longer loads the system prompt. Run the printed `cd`, and start sessions with `pb cc` or `playbook cc`. ([#1120](https://github.com/pragmatic-engineer/playbook/pull/1120))
- **setup**: `setup --yes` now installs the system prompt and links `pb`. Pass `--no-system-prompt` or `--no-short-name` to keep them off. ([#1122](https://github.com/pragmatic-engineer/playbook/pull/1122))

### Features

- **setup**: link pb from the short-name part ([#1118](https://github.com/pragmatic-engineer/playbook/pull/1118))
- **setup**: remove the retired launcher rc line with consent ([#1121](https://github.com/pragmatic-engineer/playbook/pull/1121))

### Other changes

- docs: describe pb cc without the shell launcher ([#1123](https://github.com/pragmatic-engineer/playbook/pull/1123))

## playbook 0.36.0

### Features

- **setup**: turn off Claude Code attribution with consent ([#1101](https://github.com/pragmatic-engineer/playbook/pull/1101))
- **usage**: add a launchd and systemd service for the collector ([#1104](https://github.com/pragmatic-engineer/playbook/pull/1104))
- **cli**: install, start and remove the collector service ([#1108](https://github.com/pragmatic-engineer/playbook/pull/1108))
- **setup**: keep the usage collector running in the background ([#1105](https://github.com/pragmatic-engineer/playbook/pull/1105))
- **migrate**: remind existing installs once about the collector service ([#1107](https://github.com/pragmatic-engineer/playbook/pull/1107))

### Fixes

- **eval**: find agent files under plugins/playbook ([#1096](https://github.com/pragmatic-engineer/playbook/pull/1096))
- **ci**: run the repo checks on the plugins/playbook layout ([#1095](https://github.com/pragmatic-engineer/playbook/pull/1095))
- **cc**: clear inherited agent variants before a launch ([#1102](https://github.com/pragmatic-engineer/playbook/pull/1102))
- **pbtop**: say why a telemetry view is empty and how to fix it ([#1106](https://github.com/pragmatic-engineer/playbook/pull/1106))
- **auto**: raise the default spend caps to measured session cost ([#1115](https://github.com/pragmatic-engineer/playbook/pull/1115))

### Other changes

- chore(cli): remove code nothing calls ([#1092](https://github.com/pragmatic-engineer/playbook/pull/1092))
- docs: list every ADR in the index and fix stale names ([#1097](https://github.com/pragmatic-engineer/playbook/pull/1097))
- build(toolchain): pin Rust 1.99.1 ([#1098](https://github.com/pragmatic-engineer/playbook/pull/1098))
- chore(deps): bump patch versions and refresh the lockfile ([#1094](https://github.com/pragmatic-engineer/playbook/pull/1094))
- chore(settings): use current tool names in the shared allowlist ([#1099](https://github.com/pragmatic-engineer/playbook/pull/1099))
- test(memory): read the touched time the staleness check reads ([#1090](https://github.com/pragmatic-engineer/playbook/pull/1090))
- docs(plugin): stop telling agents to call TaskOutput ([#1100](https://github.com/pragmatic-engineer/playbook/pull/1100))
- chore: fix gaps a review found in the sweep PRs ([#1103](https://github.com/pragmatic-engineer/playbook/pull/1103))
- test: stop waiting out the migrate lock budget ([#1109](https://github.com/pragmatic-engineer/playbook/pull/1109))

## playbook 0.35.0

### Breaking changes

- **plugin**: give /playbook:setup the flags and questions of setup ([#1073](https://github.com/pragmatic-engineer/playbook/pull/1073))
- /playbook:setup --yes now leaves the launcher and the ([#1077](https://github.com/pragmatic-engineer/playbook/pull/1077))
- **telemetry**: the playbook-telemetry command is gone. Run pbtop with ([#1082](https://github.com/pragmatic-engineer/playbook/pull/1082))

### Features

- **cli**: add playbook setup and deprecate init ([#1064](https://github.com/pragmatic-engineer/playbook/pull/1064))
- **plugin**: add Python engineering standards skill ([#1066](https://github.com/pragmatic-engineer/playbook/pull/1066))
- **plugin**: add the remaining Python standards references ([#1067](https://github.com/pragmatic-engineer/playbook/pull/1067))
- **cli**: name playbook setup in every message and hint ([#1068](https://github.com/pragmatic-engineer/playbook/pull/1068))
- **install**: install pbtop alone with --pbtop ([#1072](https://github.com/pragmatic-engineer/playbook/pull/1072))
- **install**: add install-pbtop.sh as the pbtop entry point ([#1078](https://github.com/pragmatic-engineer/playbook/pull/1078))
- **cli**: read release-notes.json in playbook update ([#1079](https://github.com/pragmatic-engineer/playbook/pull/1079))
- **cli**: print release notes line on first run after upgrade ([#1083](https://github.com/pragmatic-engineer/playbook/pull/1083))
- **install**: print the release notes when replacing a binary ([#1084](https://github.com/pragmatic-engineer/playbook/pull/1084))

### Fixes

- **cli**: finish the review of the setup naming change ([#1076](https://github.com/pragmatic-engineer/playbook/pull/1076))

### Other changes

- docs: finish the init to setup rename ([#1070](https://github.com/pragmatic-engineer/playbook/pull/1070))
- ci(release): run every release step from one workflow ([#1071](https://github.com/pragmatic-engineer/playbook/pull/1071))

## playbook 0.34.0

The first release with one version for the binary and the plugin. The binary goes from 0.26 to 0.34.0, so binary versions 0.27 to 0.33 do not exist.

### Features

- **telemetry**: start the collector from SessionStart, add doctor rows ([#1015](https://github.com/pragmatic-engineer/playbook/pull/1015))
- **review**: follow Conventional Comments in review skills ([#1021](https://github.com/pragmatic-engineer/playbook/pull/1021))
- **telemetry**: add the pbtop layout editor ([#1018](https://github.com/pragmatic-engineer/playbook/pull/1018))
- **review**: add review suggest for a quick or deep call ([#1022](https://github.com/pragmatic-engineer/playbook/pull/1022))
- **telemetry**: add a projection panel to pbtop ([#1026](https://github.com/pragmatic-engineer/playbook/pull/1026))
- **telemetry**: add usage doctor to list limit sources ([#1028](https://github.com/pragmatic-engineer/playbook/pull/1028))
- **review**: add /playbook:review to pick a quick or deep review ([#1027](https://github.com/pragmatic-engineer/playbook/pull/1027))
- **telemetry**: read cached limits and plan in pbtop ([#1031](https://github.com/pragmatic-engineer/playbook/pull/1031))
- **cli**: record versions and heal owned files at session start ([#1037](https://github.com/pragmatic-engineer/playbook/pull/1037))
- **cli**: add doctor --fix and heal after playbook update ([#1038](https://github.com/pragmatic-engineer/playbook/pull/1038))
- **cli**: add migrations for the 0.25 to 0.26 upgrade gap ([#1039](https://github.com/pragmatic-engineer/playbook/pull/1039))
- **cli**: check for newer releases and print a notice ([#1043](https://github.com/pragmatic-engineer/playbook/pull/1043))
- **cli**: add playbook migrate as the front door to migrations ([#1046](https://github.com/pragmatic-engineer/playbook/pull/1046))
- **cli**: add the auto update that installs a newer release ([#1047](https://github.com/pragmatic-engineer/playbook/pull/1047))
- **install**: show breaking changes when replacing a binary ([#1049](https://github.com/pragmatic-engineer/playbook/pull/1049))
- **cli**: heal once on the first run of a new binary ([#1051](https://github.com/pragmatic-engineer/playbook/pull/1051))
- **plugin**: check the breaking changes mirror in plugin.json ([#1052](https://github.com/pragmatic-engineer/playbook/pull/1052))
- **cli**: turn on update.mode auto in the release check ([#1050](https://github.com/pragmatic-engineer/playbook/pull/1050))
- **cli**: show breaking changes in playbook update ([#1048](https://github.com/pragmatic-engineer/playbook/pull/1048))
- **update**: show the release check status and add ADR 0036 ([#1054](https://github.com/pragmatic-engineer/playbook/pull/1054))
- **cli**: show breaking changes once when a new binary heals ([#1055](https://github.com/pragmatic-engineer/playbook/pull/1055))
- **plugin**: add Rust engineering standards skill ([#1060](https://github.com/pragmatic-engineer/playbook/pull/1060))
- **plugin**: add the remaining Rust standards references ([#1061](https://github.com/pragmatic-engineer/playbook/pull/1061))
- **release**: ship the binary and the plugin as one release line

### Fixes

- **update**: stop warning that the plugin differs from the binary ([#1023](https://github.com/pragmatic-engineer/playbook/pull/1023))
- **plugin**: stop using a bare plugin root path ([#1029](https://github.com/pragmatic-engineer/playbook/pull/1029))
- **plugin**: hint to upgrade an old binary ([#1032](https://github.com/pragmatic-engineer/playbook/pull/1032))
- **plugin**: find the plugin root with one resolver ([#1033](https://github.com/pragmatic-engineer/playbook/pull/1033))
- **plugin**: add path plugin-root and fallbacks for an old setup ([#1035](https://github.com/pragmatic-engineer/playbook/pull/1035))
- **doctor**: check the plugin against the supported version range ([#1042](https://github.com/pragmatic-engineer/playbook/pull/1042))
- pass the breaking notice flag only to the steps that read it ([#1056](https://github.com/pragmatic-engineer/playbook/pull/1056))
- **cli**: hide only the breaking entries the caller already showed ([#1058](https://github.com/pragmatic-engineer/playbook/pull/1058))
- **release**: address review findings for one release line

### Other changes

- docs(telemetry): document telemetry on by default and add ADR 0033 ([#1019](https://github.com/pragmatic-engineer/playbook/pull/1019))
- docs(plugin): add a Conventional Commits reference ([#1025](https://github.com/pragmatic-engineer/playbook/pull/1025))
- docs(telemetry): describe tabs, limit sources and usage doctor ([#1034](https://github.com/pragmatic-engineer/playbook/pull/1034))
- test(telemetry): give each collector test its own port ([#1036](https://github.com/pragmatic-engineer/playbook/pull/1036))
- docs(adr): add ADR 0035 and the breaking changes record ([#1041](https://github.com/pragmatic-engineer/playbook/pull/1041))
- docs(plugin): match doctor.md to the new layer 6 checks ([#1044](https://github.com/pragmatic-engineer/playbook/pull/1044))
- docs(plugin): describe playbook migrate in the doctor command ([#1053](https://github.com/pragmatic-engineer/playbook/pull/1053))
- test(cli): ignore SQLite sidecars in the smoke idempotence check ([#1057](https://github.com/pragmatic-engineer/playbook/pull/1057))

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
