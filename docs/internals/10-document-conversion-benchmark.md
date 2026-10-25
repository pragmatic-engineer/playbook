# Document conversion benchmark

Measurement behind the `docs.convert` default. Design: [ADR-0027](../adr/0027-anydoc-as-optional-local-tool.md), replaced by [ADR-0051](../adr/0051-bundle-anydoc-as-pbdoc-helper.md), which bundles the converter. The token measurements below used the npm `anydoc` 0.2.4, the same engine `pbdoc` is built from. Command and safeguards: [Reading documents](../guides/06-reading-documents.md).

## Question

Does `playbook doc read` cost fewer tokens than the model reading a raw PDF or Office file, without losing correctness?

## Decision rule

Default `docs.convert` to `auto` only if all hold:

1. Text PDFs need at least 30 percent fewer tokens.
2. The PDF answers are as correct as with the raw file.
3. Office formats count as a benefit when the raw route needs extra tool rounds. A loss of correctness on a format is a reason to say so in the hint, not to hide it.

The default is chosen on the per document benefit, not on how often documents were read on the machine that ran the test. That machine had no PDF or Office reads in its history, so its traffic says nothing about a work computer that reads many documents.

## Method

- Converter: anydoc 0.2.4 installed in a scratch folder from npm with install scripts off. Model: Haiku, headless, no session persistence, no settings, no memory.
- Documents: synthetic fixtures made by a generator, no real content. A 20 page text PDF, a 2 page text PDF, a docx with a heading, a table and one tracked deletion, a pptx with 3 slides, an xlsx with 2 sheets and a 40 row csv.
- Three fixed questions per document with a known answer. A reply is correct if it contains the expected value. Each document and route ran 3 times, so a cell is out of 9.
- Input tokens are input plus cache writes plus cache reads as the CLI reports them. They include about 10k of fixed tool definition overhead in the raw and doc routes, so a short document shows a small percent change.
- Spend for the 54 calls: about 0.07 USD.

Routes:

- **Raw**: tools Read and Bash, the model opens the file itself.
- **Doc**: same tools, the model is told to run `playbook doc read <file>` with Bash.
- **Inline**: framed output in the prompt, no tools. This is the floor for what the text costs.

## Results

Averages of 3 runs. Tokens are input tokens including overhead.

| Document | Raw tokens | Doc tokens | Change | Inline tokens | Raw turns | Doc turns | Raw correct | Doc correct | Inline correct |
| :- | -: | -: | -: | -: | -: | -: | -: | -: | -: |
| text PDF, 20 pages | 67,918 | 36,865 | -46% | 27,887 | 2.0 | 4.7 | 9 of 9 | 9 of 9 | 9 of 9 |
| text PDF, 2 pages | 15,084 | 12,216 | -19% | 3,855 | 2.0 | 2.0 | 9 of 9 | 9 of 9 | 9 of 9 |
| docx, tracked deletion | 17,778 | 12,215 | -31% | 3,853 | 2.7 | 2.0 | 9 of 9 | 6 of 9 | 6 of 9 |
| xlsx, 2 sheets | 18,072 | 12,175 | -33% | 3,794 | 2.7 | 2.0 | 9 of 9 | 9 of 9 | 9 of 9 |
| pptx, 3 slides | 12,537 | 12,126 | -3% | 3,763 | 2.0 | 2.0 | 9 of 9 | 9 of 9 | 9 of 9 |
| csv, 40 rows | 12,202 | 12,766 | +5% | 4,416 | 2.0 | 2.0 | 9 of 9 | 9 of 9 | 7 of 9 |

Cost per call in US dollars. Cost also follows cache behavior, so treat the ratios as indicative.

| Document | Raw cost | Doc cost |
| :- | -: | -: |
| 20 page PDF | 0.0083 | 0.0018 |
| 2 page PDF | 0.00076 | 0.00040 |
| docx | 0.0010 | 0.00043 |
| xlsx | 0.0010 | 0.00039 |
| pptx | 0.0008 | 0.00038 |

## Findings

- **Text PDFs pass the bar.** Across both PDFs the doc route used 41 percent fewer tokens (49,081 against 83,002) with 18 of 18 answers correct. Savings grow with document size. The 2 page PDF is only about 114 tokens of text, so fixed overhead hides most of the change.
- **A long document makes the model search.** The 20 page PDF converts to about 91 KB of text. Claude Code's Bash tool saves output over its size limit to a file and returns a short preview, so the model searched that file with grep instead of reading it whole. That took 4.7 turns instead of 2, still cost less, and found the right answers every time. The file belongs to Claude Code. Playbook's own scratch file is already deleted by then.
- **Office files gain when the raw route needs extra rounds.** docx and xlsx took 2.7 turns raw and 31 to 33 percent more input tokens than the doc route. pptx was a tie because the model reads a small pptx in one Bash call. The doc route also removes the Python the model would write.
- **Tracked changes are lost.** In the docx fixture the model could not say what a tracked deletion removed, because anydoc drops deleted text. The raw route answered it. The output header says tracked changes are not represented, and the hint for Word formats says to open the original when they matter.
- **csv gains nothing.** The Markdown table is larger than the raw csv, so csv is not hinted.
- **Counting is a model risk.** The inline route missed 2 of 9 csv answers on row counting. This is a limit of reading text without running code, not of the conversion.

## Decision

The PDF bar is met (41 percent fewer tokens, no loss of correctness), and Office formats save rounds or tokens except pptx and csv. `docs.convert` now defaults to `auto`. It is active only when the `anydoc` converter is found, so a machine without it sees no change. Set `docs.convert` to `off` to stop the hint.

## Limits of this test

- Synthetic documents and one small model. Real scanned or multi column documents were not tested, and scanned PDFs need OCR, which playbook never runs.
- 3 runs per cell, so small differences are noise.
- The machine's transcripts held no PDF or Office reads either. Savings per document are what was measured.

## The bundled helper

[ADR-0051](../adr/0051-bundle-anydoc-as-pbdoc-helper.md) ships the converter as `pbdoc`, built from the `anydoc` 0.2.4 crate. Measured on 2026-10-19 on aarch64-apple-darwin with the release profile of this repo:

| Measure | Value |
| :- | :- |
| Stripped binary | 7,233,008 bytes (6.9 MiB) |
| Gzip -9 | 3,695,855 bytes (3.5 MiB) |
| Crates in the closure | 103 on macOS and Linux musl, 105 on Windows |
| New lock file entries | 72 |
| Network crates in the closure | none |
| Licences | all permissive (MIT, Apache-2.0, BSD-3-Clause, Zlib, Unicode-3.0, Unlicense, BSL-1.0). The one LGPL-2.1 crate is also offered under MIT and Apache-2.0. |
| Smoke run on a csv | correct table, 4.2 MB peak memory |

The release build fails when the binary passes 8 MiB or the tarball passes 4.5 MiB.

## Not re-run with pbdoc

The token and correctness table above comes from the npm package. `pbdoc` is built from the same engine, so no change is expected, but the table was not measured with `pbdoc`.
