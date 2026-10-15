# Conventional Comments

The format for code review comments in this plugin. It follows the [Conventional Comments](https://conventionalcomments.org/) standard. It applies to review comments on a diff or a pull request only. Chat replies, PR descriptions, commit messages, issue text, ADRs, plans and handoffs do not use it.

## Format

```
<label> [decorations]: <subject>

[discussion]
```

- label: one label that says what kind of comment it is.
- subject: the main message, one or two plain sentences.
- decorations: optional, in parentheses, comma separated. Example: `(non-blocking)` or `(security, non-blocking)`.
- discussion: optional. Context, reasoning or next steps, after a blank line.

Write the label in lowercase and plain text, with no bold.

## Labels

| Label | Meaning | Blocking default | Example |
|---|---|---|---|
| nitpick | A trivial preference. | non-blocking | `nitpick: Rename tmp to buffer.` |
| suggestion | Propose an improvement and say what and why it is better. | judge | `suggestion (non-blocking): Cache the lookup, it runs once per row.` |
| issue | A specific problem, seen by users or behind the scenes. Pair it with a suggestion. | blocking | `issue (blocking): The retry loop has no limit, so a down service is called forever.` |
| todo | A small, necessary change. | blocking | `todo: Add the new field to the export.` |
| question | A concern you are not sure about. Ask for an answer or a check. | needs an answer | `question: Can the list be empty here?` |
| thought | An idea that came up while reviewing. | non-blocking | `thought: We could share this helper with the importer later.` |
| chore | A task a process requires before acceptance. Link the process. | blocking | `chore: Run the license check, see CONTRIBUTING.md.` |
| typo | A misspelling. | blocking | `typo: recieve should be receive.` |
| polish | Nothing is wrong, and the code gets better with this change now. | non-blocking | `polish: Extract the three-line check into a named function.` |

When you are unsure a problem exists, use `question` and do not use `issue`.

## Decorations

| Decoration | Meaning |
|---|---|
| blocking | Must be fixed before the change is accepted. |
| non-blocking | Must not stop the change from being accepted. |
| security, test, perf, data, types, logic | Optional second decoration that names the review lens. |

Use `blocking` or `non-blocking` first when you add a lens: `issue (blocking, security): ...`. Add a decoration when the default does not fit. A bare `blocking:` label from older comments reads as `issue (blocking):`. Write the new form only.

## How to read a comment

| Comment | What to do |
|---|---|
| `issue`, `chore`, `todo`, `typo`, or any label with `(blocking)` | Fix it before merge. |
| `nitpick`, `thought`, `polish`, or any label with `(non-blocking)` | Optional. Fix it when it is cheap. |
| `question` | Answer it. |
| `suggestion` | Judge it and accept or decline with a reason. |

## Examples

```
issue (blocking, security): The export endpoint returns every tenant's rows.

The query has no tenant filter, so any signed-in user can read other tenants' data.
Add a tenant filter to the query and pass the tenant from the session.
```

```
suggestion (non-blocking, perf): Load the prices once before the loop.

Today the loop runs one query per order line. For an order with 500 lines that is 500 round trips.
One query with a list of ids returns the same data in one call.
```

Finding codes such as F1 and F2 stay internal IDs in a report. The posted comment text follows this format.
