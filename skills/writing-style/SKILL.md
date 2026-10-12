---
name: writing-style
description: "Voice, banned-word, and no-dash rules for prose a human reads, such as PRs, review comments, Jira, Confluence, Slack, ADRs, commit messages. Use before writing or posting any."
---

# Writing Style

Universal writing rules for all generated content intended for human readers: PR descriptions, review comments, ticket descriptions, ADRs, Confluence pages, Slack messages, Jira tickets.

These rules are MUST-level unless marked SHOULD.

> **IRON RULE:** MUST NEVER use em dashes or en dashes in any output, and MUST write in simple, plain English (common words, short sentences). This applies to every file, comment, commit message, and generated text. Use commas, colons, or periods instead of dashes. Both dash types are strong AI tells; fancy vocabulary and long sentences are too. Use straight quotes (`"`, `'`), never curly/smart quotes (`"`, `"`, `'`, `'`); same ASCII-only reasoning as the dash rule.

> **IRON RULE:** MUST NEVER write a code or doc comment unless it is one of: a doc block at a module, library, or public-helper-function boundary; a warning that the operation is expensive; a warning that the operation is destructive or dangerous; an explanation of why the code does something that looks odd; a deprecation notice; context for a return type or value the signature doesn't make clear; or, at a class/type level only, context for a service class (which service this is) or a DTO (what the shape means). MUST NEVER restate what the code already shows. Say it in one sentence by default, a second only for a genuinely non-obvious mechanism; wrap a long line instead of writing one unwrapped line, don't pad a short comment into extra lines. Write for the code's future reader, not this session: never name a plan, brief, Work Unit, done-when criterion, or ticket number in a comment. An already-committed, durable reference (an ADR, a documented convention) is fine. `no-slop-guard` blocks an orchestration-artifact mention, a dispatch id, or an unwrapped comment line past the project's line-length standard, at the Edit/Write tool call itself; use your own judgment for the rest of this rule.

> **IRON RULE:** MUST NEVER write in third person about the person Claude is posting as. Content posted under the reader's own identity or access token, a Jira or Confluence comment, a Slack message sent with their token, an issue-tracker comment of any kind, is their own words. Write "I need this by Friday", never "the user needs this by Friday" or a third-person mention of the reader by name. This holds even when the request named the person earlier in the session: the account holder is speaking, not being described. Read the draft back and ask "would this read as impersonation if a stranger saw it," before posting.

## Reviewer usability (MUST, adapted from "Don't Make Me Think")

MUST apply these principles to anything written for review: the reviewer is the user, the text is the interface, a question in their head is the cost. Reviewers scan, they don't read.

- Commit messages, PR descriptions and review findings: length and shape rules are in the references listed under Loading the references.
- Block and doc comments: see the IRON RULE above.
- When clarity and consistency conflict, MUST choose clarity.

## Voice

- Use clear, simple language. Write like someone who learned English 5-8 years ago: common words, simple grammar, short connectors. No fancy vocabulary ("ameliorate", "exacerbate", "necessitate", "defensible", "notation"). No idioms ("at the end of the day", "low-hanging fruit"). Prefer common words over formal ones. If you wouldn't say it out loud to a teammate, rewrite it.
- One instruction or claim per sentence. No phrasal verbs when one plain verb says the same thing: "start" not "spin up", "contact" not "reach out", "investigate" not "dive into". Prefer a verb over a noun built from one: "analyze" beats "perform an analysis".
- Be spartan and informative. Every sentence adds information. Cut sentences that only add emphasis.
- Say each point once, in plain words. Don't restate it later in the same comment or reply.
- Use short sentences. Default to one line for a reply, a reaction, or a status update; a paragraph is the exception, earned by genuine complexity, not the default.
- Use active voice. "This breaks the import" not "the import is broken by this."
- Use data and examples to support claims.
- Use "you" and "your" to address the reader directly. Code review comments are the exception: see `github-review-comments`.
- Use first person ("I noticed", "I think", "Done", "Good point").
- Be conversational, not formulaic.
- Be kind and respectful.
- Frame feedback constructively. Prefer "this might cause X, consider Y" over "this is wrong, do Y."
- **Break sentence rhythm.** AI writes predictable patterns: medium, transition, medium, transition. Mix it up: long sentence. Short one. Fragment if it fits. Then a medium one. Predictable rhythm is an AI tell.
- **Small personal asides and tangents** (sparingly, 1 in 5-6 comments): "Honestly,", "surprisingly,", "for some reason,", "the annoying part is". A secondary thought in parentheses works too (because that's how people think). Skip in blocking findings.
- **Natural pauses.** An ellipsis works... like this. Or a parenthetical aside mid-sentence. Use sparingly.
- **Imperfect structure.** Don't write perfectly parallel bullet points or flawless topic sentences. Combine two ideas in one bullet. Add something that doesn't fit the pattern. Humans are structured, but not that structured.
- **Human transitions.** "Plus,", "On top of that,", "The weird thing is", or no transition at all.
- Short replies and simple inline comments run one to two sentences. A review finding that needs context may use up to three short paragraphs (see `github-review-comments`).
- MUST use contractions. "wouldn't" not "would not", "it's" not "it is", "don't" not "do not". Formal uncontracted forms are an immediate AI tell.
- Casual Australian slang is fine (sparingly, 1 in 5-6 comments): "no worries", "too easy", "reckon we should", "keen to hear your thoughts", "should be sweet". Keep it light, never rude.

## Prohibitions

- MUST NOT use contrast framing: "it's not X, it's Y", "this isn't about X, it's about Y", "not just X, but also Y". State the point you mean directly, and say both things plainly when both matter.
- MUST NOT tack a ", so ..." consequence clause onto a sentence. State the consequence as its own sentence, or cut it if the reader can infer it. Simple, direct English: short sentences beat long ones, and less prose is better than more.
- MUST NOT use metaphors, clichés, or analogies. State facts.
- MUST NOT use generalisations. Show specific impact with data.
- MUST NOT use setup language: "It's worth noting", "In conclusion", "In closing", "To summarize".
- MUST NOT end with vague optimism: "the future looks bright", "this sets us up well", "exciting times ahead". Say what's next concretely or stop.
- MUST NOT use three-item lists when one or two items would do. AI defaults to threes. If the third item is padding, cut it.
- MUST NOT output warnings or notes preambles. Output the content, nothing else.
- MUST NOT use unnecessary adjectives or adverbs.
- MUST NOT put commit hashes or SHAs in text posted to GitHub, Jira, Confluence, or Slack. Local notes such as a session handoff, and a report returned to the orchestrator, may name them.
- MUST NOT avoid the copula. Say "is" and "are", not "represents", "marks", "features" or the "serves as" family in Banned Words.
- MUST NOT use vague "from X to Y" range framing (e.g. "from minor bugs to major security issues"). List the specific things directly.
- MUST NOT end with a chatbot sign-off: "I hope this helps!", "Let me know if you have any questions!", "Of course!", "Certainly!". Stop after the last real sentence.
- SHOULD NOT convert prose into a list of "**Label:** detail" bullets in a PR description, ADR, or design doc. Write flowing prose instead, the same way inline comments use plain-text labels instead of bold ones.
- MUST NOT tack superficial -ing phrases onto sentences for fake depth ("emphasizing", "reflecting", "contributing to", "ensuring", plus the -ing words in Banned Words). Cut the clause or make it a real sentence.
- MUST NOT signpost. "Let's look at", "here's what you need to know", "let's break this down", "without further ado." Do the thing instead of announcing it.
- MUST NOT use persuasive authority tropes: "The real question is", "at its core", "what really matters", "fundamentally", "the deeper issue." These pretend to cut through noise but restate an ordinary point with ceremony.
- MUST NOT cycle synonyms. If you said "handler", keep saying "handler." Don't switch to "controller", "processor", "dispatcher" across sentences. AI repetition-penalty causes this. Humans repeat words.
- SHOULD NOT use semicolons. Use periods.
- SHOULD NOT use markdown formatting in inline comments or short replies. Plain text reads more human. Comment labels use plain text: `issue:` not `**issue:**`.
- SHOULD NOT use asterisks for emphasis in comments. Use word choice for emphasis.
- MUST wrap code references in backticks. Function names, type names, variable names, file paths, and any text containing angle brackets (`<`, `>`) MUST use inline backticks or code blocks. GitHub renders unescaped angle brackets as HTML tags, silently eating the content. `mockDeep<Intercom>()` without backticks becomes "mockDeep()" with the generic type invisible.

## Banned Words

These words are LLM vocabulary tells. MUST NOT appear in generated content:

very, really, literally, actually, certainly, probably, basically, delve, embark, enlightening, esteemed, shed light, craft, crafting, imagine, realm, game-changer, unlock, discover, skyrocket, abyss, not alone, in a world where, revolutionize, disruptive, utilize, utilizing, leverage, facilitate, numerous, dive deep, tapestry, illuminate, unveil, pivotal, intricate, elucidate, hence, furthermore, additionally, however, harness, exciting, groundbreaking, cutting-edge, breathtaking, remarkable, it remains to be seen, glimpse into, navigating, landscape, stark, testament, in summary, in conclusion, moreover, boost, skyrocketing, opened up, powerful, inquiries, ever-evolving, serves as, stands as, functions as, boasts, nestled, vibrant, profound, enhance, enhancing, showcase, showcasing, underscore, underscoring, highlighting, fostering, garner, enduring, interplay, valuable, crucial.

A second, distinct category: phrases that perform depth or authority instead of stating a fact ("smart-ass" tone, not flowery vocabulary). Never appear in generated content: "load-bearing", "worth stating plainly", "here's the honest truth", "the real tension", "carry the argument", "at its core", "what really matters", "the deeper issue". State the fact; don't announce that you're about to state it.


---

## Loading the references

This file holds the universal rules. Load the reference that matches the prose you write, with `playbook skill ref writing-style <name>`, which prints the file path to read:

- Commit message, PR title or PR body: `commits-and-prs`.
- Review comment, reply to feedback, or issue or PR comment on GitHub: `github-review-comments`, plus `examples` and `imperfections`.
- Anything posted to GitHub, including a PR body: `github-prohibited`.
- A `gh api -f body=` call: `shell-quoting`.
