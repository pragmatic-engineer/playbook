# ADR 0013 Blueprint: Org-level memory scoping

One Work Unit: the change is small (three existing match sites, one path-parsing function, two docs), tightly coupled (a scope value only means anything once storage, graph-build, and both read paths agree on it), and better verified as one coherent change than split.

## WU-1: Add `Scope::Org`

**Files:**
- `src/hooks/rebuild_memory_graph.rs` (the `Scope` enum, `scope_and_project`, `node_id`, the link-resolution match)
- `src/hooks/memory_anchors.rs` (`in_scope`)
- `src/hooks/session_init.rs` (`in_promotion_scope`)
- `shell/memory-context.sh` (a third, independent scope match in jq, not caught by the issue's own investigation: the SessionStart slice filters scope itself rather than going through Rust's `in_scope`)
- `docs/concepts/02-memory-system.md`
- `prompts/SYSTEM_PROMPT.md`

**Changes:**

1. `rebuild_memory_graph.rs`: add `Scope::Org` to the enum and its `as_str` (`"org"`). Change `scope_and_project`'s split from a 2-way (`>= 3` vs else) to a 3-way: 1 segment → `(Global, None)`; exactly 2 segments → `(Org, Some(parts[0].to_string()))`; 3+ segments → `(Project, Some("owner/repo"))` (unchanged). `node_id` and the link-resolution match both get an `Org` arm mirroring `Project`'s existing shape, keyed on the same `project: Option<&str>` parameter (it already just means "the resolved scope key," no new parameter needed).

2. `memory_anchors.rs`'s `in_scope` and `session_init.rs`'s `in_promotion_scope`: add `Some("org") => node.get("owner").and_then(Value::as_str) == Some(current_owner)`, where `current_owner` is `repo_slug().split('/').next()` (empty string when not in a git repo, matching the existing empty-string-means-no-match behavior for `repo`).

3. Precedence: extend the existing Pass 2 link-resolution fallback (own scope first, then global) to also cover `Org`, same pattern as `Project` already uses (implemented as one shared match arm, since the logic is identical). No render-order change: neither injection path (`append_promoted_facts`, `memory-context.sh`) sorts by scope today, both sort by fact name; org facts join that same unordered-by-scope rendering.

4. `docs/concepts/02-memory-system.md`: document the third scope, its storage path (`memory/<owner>/*.md`), and the global → org → project precedence.

5. `prompts/SYSTEM_PROMPT.md`'s "Where to save" (line 61 as of this writing): change the binary question to a three-way branch: repo-specific → project; useful across every repo under one owner but not truly universal → org (`memory/<owner>/`, create the owner directory and its `MEMORY.md` on first save the same way a project subfolder is created today); otherwise → global.

**Test scenarios:**

- An org-scoped node (`scope: "org"`, `owner: "acme"`) is in scope for repo `acme/widget` and `acme/gadget`, out of scope for `other-org/thing`.
- A 2-segment relative path (`acme/fact.md`) builds an `Org` node at graph-rebuild time, not `Global`.
- A 3-segment path (`acme/widget/fact.md`) still builds a `Project` node (regression: the existing split must not shift).
- A 1-segment path (`fact.md`) still builds a `Global` node (regression).
- A `links: relates_to` edge from an org-scoped fact resolves in its own org scope first, falling back to global when no org-scoped target exists (same pattern already covered for project-scoped sources).
- `in_promotion_scope` and `in_scope` agree on the same node (same test fixture through both, since they're deliberately duplicated logic, per `feedback_code_comments_stay_short`-adjacent convention already established for this pair).

**Done when:** `cargo build` and `cargo test` clean, the four scope-classification scenarios above pass, and `docs/concepts/02-memory-system.md` / `prompts/SYSTEM_PROMPT.md` describe the third tier accurately.

**Verify:** `cargo test --lib hooks::rebuild_memory_graph hooks::memory_anchors` plus the relevant `tests/hooks_session.rs`/`tests/hooks_graph_reader.rs` integration tests.
