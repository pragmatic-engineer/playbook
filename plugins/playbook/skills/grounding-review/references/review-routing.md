# Review routing

How `/playbook:review` picks a quick or a deep review. Load this file when the `review-suggest` tool is not in the session's tool map (Part 1), and when the answer comes back with low confidence (Part 2). With the tool and a high confidence answer, you do not need it.

## Part 1: the rules by hand

These are the same rules `playbook review suggest` applies. Use them in order and stop at the first class that has a hit.

Gather the facts first. For a PR: `gh pr view <n> --json files,labels,author` (each file has `path`, `additions`, `deletions`) and `gh api /user -q .login` to see whether the PR is yours. For a local branch: `git diff -M --numstat origin/<default>...HEAD`. Add the added and deleted lines of each file.

Sort every changed file into one kind:

- **Docs:** `.md`, `.mdx`, `.txt`, `.rst`, images, anything under `docs/`. Plugin commands, skills, agents, hooks and `prompts/` are not docs: they change what the plugin does. Dependency manifests such as `requirements.txt` are not docs.
- **Lock files:** `Cargo.lock`, `package-lock.json`, `yarn.lock`, `pnpm-lock.yaml`, `poetry.lock`, `Gemfile.lock`, `go.sum`, `uv.lock`, `composer.lock`, `Pipfile.lock`.
- **Tests:** paths under `test/`, `tests/`, `__tests__/`, `spec/` or `fixtures/`, and names ending `_test.*`, `_spec.*`, `.test.*` or `.spec.*`.
- **Plugin files:** commands, skills, agents and hooks of a plugin, and `prompts/`.
- **Code:** everything else.

Size means the changed lines of code, tests and plugin files. Docs and lock file lines do not count. Packages means the distinct folders under `crates/`, `packages/`, `apps/`, `services/`, `libs/` and `plugins/` that hold a file which is not docs or a lock file. All other paths count together as one package.

The cuts, with their config keys (read them with the `config-get` intent; use the default when it is not set):

| Cut | Default | Config key |
|---|---|---|
| Large change | more than 400 lines | `review.suggest.deepLines` |
| Small change | at most 60 lines | `review.suggest.quickLines` |
| Wide change | more than 25 files | `review.suggest.deepFiles` |
| Cross-cutting | 3 or more packages | `review.suggest.packageSpan` |
| Untested code | 150 or more lines | `review.suggest.untestedLines` |

The classes, in order:

1. **Security label.** The PR has a label with `security` in its name: deep, high confidence.
2. **Changes that cannot alter behavior:** quick, high confidence, when any of these holds.
   - Only lock files changed.
   - Only docs changed.
   - Only tests changed.
   - Only docs, tests and lock files changed, in any mix.
   - Files were only moved or renamed with no edited line, and none of them is a risky path (below).
3. **Risks:** deep, high confidence, when any of these holds.
   - Large change, wide change, or cross-cutting.
   - A path for auth, crypto, secrets, credentials, passwords, permissions, `.env`, `.pem` or `.key` files.
   - A CI workflow under `.github/workflows/` or `.github/actions/`.
   - A release file (a name with `release` in it).
   - A hooks or guards folder, or a file with `guard` in its name.
   - A migration, a schema or a `.sql` file.
   - A dependency manifest such as `Cargo.toml`, `package.json`, `go.mod` or `pyproject.toml`.
   - The command surface: `main.*`, `cli.*`, `command.rs`, `cmd/`, `bin/`, `intents.json`, `.proto` or `openapi` files.
   - Plugin files with more changed lines than the small cut.
   - The untested code count or more changed lines of code outside Rust files, with fewer than one test line for every nine code lines. Rust files do not count, because they hold their tests inline.
4. **Small change:** at most the small cut and no risk: quick, high confidence.
5. **Close call:** none of the above. Confidence is low. Lean quick when the change is yours, deep when it is someone else's.

Report at most four reasons, each one short plain sentence, such as "Changes CI workflows." or "Large change: 523 changed lines."

## Part 2: the low confidence pass

A low confidence answer means no rule was decisive. Ask one cheap read for a second opinion, once, and use it.

Dispatch one `playbook:review-triage` agent (the `delegating-subagents` skill picks the agent under the user's ceiling). Its prompt holds:

- One candidate lens named `deep-swarm`, described as "does this change need the full reviewer swarm: logic, security, tests, data, types and performance specialists".
- The reasons and signals from the rule list.
- The diff when it is under about 800 changed lines. For a bigger one, give `git diff --stat` and the paths of the largest files, and the checkout path so the agent can read them. Never paste a very large diff.

Read the tier it returns for `deep-swarm`:

- `full-lens` means deep.
- `cheap-check` or `skip` means quick.

Use that answer as the recommendation and keep the reason it gives, in one short line. If the dispatch fails, returns nothing, or returns something you cannot read, keep the lean from the rule list and say that the second opinion was not available. A failed pass never blocks the review and never lowers it to quick on its own.

## Part 3: asking

In ask mode, put the recommended depth first and mark it `(Recommended)`, with the reasons in one short line:

```
Which review?
1. Deep review (Recommended): <reasons, one line>
2. Quick review: one reviewer, one pass
```

The user can answer with the number, `quick` or `deep`, or press enter for the recommendation. Whatever they pick wins.
