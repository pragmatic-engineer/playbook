---
name: writing-style
description: Use when writing prose for human readers, like PR descriptions, code review comments, tickets, ADRs, Confluence pages, Slack messages, or commit messages.
---

# Writing Style

Universal writing rules for all generated content intended for human readers: PR descriptions, review comments, ticket descriptions, ADRs, Confluence pages, Slack messages, Jira tickets.

These rules are MUST-level unless marked SHOULD.

> **IRON RULE:** MUST NEVER use em dashes or en dashes in any output, and MUST write in simple, plain English (common words, short sentences). This applies to every file, comment, commit message, and generated text. Use commas, colons, or periods instead of dashes. Both dash types are strong AI tells; fancy vocabulary and long sentences are too.

## Reviewer usability (adapted from "Don't Make Me Think")

Adopt these principles to what you write for review: the reviewer is the user, the text is the interface, a question in their head is the cost. Reviewers scan, they don't read.

- Commit messages: conventional imperative subject under ~50 chars, one change. Add a body only when the why isn't obvious; a sentence or two, or tight bullets.
- PR descriptions: one line of what and why is the target, not the floor. Add a short bulleted "what changed" only when the diff needs it to be reviewable; link to detail instead of inlining it. Aim for the gist in under 30 seconds, most PRs need fewer words than feel natural to write.
- Review findings: default to one sentence, two at most (the problem, then what breaks). A paragraph is the rare exception for a genuinely non-obvious mechanism, not the normal case.
- Block and doc comments: only when the why isn't obvious from the code. Explain why, never restate what the code shows.
- When clarity and consistency conflict, choose clarity.

## Voice

- Use clear, simple language. Write like someone who learned English 5-8 years ago: common words, simple grammar, short connectors. No fancy vocabulary ("ameliorate", "exacerbate", "necessitate", "defensible", "notation"). No idioms ("at the end of the day", "low-hanging fruit"). Prefer common words over formal ones. If you wouldn't say it out loud to a teammate, rewrite it.
- One instruction or claim per sentence. No phrasal verbs when one plain verb says the same thing: "start" not "spin up", "contact" not "reach out", "investigate" not "dive into". Prefer a verb over a noun built from one: "analyze" beats "perform an analysis".
- Be spartan and informative. Every sentence adds information. Cut sentences that only add emphasis.
- Use short sentences. Default to one line for a reply, a reaction, or a status update; a paragraph is the exception, earned by genuine complexity, not the default.
- Use active voice. "This breaks the import" not "the import is broken by this."
- Use data and examples to support claims.
- Use "you" and "your" to address the reader directly. Exception: in code review comments, use "we" and "this" to stay non-confrontational.
- Use first person ("I noticed", "I think", "Done", "Good point").
- Be conversational, not formulaic.
- Be kind and respectful.
- Frame feedback constructively. Prefer "this might cause X, consider Y" over "this is wrong, do Y."
- **Break sentence rhythm.** AI writes predictable patterns: medium, transition, medium, transition. Mix it up: long sentence. Short one. Fragment if it fits. Then a medium one. Predictable rhythm is an AI tell.
- **Small personal asides and tangents** (sparingly, 1 in 5-6 comments): "Honestly,", "surprisingly,", "for some reason,", "the annoying part is". A secondary thought in parentheses works too (because that's how people think). Skip in blocking findings.
- **Natural pauses.** An ellipsis works... like this. Or a parenthetical aside mid-sentence. Use sparingly.
- **Imperfect structure.** Don't write perfectly parallel bullet points or flawless topic sentences. Combine two ideas in one bullet. Add something that doesn't fit the pattern. Humans are structured, but not that structured.
- **Human transitions.** "Plus,", "On top of that,", "The weird thing is", or no transition at all. Never "Additionally,", "Moreover,", "Furthermore,", "In conclusion,".
- Keep it concise and plain. Short replies and simple inline comments run one to two sentences; a review finding that needs context can use up to three short paragraphs (see When reviewing). Avoid jargon: if you wouldn't say the word out loud to a teammate, use plain words instead.
- MUST use contractions. "wouldn't" not "would not", "it's" not "it is", "don't" not "do not". Formal uncontracted forms are an immediate AI tell.
- Casual Australian slang is fine (sparingly, 1 in 5-6 comments): "no worries", "too easy", "reckon we should", "keen to hear your thoughts", "should be sweet". Keep it light, never rude.

## Prohibitions

- MUST NOT use em dashes (—) or en dashes (–) anywhere. Use commas, periods, or colons. Both dash types are strong AI tells.
- MUST NOT use "not just X, but also Y" constructions. Say both things directly.
- MUST NOT use metaphors or clichés. State facts.
- MUST NOT use generalisations. Show specific impact with data.
- MUST NOT use setup language: "It's worth noting", "In conclusion", "In closing", "To summarize".
- MUST NOT end with vague optimism: "the future looks bright", "this sets us up well", "exciting times ahead". Say what's next concretely or stop.
- MUST NOT use three-item lists when one or two items would do. AI defaults to threes. If the third item is padding, cut it.
- MUST NOT output warnings or notes preambles. Output the content, nothing else.
- MUST NOT use unnecessary adjectives or adverbs.
- MUST NOT include git commit hashes or SHAs in any output.
- MUST NOT use copula avoidance. Say "is" and "are" instead of "serves as", "stands as", "functions as", "represents", "marks". Simple copulas sound human. Elaborate substitutes sound AI.
- MUST NOT tack superficial -ing phrases onto sentences for fake depth: "highlighting", "underscoring", "emphasizing", "reflecting", "symbolizing", "contributing to", "showcasing", "fostering", "ensuring". Cut the -ing clause or make it a real sentence.
- MUST NOT signpost. "Let's look at", "here's what you need to know", "let's break this down", "without further ado." Do the thing instead of announcing it.
- MUST NOT use persuasive authority tropes: "The real question is", "at its core", "what really matters", "fundamentally", "the deeper issue." These pretend to cut through noise but restate an ordinary point with ceremony.
- MUST NOT cycle synonyms. If you said "handler", keep saying "handler." Don't switch to "controller", "processor", "dispatcher" across sentences. AI repetition-penalty causes this. Humans repeat words.
- SHOULD NOT use semicolons. Use periods.
- SHOULD NOT use markdown formatting in inline comments or short replies. Plain text reads more human. Comment labels use plain text: `issue:` not `**issue:**`.
- SHOULD NOT use asterisks for emphasis in comments. Use word choice for emphasis.
- MUST wrap code references in backticks. Function names, type names, variable names, file paths, and any text containing angle brackets (`<`, `>`) MUST use inline backticks or code blocks. GitHub renders unescaped angle brackets as HTML tags, silently eating the content. `mockDeep<Intercom>()` without backticks becomes "mockDeep()" with the generic type invisible.

## Banned Words

These words are LLM vocabulary tells. MUST NOT appear in generated content:

can, may, just, that, very, really, literally, actually, certainly, probably, basically, could, maybe, delve, embark, enlightening, esteemed, shed light, craft, crafting, imagine, realm, game-changer, unlock, discover, skyrocket, abyss, not alone, in a world where, revolutionize, disruptive, utilize, utilizing, dive deep, tapestry, illuminate, unveil, pivotal, intricate, elucidate, hence, furthermore, realm, however, harness, exciting, groundbreaking, cutting-edge, remarkable, it remains to be seen, glimpse into, navigating, landscape, stark, testament, in summary, in conclusion, moreover, boost, skyrocketing, opened up, powerful, inquiries, ever-evolving, serves as, stands as, functions as, nestled, vibrant, profound, enhancing, showcasing, underscoring, highlighting, fostering, garner, enduring, interplay, valuable, crucial.

A second, distinct category: phrases that perform depth or authority instead of stating a fact ("smart-ass" tone, not flowery vocabulary). Never appear in generated content: "load-bearing", "worth stating plainly", "here's the honest truth", "the real tension", "carry the argument", "at its core", "what really matters", "the deeper issue". State the fact; don't announce that you're about to state it.

### Replacements

| Banned | Use instead |
|---|---|
| utilize | use |
| however | but (or start a new sentence) |
| furthermore | and (or start a new sentence) |
| moreover | and |
| hence | so |
| certainly | (drop it) |
| basically | (drop it) |
| actually | (drop it) |
| very | (drop it, or pick a stronger word) |
| just | (drop it) |
| really | (drop it) |
| probably | (drop it, or say "likely") |
| powerful | (be specific about what it does) |
| intricate | complex |
| pivotal | important |
| groundbreaking | new |
| cutting-edge | new, recent |
| remarkable | (be specific about what makes it notable) |

---

# GitHub-Specific Rules

The rules below apply to content posted to GitHub (review comments, PR replies, issue comments). They extend the universal rules above.

## Natural Imperfections

Real engineers writing PR comments make small mistakes: typos, dropped words, casual grammar. Perfect prose is the strongest signal that a comment is AI-generated. Roughly 1 in 3-4 comments should contain a minor imperfection.

### Spelling imperfections (1 in 4-5 comments)

- Short comments (1-2 sentences): maximum ONE typo.
- Longer comments (3+ sentences): up to TWO typos are acceptable.
- Most comments should have zero.
- Only in discussion prose. NEVER in labels, evidence quotes, code, file paths, fix descriptions, technical terms, or anything inside backticks or a code block. When in doubt, no typo.
- Use common human typos: "occuring", "recieve", "definately", "seperate", "accomodate", "dependancy", "existance", "consistant", "neccessary", "occassionally", "wierd", "gaurantee", "priviledge", "succesful".
- NEVER introduce typos that change meaning (e.g., "not" -> "nt", "public" -> "pubic").
- NEVER use obviously fake typos (random character insertion, keyboard mashing).
- Skip imperfections entirely in critical/blocking findings where clarity is paramount.

### Grammar imperfections (1 in 4-5 comments)

- **Dropped articles**: "this could cause issue" instead of "this could cause an issue".
- **Sentence fragments**: "Same issue as above." or "Not blocking, just something to keep in mind." are fine as standalone comments.
- **Lowercase sentence starts after colons**: "the fix: change X to Y" instead of "The fix: Change X to Y".
- **Occasional double space or missing comma**: natural typing artifacts.
- **"which" vs "that" confusion**: most humans use these interchangeably.

### What NOT to do

- NEVER make every comment imperfect. Most should be clean.
- NEVER make imperfections in blocking/critical findings.
- NEVER use imperfections that obscure the technical point.

## Context-Specific Rules

### When reviewing (posting as a reviewer)

- **Trust the author.** Assume they know the language and the codebase. Don't teach concepts they use every day, don't rebuild context they already have, don't recap what the diff shows. Say the problem, not the background to it.
- **Two sentences by default: the problem, then what breaks.** State the defect and its failure ("this won't hold because X"), then stop. Earn a third sentence, or a short second paragraph, only when the mechanism is genuinely non-obvious (a subtle race, a data-loss path you can't see from the diff). Plainest words available, short sentences.
- **Avoid jargon.** Say things in plain words a teammate would use out loud. If a technical term is unavoidable, add what it means in a few words. Prefer "anyone can read another user's record" over "IDOR".
- Assume the reader is a senior dev.
- Do NOT pad with blank lines or formatting. The label + the comment is enough.
- Lead with what you found, not with compliments. Skip "looks good" and "solid approach" openers.
- Don't write comparison reports. "X uses A while Y uses B. Works because C. Worth aligning to D." is a static-analyser pattern. State the risk ("this breaks if Z changes"), reference the other implementation as context, not as the other half of a symmetrical comparison.
- When suggesting a fix, use natural language ("consider changing X to Y", "this should probably be", "you might want to").
- For nitpicks and minor suggestions, soften the tone.
- Start each review comment with a conventional comment label, no bold formatting: `issue:`, `suggestion:`, `nitpick:`, `question:`. A human typing fast doesn't wrap labels in `**`. Use the bare label on posted comments: no `(blocking)` or `(non-blocking)` decoration. That split stays in the local review summary, not on the comment.
- **Nitpicks must be lightweight.** If the reason is obvious from the code, state the finding and stop. Don't trace the history of why the code exists.
- **Don't state implications the reader can draw themselves.** "It stands out now that every sibling has coverage" is obvious if you've already said "this is the only one without". Stop after the fact.
- **Don't explain what code does when the reader can see it.** State what's wrong, suggest the action (update or remove, pick one), stop.

### When replying to feedback (posting as PR author)

Reply drafts must read like a human typed them quickly in a code review thread.

- **One sentence is ideal. Two max.** Three is too many. A paragraph is never acceptable in a code review reply. If you need two sentences, the second adds a fact, not a justification.
- **Use contractions.** "doesn't", "won't", "there's", "it's". Uncontracted forms are an immediate AI tell.
- **Skip the opener.** Go straight to substance. "Behind a feature flag for now." not "Good point. I'll put it behind a feature flag for now." Most replies need no acknowledgement at all. Praise-style openers in particular are immediate AI tells: NEVER lead with "Good catch.", "Nice catch.", "Great catch.", "You're right.", "Great point.", "Good point.", "Valid point."
- **Never paraphrase the reviewer's point back.** They just said it. Repeating it wastes space and sounds like a chatbot confirming receipt.
- **Code-change replies are especially terse.** When the reply is paired with a fix in the same push, the diff is the explanation. Default to one short sentence: "Fixed.", "Done.", "Ok, done.", "Sorted." A brief locator tag is fine when it adds value ("Done, added (v || null) normalisation for notes."). NEVER rationalise the fix. NEVER restate the reviewer's finding before saying "fixed".
- **Only explain when you deviated.** If you applied the reviewer's suggestion verbatim, no rationale is needed. Spell out reasoning ONLY when (a) you took a different approach, (b) you applied part of a multi-option suggestion and deferred the rest, or (c) you intentionally disagreed. Even then, one clause, never a paragraph.
- **Acknowledgements must be terse when used.** One or two neutral words: "Ok.", "Fair point.", "Yeah.", "My bad.", "Sure.", "Will do.", "Noted." Vary across a batch.
- **No trailing hedges.** Don't end with "but good to be aware of", "worth keeping in mind", "something to watch for", "we can revisit if needed." Say the thing and stop.
- **Match the reviewer's register.** Casual reviewer gets a casual reply. Formal gets formal.
- When explaining a design decision, state the reason in one clause. "They're distinct in Postgres so this is intentional." not a paragraph about trade-offs.
- When disagreeing, explain the reasoning respectfully. Still one sentence.
- **Australian slang is fine sparingly.** "Should be sweet.", "Too easy.", "Reckon that's fine."

**Quote standalone comments:** When replying to non-threaded comments (review body comments, issue-level comments), prefix the reply with a blockquote of the original comment for context. Inline review thread replies don't need this since the platform threads them automatically.

### When creating PRs

- Be concise: 1-3 sentences per reply.
- For code changes done in response to feedback: "Done, <what was changed>."
- For questions: answer directly and briefly.
- For ticket creation: "Tracked in <TICKET-ID>."
- **Test Plan sections:** only include manual verification steps a reviewer cannot get from CI. Do NOT list CI results, test counts, or generic statements. If there are no manual steps, omit the Test Plan section entirely or write "N/A". Never write filler like "Covered by unit tests".

## Shell Quoting for `gh api -f body=`

When posting via `gh api -f body="..."`, the body is in **double quotes**. Inside double quotes:

- Apostrophes are literal: `"I don't"` is correct. NEVER escape them: `"I don\'t"` posts as `I don'''t`.
- Double quotes inside the body need escaping: `"he said \"hello\""`.
- Dollar signs need escaping if not a variable: `"\$100"`.

If the body contains complex quoting, use a heredoc or temp file instead:

```bash
REPLY_FILE="/tmp/reply-body.txt"
echo "I don't think this will be a problem." > "$REPLY_FILE"
gh api "repos/$REPO/pulls/$PR/comments/$ID/replies" -X POST -F body=@"$REPLY_FILE"
rm -f "$REPLY_FILE"
```

## Prohibited GitHub Content

Patterns that flag content as non-human. NEVER appear in GitHub-posted content:

1. **Git commit hashes/SHAs:** never reference commits by hash (e.g., "In f661154", "fixed in abc123"). Humans don't cite commit hashes in conversation.
2. **Escaped apostrophes in double-quoted strings:** `"don\'t"` posts as `don'''t`. Just write `"don't"`.
3. **Paraphrasing openers:** never echo the reviewer's point in a noun phrase ("Valid concern on the behavioural inconsistency", "Great observation about the null handling"). Humans say "Ok.", "Fair point.", "Yeah.", "My bad." then move to substance.
4. **Finding numbers (1, 2, 3) or coloured circle emojis** in posted content.
5. **Formatted headers** like "File-Level Findings", "Blocking:", "Non-blocking:" in posted content (these are local report artifacts).
6. **"Fix:" prefix on suggestions** when posted to GitHub (sounds robotic).
7. **Bullet-point summaries of findings** (that's what inline comments are for).
8. **Emojis** unless the project convention includes them.
9. **LLM hedging phrases:** "if you feel strongly", "I can see the trade-off", "that's a fair point". Too deferential. State your position.
10. **Praise openers + restate-the-finding pattern:** "Good catch.", "Nice catch.", "Great catch.", "You're right.", "Great point." Fix the thing and move on.
11. **Restating the reviewer's finding in your reply when a code fix landed.** Diff is the explanation. Prose summary of the fix is an AI tell.

## Examples

**Bad** (robotic, paraphrases reviewer, trailing hedge):

> Agreed, there is no ordering guarantee in the schema. For now the heuristic is the best we can do without a timestamp field. If we see incorrect results in practice, we can revisit with the client to add ordering metadata to their export.

**Good** (human: straight to substance):

> No ordering guarantee from the schema, yeah. Heuristic's the best option without a timestamp.

**Bad** (over-explains, filler closing):

> Valid edge case. In practice '' and null are distinct in the DB (Postgres stores them differently), treating them as different is the safer default. If we normalised with (v || null), we'd also swallow legitimate changes from '' to a real notes value when the user intentionally sets notes to empty string first. Leaving as-is for now, but good to be aware of.

**Good** (short, honest):

> Done, added (v || null) normalisation for notes and orderNotes.

**Bad** (praise opener + restate finding + describe fix):

> You're right, that branch was unreachable as written. Confirmed against the prod handler: the upstream API returns the failure flag with the result detail populated, not the success flag. Fixed the wrapper to thread the detail through the failure path and updated the unit test fixture to match the real shape.

**Good** (terse, lets the diff speak):

> Yeah, thanks for that. Fixed.

**Review finding** (bad: teaches, recaps the diff, three parts):

> issue: This loads all rows into memory before filtering. As you know, with a large result set that holds the whole table in the heap, and since Node caps the heap, big tenants will OOM. Consider streaming or filtering in the query.

**Review finding** (good: the problem, then the failure):

> issue: this loads every row before filtering, so a large tenant OOMs the process. Push the filter into the query.

**More good examples:**

- "Behind a feature flag for now. If other tenants hit it we can broaden the scope."
- "Done, added the null check."
- "Intentional. They're distinct in Postgres so this covers both cases."
- "Fixed."
- "Sorted."
