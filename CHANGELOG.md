# Changelog

Release notes for the playbook plugin in `plugins/playbook`. The binary has its own releases. Each plugin release has a section named `## playbook <version>`, newest first, and the top section always matches the version in `plugins/playbook/.claude-plugin/plugin.json`.

## playbook 0.27.3

- The install command in the doctor hint now names pragmatic-engineer/marketplace, the new name of the marketplace repository.
- The plugin manifest points at the playbook repository and no longer mentions the playbook-hooks plugin.

## playbook 0.27.2

- The plugin now lives in this repository. It moved from pragmatic-engineer/skills at commit a27dbe7c2c996b9d01008eb080edbc7dac639b3b, where its history stays readable.
- No change to the skills, commands, agents or output style.
