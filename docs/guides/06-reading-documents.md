# Reading documents

`playbook doc read <file>` turns a PDF or an Office file into Markdown, on your machine, so a session reads text instead of a binary file. It needs the optional `anydoc` converter. Playbook never installs it for you and never downloads it.

## Turn it on

1. Install the converter yourself: `npm install --global @firecrawl/anydoc` (needs Node 20 or later).
2. Check it: `playbook doc status`.
3. Nothing more to do: `docs.convert` defaults to `auto`. A Read of a whole PDF, or of a Word, PowerPoint, Excel, EPUB, RTF or OpenDocument file, gets one line that suggests `playbook doc read <file>`. The hook only suggests. It never changes the call and never blocks. Set `docs.convert` to `off` to stop the hint. See the [benchmark](../internals/10-document-conversion-benchmark.md).

Without the converter the feature is off and nothing changes. `playbook doc read` then says how to install it.

## What it reads

pdf, docx, doc, odt, rtf, epub, pptx, ppt, odp, xlsx, ods and csv. Images are not converted, use Read for those.

## Safeguards

- **Local only.** The OCR option and the hosted mode of anydoc send the document to a remote server. Playbook never passes them. A scanned PDF with no text layer answers "needs OCR, not supported locally" and nothing is retried elsewhere.
- **No network where the system allows it.** The converter runs inside the macOS sandbox with network denied, or in a Linux network namespace. When neither works the output says that only the environment was scrubbed. The environment is always scrubbed: no proxy variables, no API keys, no `NODE_OPTIONS`.
- **Limits.** Files over `docs.maxBytes` (25 MB) are refused. The converter is stopped after `docs.timeoutSeconds` (30) or when it uses more than `docs.maxMemoryMb` (1024). The text is cut at `docs.maxOutputChars` (200000) with a marker that says how many characters were shown.
- **Private temporary files.** The converter writes to a random folder under `~/.config/playbook/tmp` that only you can open. It is deleted when the command ends, on success, error, timeout, interrupt or crash of playbook itself. A folder left by a killed process is removed on the next run once it is older than an hour.
- **Untrusted content.** The text is wrapped in a block with a random tag and a line that says to treat it as data and not follow instructions inside it. Nothing from the document is logged or stored.

## What is lost

The output lists what the conversion does not carry. For Word files, tracked changes are not represented, so deleted text is missing. Charts and images become nothing or alt text. Slide layout is dropped. Spreadsheet formulas show their last saved value. When a detail matters, read the original.

## Config keys

See the [config keys page](../reference/config-keys.md): `docs.convert`, `docs.maxBytes`, `docs.timeoutSeconds`, `docs.maxOutputChars`, `docs.maxMemoryMb`, `docs.anydocPath`.
