# ADR-0030: Keep the Skills, Commands and Agents in Their Own Repository

- **Status:** Accepted
- **Date created:** 2026-10-13
- **Date modified:** 2026-10-13

## Context

One repository held two things with different lifecycles. The `playbook` binary and its hooks are code, versioned and released as binaries. The skills, commands and agents are prose that Claude Code loads. A prose edit needed a binary release to reach the marketplace, and a binary release carried unchanged prose. The tests tied the two together: about forty test files read the prose and asserted that it named the binary's subcommands.

The prose is also useful without the binary. A person who installs only the plugin gets skills that name commands they do not have. ADR 0029 kept the plugin root at the repository root. That no longer fits.

## Decision

1. **A second repository, `pragmatic-engineer/skills`.** It holds the `playbook` plugin (skills, commands, agents, output styles) under `plugins/playbook/`, and the `pragmatic-engineer` marketplace. It is the former `pragmatic-engineer/marketplace` repository, renamed, so the marketplace history stays. The playbook history of the moved paths was imported into it with `git filter-repo`, so `git log` there shows the work.
2. **This repository keeps the binary and the hooks.** The `playbook-hooks` plugin (hooks, the `bin/playbook` shim, `install.sh`, the seed files) is versioned with the binary and ships as `playbook-hooks-plugin-<version>.zip` on the release.
3. **The marketplace lists both.** `playbook` points at a path inside the skills repository. `playbook-hooks` points at the release archive in this repository with its sha256. A workflow in the skills repository pins the archive after each release. Install ids and the marketplace name stay `playbook@pragmatic-engineer` and `playbook-hooks@pragmatic-engineer`.
4. **The plugin sits in a subdirectory.** Claude Code copies the whole source directory into its cache. A plugin at the repository root would copy the checks, the workflows and the docs with it, so the source is `./plugins/playbook`.
5. **The prose stays free of the binary.** The skills name what they need in plain words. The binary offers a map of tasks to commands (`playbook tools map`). The skills repository lists the intents it relies on in `intents.json`, and a daily workflow compares that list with the latest released binary. A missing intent is a warning, and a failure only for an entry marked `required`. None is marked.
6. **Prose tests move with the prose.** Tests that only read the plugin files run in a small crate in the skills repository. Tests that asserted prose against binary tables were dropped, because the contract check replaces them.
7. **The skills plugin has its own version line.** It continues from 0.26.0 and releases on its own tag.

## Limits that remain

- The binary still reads some skills plugin files at run time (`playbook pr rules`, the effort commands and the `playbook cc` variants). It finds them through the installed skills plugin and not through the hooks plugin root, and each command works with no plugin files at all: `pr rules` prints an error the skills treat as a cue to read the files themselves, and the effort commands report a component as unknown. `playbook skill ref` and `playbook review ref` are gone.
- The authoring guides and the policy table moved with the prose, and the pages that link across the two repositories use absolute URLs.
- Commit hashes of the moved paths changed with the history rewrite, and the rewritten commits are not signed.

## Alternatives considered

- **Keep one repository and split releases.** Rejected: tag prefixes already separate tools, but the prose would still need the binary's CI and review rules.
- **Git submodule for the plugin.** Rejected: the same reasons as in ADR 0029.
- **Pin the skills plugin to a release archive too.** Rejected: a path inside the marketplace repository needs no archive, no hash and no pin pull request.

## Consequences

- Prose changes ship without a binary release, and the binary ships without touching prose.
- Two repositories to keep in step. The contract workflow and the install guide are the link.
- Fresh installs add the marketplace from `pragmatic-engineer/skills`. An existing marketplace entry keeps working through the repository rename redirect.
