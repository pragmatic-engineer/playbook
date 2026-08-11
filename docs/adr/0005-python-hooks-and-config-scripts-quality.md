# ADR 0005 Quality Gate Report

- **Parent ADR:** `docs/adr/0005-python-hooks-and-config-scripts.md`
- **Blueprint:** `docs/adr/0005-python-hooks-and-config-scripts-blueprint.md`
- **Date:** 2026-08-11

## Quality Gate Result

**Fact-Check:**        PASS (16/16 cited paths exist, 14/14 hooks present, jq counts 12/10/5 match the ADR)
**Adversarial Review:** PASS (2 WARN)
**Test Review:**       PASS (1 WARN)

Mechanism note: the fact-check ran deterministically through the shell (path existence, hook inventory, jq-count claims, a dash check). The adversarial and test phases were a fresh critical read, consistent with the precedent in `docs/adr/0003-...-quality.md` and `0004-...-quality.md` where the reviewer swarm proved unreliable this session.

## Verification Summary

| Referenced path | Confirmed? | Where used |
|---|---|---|
| The 3 guards (`rm-workspace`, `no-dash`, `bg-await`) | Yes | stay bash |
| The 11 migrating hooks | Yes | S2, S3 |
| `hooks/lib/common.sh`, `hooks/hooks.json` | Yes | WU-4, all hook WUs |
| `shell/{merge,check-shared,gen-shared}-settings.sh` + tests | Yes, jq 12/10/5 | S1 |
| `install.sh`, `shell/setup-local.sh`, `.github/workflows/shell-ci.yml` | Yes | callers |
| `docs/authoring/01-commands-skills-hooks.md` | Yes | WU-16 |

## WARNs (informational, not blocking)

- **Adversarial: mixed-language `hooks/`.** After the migration, `hooks/` holds mostly python plus three bash guards, and both `common.sh` and `common.py` exist. That is a real cost, but the alternative (all-python guards) trades the safety layer's fail-safe speed for uniformity, which the decision rejects on purpose. WU-16 documents the split so it does not confuse a future author.
- **Adversarial: the hot-path latency is accepted, not eliminated.** Every migrated non-guard hook pays python startup per fire. The decision accepts this for the non-guard hooks and requires a before-and-after timing in WU-16 so a bad regression is visible. If the `PostToolUse` pair regresses badly, D (leave as shell) is the per-hook fallback.
- **Test: python test discovery is undecided.** CI finds `*.test.sh`; python tests need either thin `*.test.sh` wrappers that invoke python or a new `*_test.py` runner in `shell-ci.yml`. WU-1 must pick one and every later WU must follow it. Pinned as the first open item.

## Structural Checks

- [x] 4 alternatives (A chosen, B/C/D rejected) each with effort and trade-offs.
- [x] The Decision explains why B, C, and D lost.
- [x] Work units have file plans with real paths; new files marked new.
- [x] Verification commands are literal.
- [x] Open items are deferred to named WUs (test discovery to WU-1, latency to WU-16, the hooks.json shared-file rule to integration).
