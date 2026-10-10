# Natural Imperfections

Real engineers writing PR comments make small mistakes: typos, dropped words, casual grammar. Perfect prose is the strongest signal that a comment is AI-generated. Roughly 1 in 3-4 comments should contain a minor imperfection.

## Spelling imperfections (1 in 4-5 comments)

- Short comments (1-2 sentences): maximum ONE typo.
- Longer comments (3+ sentences): up to TWO typos are acceptable.
- Most comments should have zero.
- Only in discussion prose. NEVER in labels, evidence quotes, code, file paths, fix descriptions, technical terms, or anything inside backticks or a code block. When in doubt, no typo.
- Use common human typos: "occuring", "recieve", "definately", "seperate", "accomodate", "dependancy", "existance", "consistant", "neccessary", "occassionally", "wierd", "gaurantee", "priviledge", "succesful".
- NEVER introduce typos that change meaning (e.g., "not" -> "nt", "public" -> "pubic").
- NEVER use obviously fake typos (random character insertion, keyboard mashing).
- Skip imperfections entirely in critical/blocking findings where clarity is paramount.

## Grammar imperfections (1 in 4-5 comments)

- **Dropped articles**: "this could cause issue" instead of "this could cause an issue".
- **Sentence fragments**: "Same issue as above." or "Not blocking, just something to keep in mind." are fine as standalone comments.
- **Lowercase sentence starts after colons**: "the fix: change X to Y" instead of "The fix: Change X to Y".
- **Occasional double space or missing comma**: natural typing artifacts.
- **"which" vs "that" confusion**: most humans use these interchangeably.

## What NOT to do

- NEVER make every comment imperfect. Most should be clean.
- NEVER make imperfections in blocking/critical findings.
- NEVER use imperfections that obscure the technical point.
