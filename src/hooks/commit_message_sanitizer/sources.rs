// SPDX-FileCopyrightText: 2026 Igor Santos
// SPDX-License-Identifier: MIT

//! Finds where a command keeps a message and writes the sanitised text back
//! to the same place: a shell word, a heredoc body, a file on disk, or the
//! `printf` or `echo` that produces it.

use super::engine::{Call, Findings, Plan};
use crate::common::atomic::write_atomic;
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
    /// literal. When nothing is left of the text, `whole` (the option that
    /// held it) is removed instead.
    Word {
        range: Range<usize>,
        whole: Option<Range<usize>>,
    },
    /// The lines of a heredoc body.
    Body(Range<usize>),
    /// A file on disk, rewritten in place.
    File(PathBuf),
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
    let skip = inline_at.unwrap_or(0);
    let text: String = word.chars().skip(skip).collect();
    if !span.dynamic {
        let start = span.start + skip;
        let unquoted_prefix =
            call.raw(span.start..start) == word.chars().take(skip).collect::<String>();
        if !unquoted_prefix {
            findings.unread.push(label.to_string());
            return Vec::new();
        }
        return vec![Source {
            label: label.to_string(),
            text,
            target: Target::Word {
                range: start..span.end,
                whole: None,
            },
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
/// the command before a pipe prints.
pub fn from_stdin(call: &Call, label: &str) -> Vec<Source> {
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
    match call.index.checked_sub(1).filter(|_| cmd.piped) {
        Some(before) => producer_chain(call, before, label),
        None => Vec::new(),
    }
}

fn producer_chain(call: &Call, before: usize, label: &str) -> Vec<Source> {
    producer(&call.cmds[before], 0, label)
}

/// A message in a file: standard input for `-`, or the file on disk.
pub fn from_path(
    call: &Call,
    path: &str,
    dynamic: bool,
    label: &str,
    findings: &mut Findings,
) -> Vec<Source> {
    if is_stdin_path(path) {
        return from_stdin(call, label);
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
    match read_file(&file) {
        Some(text) => vec![Source {
            label: named,
            text,
            target: Target::File(file),
        }],
        None => {
            findings.unread.push(named);
            Vec::new()
        }
    }
}

fn read_file(path: &Path) -> Option<String> {
    let mut bytes = Vec::new();
    File::open(path)
        .ok()?
        .take(MAX_MESSAGE_BYTES)
        .read_to_end(&mut bytes)
        .ok()?;
    Some(String::from_utf8_lossy(&bytes).into_owned())
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
            Some(option) if text.trim().is_empty() => {
                plan.edits.push((option.clone(), String::new()))
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
        Target::File(path) => {
            if write_atomic(path, &text).is_err() {
                findings
                    .unread
                    .push(format!("{} (could not rewrite)", source.label));
            }
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
