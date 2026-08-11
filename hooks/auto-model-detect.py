#!/usr/bin/env python3
# SPDX-FileCopyrightText: 2026 Igor Santos
# SPDX-License-Identifier: MIT
# UserPromptSubmit hook: detect design/architecture intent in the prompt and
# nudge Claude (the main loop) to delegate the heavy thinking to an Opus
# subagent instead of doing it inline on Sonnet.
#
# Why not auto-switch the model? Claude Code's session model is set at session
# start (or via /model). A hook can't flip it mid-stream. But it *can* push
# Claude toward Agent(model: opus) invocations of design-oriented subagents.

import os
import re
import sys

sys.path.insert(0, os.path.join(os.path.dirname(os.path.abspath(__file__)), "lib"))
import common as c  # noqa: E402

# Word-boundary patterns. Case-insensitive. Tuned for ADR / design work:
#   - explicit nouns: design, architecture, ADR, schema, tradeoff
#   - decision verbs: evaluate, compare, decide, choose between, plan, propose
#   - design-shaped questions: "should we", "how would you", "what's the best"
INTENT_RE = re.compile(
    r"(\b(design|architect|architecture|ADR|tradeoffs?|alternatives?|approach|"
    r"strategy|paradigm|pattern|abstraction|refactor plan|migration|decompos|"
    r"schema|modeling|data model|contract|interface design)\b|"
    r"\b(evaluate|compare|brainstorm|propose|recommend|critique|"
    r"review the approach|review the design)\b|"
    r"\b(should we|how (would|should) (we|you|i)|what.?s the best|"
    r"which (approach|design|pattern)|trade ?off|pros and cons)\b)",
    re.IGNORECASE,
)

MSG = """This prompt looks like design / architecture work. Your main session runs on the default model. Before reasoning inline, consider delegating to an Opus subagent, its full deliberation stays in the subagent's context, only the conclusion returns to yours.

Recommended for design-heavy prompts:
  - Plan (Agent tool, `model: "opus"`), implementation planning and architecture with codebase grounding
  - superpowers:brainstorming (Skill tool), ideation / requirements before any code

If the prompt is actually small-scope (e.g. quick choice between two named options), staying on Sonnet inline is fine. Use judgment.

Routing policy: Opus only when Sonnet wasn't enough, keep Opus under 20% of total usage. Routine/mechanical/formatting/search subagents default to Haiku (3x cheaper); escalate to Sonnet for real coding."""


def main() -> int:
    prompt = c.field(".prompt")
    if not prompt:
        return 0

    # Skip slash commands, those are explicit user intents, not natural prose.
    if prompt.startswith("/"):
        return 0

    # Skip very short prompts, usually confirmations / one-word redirects.
    if len(prompt) < 20:
        return 0

    if not INTENT_RE.search(prompt):
        return 0

    c.emit_prompt_context(MSG)
    return 0


if __name__ == "__main__":
    sys.exit(main())
