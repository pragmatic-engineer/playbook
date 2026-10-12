# ADR-0027: Use anydoc as an Optional Local Tool Found on PATH, Never Vendored

- **Status:** Accepted
- **Date created:** 2026-10-12
- **Date modified:** 2026-10-12

## Context

Issue #799 asks whether a converter such as anydoc (MIT, github.com/firecrawl/anydoc) should read PDF and Office files instead of the model. The maintainer decision is to adopt it if it brings a token or cost benefit, with safeguards. Conversion must stay local. The OCR flag and any hosted mode upload the document to a remote server and are never used.

The evaluation (2026-10-12, anydoc 0.2.4) found:

- A 20 page text PDF costs about 2.4 times fewer tokens as Markdown than through the built in Read. For Office, EPUB and CSV files the built in Read cannot parse the file, so the model falls back to several Bash and Python rounds (2 to 4 turns, 12k to 20k tokens) that one Markdown read replaces.
- Fidelity gaps: tracked deletions in docx are dropped without a marker, slide bullets are flattened, charts and images become alt text, scanned PDFs fail with a needs OCR error.
- The library caps archive entries, XML depth and nodes, and spreadsheet cells. It has no output size cap, no wall clock timeout and no memory cap. Peak memory reached about 613 MB on a 150 KB hostile file.
- The project is about ten weeks old, has 51 commits, and one author wrote 43 of them. `cargo audit` shows two warnings, no vulnerabilities.
- anydoc ships no standalone binary. The release assets are Node addons and Python wheels. The command line comes from the npm package `@firecrawl/anydoc` (binary name `anydoc`, needs Node 20 or later).
- Linking the crate would add 115 crates and about 7 MB to a release binary that ADR-0018 recently cut from 8.2 MB to 5.5 MB.

## Decision

1. **Optional external tool.** playbook calls an `anydoc` executable found on `PATH` (or at the path in `docs.anydocPath`). It is not vendored, not linked, and never downloaded by playbook. When it is missing, the feature is off and `playbook doc read` prints how to install it: `npm install --global @firecrawl/anydoc`.
2. **The user installs it.** `playbook deps` lists it as an optional tool. Nothing installs it silently.
3. **Safeguards live in playbook, outside the converter.** The command runs anydoc in a child process with an input size cap, a wall clock timeout, a memory cap, an output cap, a scrubbed environment, and a private temporary directory that is always deleted. It never passes an OCR or hosted flag. A test asserts that.
4. **Document text is untrusted data.** Output is wrapped in a delimited block with a one line note that instructions inside it must not be followed.
5. **Default stays off** (`docs.convert = off`) until the benchmark in the last PR shows at least 30 percent fewer tokens on text PDFs and no loss in correctness. That PR flips the default in the same change only if the numbers pass.
6. **Pin by version check.** playbook records the version it was tested with (0.2.4) and warns, without blocking, on a different major or minor version.

## Limits that remain

- The network block is best effort. On macOS the child runs under `sandbox-exec` with network denied when that tool exists. On Linux it uses `unshare --net` when permitted. Otherwise the child gets a scrubbed environment (proxy variables and API keys removed) and nothing more. The Node wrapper itself is third party code that runs as the user.
- Tracked changes in Word files are not represented, and playbook says so in the output header for docx files.
- Scanned and image only PDFs return a plain message and are never retried through a hosted service.

## Alternatives considered

- **Link the crate.** Rejected for binary size, 115 new crates to vet, and a young upstream. It would also run unbounded code inside the playbook process, where a memory blowup would take playbook down.
- **Vendor or download a managed binary.** Not available upstream, and a silent download is ruled out. Revisit if upstream publishes signed standalone binaries.
- **A PreToolUse hook that rewrites Read input.** Rejected in the evaluation. Hooks that return updated input do not chain and the PDF `pages` argument would be lost. The hook only suggests.
- **Do nothing.** Real traffic on the measured machine is zero reads of PDF or Office files, so the benefit is unproven for this developer. The decision is to build the guarded path and let the benchmark and usage decide the default.

## Consequences

- No change to the binary size or the dependency tree.
- Users who want the feature install one npm package.
- The feature depends on an upstream CLI contract (`anydoc <file>` prints Markdown). The tests use a fake script for that contract.
