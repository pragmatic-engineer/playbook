// SPDX-FileCopyrightText: 2026 Igor Santos
// SPDX-License-Identifier: MIT

//! Finds where a command keeps a message and writes the sanitised text back
//! to the same place: a shell word or a heredoc body. A message in a file is
//! never rewritten on disk: the command is made to read the cleaned text from
//! a heredoc instead, and the file stays as it is.

use super::engine::{Call, Findings, Plan};
use crate::common::attribution::{drop_lines, problems, Shape};
use crate::common::home_dir;
use crate::common::shell::{commands, program_index, program_name, quote, Command};
use std::fs::File;
use std::io::Read;
use std::ops::Range;
use std::path::{Path, PathBuf};

/// Longest message file read, so a stray path to a huge file stays cheap.
const MAX_MESSAGE_BYTES: u64 = 1 << 20;

/// Paths that name the standard input of the command that reads them.
const STDIN_PATHS: [&str; 4] = ["-", "/dev/stdin", "/dev/fd/0", "/proc/self/fd/0"];

/// How the sanitised text is written back.
pub enum Target {
    /// A shell word, or the value inside one: the range becomes a quoted
    /// literal. When nothing is left of the text, the range of `whole` (the
    /// option that held it) is replaced by what is left of that option, which
    /// is the other letters of a cluster such as `-nm`.
    Word {
        range: Range<usize>,
        whole: Option<(Range<usize>, String)>,
    },
    /// The lines of a heredoc body.
    Body(Range<usize>),
    /// The text of a file, read from a heredoc in its place. `replace` is the
    /// file name, which becomes `-` and a `<<` operator, and the body goes at
    /// `slot` after `lead`.
    Feed {
        replace: Range<usize>,
        slot: Range<usize>,
        lead: &'static str,
    },
}

pub struct Source {
    pub label: String,
    pub text: String,
    pub target: Target,
}

pub fn is_stdin_path(path: &str) -> bool {
    STDIN_PATHS.contains(&path)
}

/// `path` as an absolute path: `~` expands, a relative path joins `base`.
pub fn resolve(path: &str, base: &Path) -> PathBuf {
    let expanded = match path.strip_prefix("~/") {
        Some(rest) => home_dir().join(rest),
        None => PathBuf::from(path),
    };
    base.join(expanded)
}

/// `text` with `$VAR` and `${VAR}` read from this process's environment, or
/// `None` when a variable is not set.
fn expand_vars(text: &str) -> Option<String> {
    let mut out = String::new();
    let mut rest = text;
    while let Some(at) = rest.find('$') {
        out.push_str(&rest[..at]);
        let after = &rest[at + 1..];
        let (name, tail) = match after.strip_prefix('{') {
            Some(braced) => braced.split_once('}')?,
            None => {
                let end = after
                    .find(|c: char| !c.is_ascii_alphanumeric() && c != '_')
                    .unwrap_or(after.len());
                (&after[..end], &after[end..])
            }
        };
        if name.is_empty() {
            return None;
        }
        out.push_str(&std::env::var(name).ok()?);
        rest = tail;
    }
    out.push_str(rest);
    Some(out)
}

/// The message in the word at `at` in the command, or in the part of it after
/// `inline_at` characters. A word the shell expands is read through the
/// `$(...)` or backticks it holds, since only those can be rewritten.
pub fn from_word(
    call: &Call,
    at: usize,
    inline_at: Option<usize>,
    label: &str,
    findings: &mut Findings,
) -> Vec<Source> {
    let word = &call.cmd().words[at];
    let span = &call.cmd().spans[at];
    let text: String = word.chars().skip(inline_at.unwrap_or(0)).collect();
    if !span.dynamic {
        let Some(range) = value_span(call, at, inline_at) else {
            findings.unread.push(label.to_string());
            return Vec::new();
        };
        return vec![Source {
            label: label.to_string(),
            text,
            target: Target::Word { range, whole: None },
        }];
    }
    let raw = call.raw(span.start..span.end);
    let found = substitution(&raw);
    let Some((inner, start)) = found else {
        findings.unread.push(label.to_string());
        return Vec::new();
    };
    let sources = producers_in(&inner, span.start + start, label);
    if sources.is_empty() {
        findings.unread.push(label.to_string());
    }
    sources
}

/// Where the value of the option in the word at `at` sits in the script, past
/// its first `inline_at` characters, when those are written without quotes.
pub fn value_span(call: &Call, at: usize, inline_at: Option<usize>) -> Option<Range<usize>> {
    let span = call.span(at);
    let skip = inline_at.unwrap_or(0);
    let start = span.start + skip;
    let prefix: String = call.cmd().words[at].chars().take(skip).collect();
    (call.raw(span.start..start) == prefix).then_some(start..span.end)
}

/// The script inside the first `$(` and the last `)` of a word, or inside its
/// first and last backtick, with where it starts in the word.
fn substitution(raw: &str) -> Option<(String, usize)> {
    let chars: Vec<char> = raw.chars().collect();
    let dollar = chars.windows(2).position(|w| w == ['$', '(']);
    let (start, end) = match dollar {
        Some(at) => (at + 2, chars.iter().rposition(|&c| c == ')')?),
        None => (
            chars.iter().position(|&c| c == '`')? + 1,
            chars.iter().rposition(|&c| c == '`')?,
        ),
    };
    (start <= end).then(|| (chars[start..end].iter().collect(), start))
}

/// The messages a script produces: the heredocs of its `cat` commands.
fn producers_in(script: &str, base: usize, label: &str) -> Vec<Source> {
    commands(script)
        .iter()
        .flat_map(|cmd| producer(cmd, base, label))
        .collect()
}

fn producer(cmd: &Command, base: usize, label: &str) -> Vec<Source> {
    let is_cat = program_index(&cmd.words).is_some_and(|at| program_name(&cmd.words[at]) == "cat");
    if !is_cat {
        return Vec::new();
    }
    cmd.heredocs
        .iter()
        .map(|h| Source {
            label: label.to_string(),
            text: h.text.clone(),
            target: Target::Body(shift(&h.body, base)),
        })
        .collect()
}

fn shift(range: &Range<usize>, base: usize) -> Range<usize> {
    range.start + base..range.end + base
}

/// A message read from standard input: the command's own heredoc, or what
/// the command before a pipe prints. A pipe from a command that is not read
/// is reported as unread.
pub fn from_stdin(call: &Call, label: &str, findings: &mut Findings) -> Vec<Source> {
    let cmd = call.cmd();
    if !cmd.heredocs.is_empty() {
        return cmd
            .heredocs
            .iter()
            .map(|h| Source {
                label: label.to_string(),
                text: h.text.clone(),
                target: Target::Body(h.body.clone()),
            })
            .collect();
    }
    let Some(before) = call.index.checked_sub(1).filter(|_| cmd.piped) else {
        return Vec::new();
    };
    let sources = producer_chain(call, before, label);
    if sources.is_empty() {
        findings.unread.push(format!("{label} (read from a pipe)"));
    }
    sources
}

fn producer_chain(call: &Call, before: usize, label: &str) -> Vec<Source> {
    producer(&call.cmds[before], 0, label)
}

/// A message in a file: standard input for `-`, or the file on disk, whose
/// text is read from a heredoc in its place. `value` is where the file name
/// sits in the script.
pub fn from_path(
    call: &Call,
    path: &str,
    dynamic: bool,
    value: Option<Range<usize>>,
    label: &str,
    findings: &mut Findings,
) -> Vec<Source> {
    if is_stdin_path(path) {
        return from_stdin(call, label, findings);
    }
    let named = format!("{label} in {path}");
    let expanded = if dynamic {
        expand_vars(path)
    } else {
        Some(path.to_string())
    };
    let Some(file) = expanded.map(|p| resolve(&p, call.dir)) else {
        findings.unread.push(named);
        return Vec::new();
    };
    match fed_from(call, &file, value) {
        Ok(source) => vec![Source {
            label: named,
            ..source
        }],
        Err(why) => {
            findings.unread.push(format!("{named} ({why})"));
            Vec::new()
        }
    }
}

/// The text of `file` with the heredoc that stands in for it, or why the file
/// is left to the backstop: it must be a regular UTF-8 file under the size
/// cap, and no earlier command in the script may touch it, since its text at
/// the time the command runs is then not what is read here.
fn fed_from(call: &Call, file: &Path, value: Option<Range<usize>>) -> Result<Source, &'static str> {
    let text = read_message(file)?;
    if touched_earlier(call, file) {
        return Err("it is changed earlier in the same command");
    }
    let replace = value.ok_or("its name is partly quoted")?;
    let (slot, lead) = line_slot(call, call.end()).ok_or("another heredoc is open on its line")?;
    Ok(Source {
        label: String::new(),
        text,
        target: Target::Feed {
            replace,
            slot,
            lead,
        },
    })
}

/// Whether a command before this one in the script names `file`.
fn touched_earlier(call: &Call, file: &Path) -> bool {
    call.cmds[..call.index].iter().any(|cmd| {
        cmd.words
            .iter()
            .map(|word| word.trim_start_matches(['<', '>']))
            .any(|word| !word.is_empty() && resolve(word, call.dir) == file)
    })
}

/// Where the body of a heredoc for a command that ends at `from` goes, as an
/// empty range: after the line break that ends its line, with a line break of
/// its own first when the script ends there. `None` when another heredoc is
/// already open on that line.
fn line_slot(call: &Call, from: usize) -> Option<(Range<usize>, &'static str)> {
    let start = call.chars[..from]
        .iter()
        .rposition(|&c| c == '\n')
        .map_or(0, |at| at + 1);
    let end = call.chars[from..]
        .iter()
        .position(|&c| c == '\n')
        .map(|at| from + at);
    let stop = end.unwrap_or(call.chars.len());
    let busy = call.cmds.iter().any(|c| {
        !c.heredocs.is_empty()
            && c.spans
                .first()
                .is_some_and(|s| (start..=stop).contains(&s.start))
    });
    if busy {
        return None;
    }
    Some(match end {
        Some(at) => (at + 1..at + 1, ""),
        None => (stop..stop, "\n"),
    })
}

/// A heredoc terminator that no line of `text` equals.
fn unique_delimiter(text: &str) -> String {
    let mut delimiter = "PLAYBOOK_MESSAGE_END".to_string();
    while text.lines().any(|line| line == delimiter) {
        delimiter.push('_');
    }
    delimiter
}

/// The text of a regular, UTF-8 file within the size cap.
fn read_message(path: &Path) -> Result<String, &'static str> {
    let meta = std::fs::symlink_metadata(path).map_err(|_| "it cannot be read")?;
    if meta.file_type().is_symlink() {
        return Err("it is a symbolic link");
    }
    if !meta.is_file() || meta.len() > MAX_MESSAGE_BYTES {
        return Err("it is not a regular file under 1 MiB");
    }
    let mut bytes = Vec::new();
    File::open(path)
        .and_then(|file| file.take(MAX_MESSAGE_BYTES).read_to_end(&mut bytes))
        .map_err(|_| "it cannot be read")?;
    String::from_utf8(bytes).map_err(|_| "it is not valid UTF-8")
}

/// Sanitises `sources` as one message, the way git joins repeated `-m`, and
/// queues the rewrite of each one that changes.
pub fn apply(sources: Vec<Source>, plan: &mut Plan, findings: &mut Findings) {
    let trimmed: Vec<&str> = sources
        .iter()
        .map(|s| s.text.trim_end_matches('\n'))
        .collect();
    let removed = problems(&trimmed.join("\n\n"));
    let mut first_line = 1;
    for (source, text) in sources.iter().zip(&trimmed) {
        let lines = text.split('\n').count();
        let own = first_line..first_line + lines;
        first_line += lines + 1;
        let numbered: Vec<(usize, Shape)> = removed
            .iter()
            .filter(|(n, _)| own.contains(n))
            .copied()
            .collect();
        if !numbered.is_empty() {
            let local: Vec<(usize, Shape)> = numbered
                .iter()
                .map(|&(n, shape)| (n + 1 - own.start, shape))
                .collect();
            rewrite(source, &local, &numbered, plan, findings);
        }
    }
}

/// Queues the rewrite of `source` without the lines `removed` names, which are
/// reported as `numbered`, by their place in the whole message.
fn rewrite(
    source: &Source,
    removed: &[(usize, Shape)],
    numbered: &[(usize, Shape)],
    plan: &mut Plan,
    findings: &mut Findings,
) {
    let text = drop_lines(&source.text, removed);
    for (n, shape) in numbered {
        findings
            .notes
            .push(format!("{} line {n} ({})", source.label, shape.name()));
    }
    match &source.target {
        Target::Word { range, whole } => match whole {
            Some((option, kept)) if text.trim().is_empty() => {
                plan.edits.push((option.clone(), kept.clone()))
            }
            _ => plan.edits.push((range.clone(), quote(&text))),
        },
        Target::Body(range) => {
            let body = if text.is_empty() || text.ends_with('\n') {
                text
            } else {
                format!("{text}\n")
            };
            plan.edits.push((range.clone(), body));
        }
        Target::Feed {
            replace,
            slot,
            lead,
        } => {
            let body = if text.is_empty() || text.ends_with('\n') {
                text
            } else {
                format!("{text}\n")
            };
            let delimiter = unique_delimiter(&body);
            plan.edits
                .push((replace.clone(), format!("- <<'{delimiter}'")));
            plan.edits
                .push((slot.clone(), format!("{lead}{body}{delimiter}\n")));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn expand_vars_reads_set_variables_and_refuses_unset_ones() {
        std::env::set_var("SANITIZER_TEST_DIR", "/tmp/x");

        assert_eq!(
            expand_vars("$SANITIZER_TEST_DIR/m.txt").as_deref(),
            Some("/tmp/x/m.txt")
        );
        assert_eq!(
            expand_vars("${SANITIZER_TEST_DIR}/m").as_deref(),
            Some("/tmp/x/m")
        );
        assert_eq!(expand_vars("$SANITIZER_TEST_UNSET_VAR/m"), None);
        assert_eq!(expand_vars("plain").as_deref(), Some("plain"));
    }

    #[test]
    fn stdin_paths_are_recognised() {
        for path in ["-", "/dev/stdin", "/dev/fd/0", "/proc/self/fd/0"] {
            assert!(is_stdin_path(path), "{path}");
        }
        assert!(!is_stdin_path("/dev/null"));
    }

    #[test]
    fn a_substitution_is_the_text_between_dollar_paren_and_the_last_paren() {
        let raw = "\"$(cat <<'EOF'\nit's (fine)\nEOF\n)\"";

        let (inner, start) = substitution(raw).expect("a substitution");

        assert_eq!(inner, "cat <<'EOF'\nit's (fine)\nEOF\n");
        assert_eq!(start, 3);
    }
}
