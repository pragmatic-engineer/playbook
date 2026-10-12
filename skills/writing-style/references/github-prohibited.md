# Prohibited GitHub Content

Read before posting anything to GitHub, including a PR title or body. It extends SKILL.md.

Patterns that flag content as non-human. NEVER appear in GitHub-posted content:

1. **Git commit hashes/SHAs:** never reference commits by hash (e.g., "In f661154", "fixed in abc123"). Humans don't cite commit hashes in conversation.
2. **Escaped apostrophes in double-quoted strings:** `"don\'t"` posts as `don'''t`. Just write `"don't"`.
3. **Paraphrasing openers:** never echo the reviewer's point in a noun phrase ("Valid concern on the behavioural inconsistency", "Great observation about the null handling"). Humans say "Ok.", "Fair point.", "Yeah.", "My bad." then move to substance.
4. **Finding numbers (1, 2, 3) or coloured circle emojis** in posted content.
5. **Formatted section headers** like "File-Level Findings" or "Summary" grouping multiple findings under one banner, in posted content (these are local report artifacts). This does not apply to the bare per-comment label (`blocking:`, `issue:`, `suggestion:`, `nitpick:`, `question:`) that starts each individual finding.
6. **"Fix:" prefix on suggestions** when posted to GitHub (sounds robotic).
7. **Bullet-point summaries of findings** (that's what inline comments are for).
8. **Emojis** unless the project convention includes them.
9. **LLM hedging phrases:** "if you feel strongly", "I can see the trade-off", "that's a fair point". Too deferential. State your position.
10. **Praise openers + restate-the-finding pattern:** "Good catch.", "Nice catch.", "Great catch.", "You're right.", "Great point." Fix the thing and move on.
11. **Restating the reviewer's finding in your reply when a code fix landed.** Diff is the explanation. Prose summary of the fix is an AI tell.
