# ADR 0013: Org-level memory scoping

- **Status:** Accepted
- **Date:** 2026-09-04
- **Date modified:** 2026-09-04

## Context

Issue #318, raised during the ADR-0012 design session, asks for facts shared across every repo under one owner (e.g. all `pragmatic-engineer/*` repos), not just the two scopes the memory system has today.

The memory system currently hardcodes exactly two scope values, `global` and `project`, documented at `docs/concepts/02-memory-system.md:3` and matched identically in four places, one more than issue #318's own investigation found:

- `src/hooks/memory_anchors.rs:513-518`'s `in_scope`
- `src/hooks/session_init.rs:307-313`'s `in_promotion_scope` (a deliberate duplicate of `in_scope`, not a shared function, per this codebase's established per-module-duplication convention)
- `src/hooks/rebuild_memory_graph.rs:410-413`'s `Scope` enum, the source of truth both Rust callers derive their node's `scope` field from at graph-build time
- `shell/memory-context.sh`'s own `in_scope` jq filter, a third independent implementation: the SessionStart slice filters scope directly in jq rather than going through either Rust function above

`Scope` is derived from path shape, not from a fact's own frontmatter, in `rebuild_memory_graph.rs:426-433`'s `scope_and_project`:

```rust
fn scope_and_project(rel: &str) -> (Scope, Option<String>) {
    let normalized = rel.replace('\\', "/");
    let parts: Vec<&str> = normalized.split('/').collect();
    if parts.len() >= 3 {
        (Scope::Project, Some(format!("{}/{}", parts[0], parts[1])))
    } else {
        (Scope::Global, None)
    }
}
```

A relative path with 3+ segments (`owner/repo/fact.md`) is project-scoped; anything shorter (`fact.md`, or `owner/fact.md`) is global. This is the concrete technical constraint this ADR has to resolve: naively storing an org fact at `memory/<owner>/fact.md` (2 segments) would be silently classified `Global` by the current logic, not a new `Org` scope, since nothing today distinguishes "a global fact that happens to live one directory deep" from "an org-scoped fact." `docs/adr/0012-unify-state-under-config-playbook.md`'s Consequences section already confirmed the `<owner>/<repo>/` nesting is compatible with an org tier sitting between them; this ADR is where that gets designed for real, per that ADR's own explicit deferral.

Issue #318 named a second, distinct need: cross-repo scoping across an explicit, non-hierarchical set of repos (not necessarily sharing one owner, not literally every repo). That doesn't fit the owner/repo directory hierarchy at all, and the issue itself notes the existing `links: relates_to` graph edge (`docs/concepts/02-memory-system.md:61`) may already cover some of what would otherwise need it. This ADR resolves org-level scoping only; cross-repo scoping across an arbitrary named set of repos is out of scope here and tracked as a follow-up (see Consequences), since it has no clear storage model yet and doesn't have a concrete use case pushing on it the way org-level scoping does.

## Decision Drivers

- **Minimal new surface.** The memory system already has three scope-matching call sites and a path-based scope inference function; a new scope should extend that shape, not replace it or add a second parallel mechanism.
- **No silent misclassification.** An org fact must never be classified `Global` (over-broad injection into repos outside the org) or `Project` (under-broad, invisible to sibling repos) by accident.
- **Conflict precedence must extend cleanly.** `docs/concepts/02-memory-system.md:61` already defines link resolution (own scope first, then a fallback to global) and contradiction handling (a more specific fact wins) for the existing two tiers. A third tier needs the same rule extended, not a special case.
- **Don't block on cross-repo scoping.** The org-level need is concrete (issue names a specific example: shared facts across `pragmatic-engineer/*`); the cross-repo need is still undesigned per the issue's own text. Coupling them risks shipping neither.

## Alternatives

### A. Add `scope: org`, keyed by directory depth (chosen)

Store org facts at `memory/<owner>/*.md`, sibling to the existing `memory/<owner>/<repo>/*.md` project tier. Change `scope_and_project` to a three-way split on segment count: 1 segment → `Global`, exactly 2 → `Org` (capturing the owner), 3+ → `Project` (unchanged). Add `Scope::Org` to the enum. Extend `in_scope`/`in_promotion_scope` (both call sites) with `Some("org") => node.get("owner") == Some(current_owner)`, where `current_owner` is the first segment of the already-resolved `repo_slug()`.

- Effort: S. Touches 3 files with existing match arms, one path-parsing function, no new storage location, no migration (an org directory simply doesn't exist until the first org fact is saved there).
- Precedence: link resolution checks a fact's own scope first, then falls back to global, extended to also fall back from org (unchanged pattern, one more scope to try in the same order). Contradiction handling extends from "project wins over global" to "project wins over org wins over global," matching specificity.
- Risk: a fact author saving at the wrong depth (e.g. a typo'd path) gets silently reclassified rather than erroring. Mitigated the same way the existing global/project split already is: `/playbook:learn-project` and the save-fact flow in `SYSTEM_PROMPT.md` construct the path programmatically, a human is not hand-typing `memory/<owner>/<repo>/fact.md` paths directly.

### B. Add `scope: org` via explicit frontmatter, not path depth

Keep every fact under its current path shape and add an `owner:` frontmatter field read at graph-build time to distinguish org from global, instead of inferring from directory depth.

- Effort: M. Requires a storage-location decision anyway (where does an org fact's `.md` file physically live if not `memory/<owner>/`?), so this doesn't actually avoid the directory question Alternative A answers directly; it just moves the same decision into frontmatter parsing plus a new required field with its own validation and failure mode (missing/wrong `owner:` on a real org fact).
- Rejected: strictly more surface than Alternative A for the same outcome, no compensating benefit identified.

### C. No new scope; simulate org-level via `relates_to` edges across per-repo duplicate facts

Save the same fact once per repo under existing `project` scope, linked via `relates_to`.

- Effort: S to add, but O(n) duplication per org-wide fact and O(n) edit cost every time the fact changes (every sibling copy needs updating, with no mechanism to detect drift between copies).
- Rejected: doesn't solve the problem, it works around not having org scope, at a cost the issue's own text already identifies as unacceptable ("does not fit... at all" for the cross-repo case, and org-level has the same duplication problem if approached this way).

## Decision

**Alternative A.** Add `Scope::Org`, keyed by exactly-2-segment paths under `memory/<owner>/`. Precedence global → org → project. Cross-repo (named, non-hierarchical) scoping is explicitly deferred; see Consequences.

## Consequences

- `docs/concepts/02-memory-system.md` gets a third scope documented alongside global/project, plus the new `memory/<owner>/*.md` path shape.
- `prompts/SYSTEM_PROMPT.md`'s Memory section (already covers where to save global vs project facts) gets an org-scope save rule: "is this fact useful across every repo under one owner, but not truly universal?"
- `graph.json`'s (`memory.graph.json`'s) node shape needs no new field: the existing `project` field carries `owner` for org nodes and `owner/repo` for project nodes, discriminated by the `scope` field a reader already has to check first.
- Cross-repo (arbitrary, non-hierarchical, not-necessarily-same-owner) scoping remains undesigned. If a concrete need surfaces, it gets its own ADR; this one does not block on it, and nothing decided here forecloses a future design (a cross-repo mechanism would layer on top of, not replace, org/global/project).
- No migration needed: org scope is purely additive, existing global and project facts are unaffected, and the `memory/<owner>/` directory does not need to exist until the first org fact is saved there.
