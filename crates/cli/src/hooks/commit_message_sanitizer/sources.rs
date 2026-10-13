// SPDX-FileCopyrightText: 2026 Igor Santos
// SPDX-License-Identifier: Apache-2.0

//! Finds where a command keeps a message and writes the sanitised text back
//! to the same place: a shell word, a heredoc body, or the `printf` or `echo`
//! that produces it. A message in a file or in the repository is never
//! rewritten there: the command is made to read the cleaned text from a
//! heredoc instead. When there is no safe place for that heredoc, the message
//! is reported as unread and left to the backstop.

use super::engine::{Call, Findings, Inline, Plan};
use crate::common::attribution::{drop_lines, problems, prose_problems, Shape};
use crate::common::home_dir;
use crate::common::shell::{commands, program_index, program_name, quote, unescape, Command, Span};
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
    /// Text that lives outside the command, in a file or the repository, read
    /// from a heredoc in its place so the original is never touched.
    /// `replace` is what stands for it now, which becomes `opener` and a `<<`
    /// operator, and the body goes at `slot` after its lead. There is no slot
    /// when a heredoc would not be safe there, and the error says why.
    Feed {
        replace: Range<usize>,
        opener: String,
        slot: Result<Slot, &'static str>,
    },
    /// The format of a `printf`, or the text of `echo -e`, which read
    /// backslashes as escapes.
    Escaped { range: Range<usize>, percent: bool },
}

/// Where the body of a fed heredoc goes.
pub struct Slot {
    /// An empty range, after the line break that ends the command's line.
    at: Range<usize>,
    /// What goes before the body: a line break when the script ends there.
    lead: &'static str,
    /// The delimiter of the heredoc this one sits inside, which its own
    /// terminator must differ from.
    avoid: Option<String>,
}

pub struct Source {
    pub label: String,
    pub text: String,
    pub target: Target,
}

/// Which rule judges the text.
#[derive(Clone, Copy)]
pub enum Rule {
    /// A commit message, with its trailer block.
    Commit,
    /// A PR title or body, which has no trailer block.
    Prose,
}

fn is_stdin_path(path: &str) -> bool {
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
    let place = Place {
        call,
        base: span.start + start,
        inside: true,
        before: call.index,
    };
    let sources = producers_in(&inner, place, label);
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

/// Where the script that holds a producer sits in the script of the call that
/// reads its output.
#[derive(Clone, Copy)]
struct Place<'a> {
    call: &'a Call<'a>,
    /// Characters of the call's script before it: 0 for the call's own.
    base: usize,
    /// It is the inside of a `$(...)` in a word of the call.
    inside: bool,
    /// How many commands of the call's script come before the producer.
    before: usize,
}

/// The messages a script produces: `cat` with a heredoc or files, `printf`,
/// `echo`.
fn producers_in(script: &str, place: Place, label: &str) -> Vec<Source> {
    commands(script)
        .iter()
        .flat_map(|cmd| {
            let end = cmd.spans.last().map_or(0, |s| s.end);
            let tail_blank = script.chars().skip(end).all(char::is_whitespace);
            producer(cmd, place, tail_blank, label)
        })
        .collect()
}

fn producer(cmd: &Command, place: Place, tail_blank: bool, label: &str) -> Vec<Source> {
    let Some(at) = program_index(&cmd.words) else {
        return Vec::new();
    };
    let args = &cmd.words[at + 1..];
    let spans = &cmd.spans[at + 1..];
    let base = place.base;
    match program_name(&cmd.words[at]).as_str() {
        "cat" if cmd.heredocs.is_empty() => cat_files(args, spans, place, tail_blank, label),
        "cat" => cmd
            .heredocs
            .iter()
            .map(|h| Source {
                label: label.to_string(),
                text: h.text.clone(),
                target: Target::Body(shift(&h.body, base)),
            })
            .collect(),
        "printf" if args.len() == 1 && !spans[0].dynamic => {
            let format = unescape(&args[0]).replace("%%", "%");
            if format.contains('%') {
                return Vec::new();
            }
            vec![Source {
                label: label.to_string(),
                text: format,
                target: Target::Escaped {
                    range: shift(&(spans[0].start..spans[0].end), base),
                    percent: true,
                },
            }]
        }
        "echo" => echo(args, spans, base, label),
        _ => Vec::new(),
    }
}

/// The text of the files a `cat` prints, read from a heredoc instead. Nothing
/// when any of them cannot be fed safely, which leaves it to the backstop.
fn cat_files(
    args: &[String],
    spans: &[Span],
    place: Place,
    tail_blank: bool,
    label: &str,
) -> Vec<Source> {
    let (Some(first), Some(last)) = (spans.first(), spans.last()) else {
        return Vec::new();
    };
    if args
        .iter()
        .zip(spans)
        .any(|(a, s)| a.starts_with('-') || s.dynamic)
    {
        return Vec::new();
    }
    let mut text = String::new();
    for arg in args {
        let file = resolve(arg, place.call.dir);
        match read_message(&file) {
            Ok(part) if !touched_before(place.call, &file, place.before) => text.push_str(&part),
            _ => return Vec::new(),
        }
    }
    let slot = if place.inside {
        // Inside a substitution the body follows the command, before its `)`.
        host_delimiter(place.call.inline, &text).and_then(|avoid| {
            tail_blank
                .then(|| Slot {
                    at: shift(&(last.end..last.end), place.base),
                    lead: "\n",
                    avoid,
                })
                .ok_or("its command is not the last in its substitution")
        })
    } else {
        slot_after_line(place.call, &text, false)
    };
    vec![Source {
        label: format!("{label} in {}", args.join(" ")),
        text,
        target: Target::Feed {
            replace: shift(&(first.start..last.end), place.base),
            opener: String::new(),
            slot,
        },
    }]
}

/// `echo` with one text word, and `-e` when the text reads escapes.
fn echo(args: &[String], spans: &[Span], base: usize, label: &str) -> Vec<Source> {
    let is_flag =
        |a: &String| a.len() > 1 && a.starts_with('-') && a[1..].chars().all(|c| "neE".contains(c));
    let flags: Vec<&String> = args.iter().take_while(|a| is_flag(a)).collect();
    let rest = &args[flags.len()..];
    if rest.len() != 1 || spans[flags.len()].dynamic {
        return Vec::new();
    }
    let reads_escapes = flags.iter().any(|f| f.contains('e'));
    let literal = flags.iter().any(|f| f.contains('E'));
    // Without a flag, whether a backslash is an escape depends on the shell.
    if rest[0].contains('\\') && !reads_escapes && !literal {
        return Vec::new();
    }
    let span = &spans[flags.len()];
    let text = if reads_escapes {
        unescape(&rest[0])
    } else {
        rest[0].clone()
    };
    let range = shift(&(span.start..span.end), base);
    vec![Source {
        label: label.to_string(),
        text,
        target: if reads_escapes {
            Target::Escaped {
                range,
                percent: false,
            }
        } else {
            Target::Word { range, whole: None }
        },
    }]
}

fn shift(range: &Range<usize>, base: usize) -> Range<usize> {
    range.start + base..range.end + base
}

/// A message read from standard input: the command's own heredoc, a here
/// string, a file redirected in, or what the command before a pipe prints.
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
    if let Some(at) = cmd.words.iter().position(|w| w == "<<<") {
        return if at + 1 < cmd.words.len() {
            from_word(call, at + 1, None, label, findings)
        } else {
            Vec::new()
        };
    }
    let redirected = cmd
        .words
        .iter()
        .enumerate()
        .find_map(|(i, w)| match w.as_str() {
            "<" => cmd.words.get(i + 1).map(|path| Named {
                path,
                dynamic: cmd.spans[i + 1].dynamic,
                replace: Some(cmd.spans[i].start..cmd.spans[i + 1].end),
                opener: "",
            }),
            w if w.starts_with('<') => Some(Named {
                path: &w[1..],
                dynamic: cmd.spans[i].dynamic,
                replace: Some(cmd.spans[i].start..cmd.spans[i].end),
                opener: "",
            }),
            _ => None,
        });
    if let Some(named) = redirected {
        return from_path(call, &named, label, findings);
    }
    let Some(before) = call.index.checked_sub(1).filter(|_| cmd.piped) else {
        findings.unread.push(format!(
            "{label} (standard input is not a heredoc or a pipe)"
        ));
        return Vec::new();
    };
    let place = Place {
        call,
        base: 0,
        inside: false,
        before,
    };
    let sources = producer(&call.cmds[before], place, true, label);
    if sources.is_empty() {
        findings.unread.push(format!("{label} (read from a pipe)"));
    }
    sources
}

/// A file named in the command, and where its name sits so a heredoc can
/// stand in for it.
pub struct Named<'a> {
    pub path: &'a str,
    pub dynamic: bool,
    /// What stands for the file now, which is replaced when the message is
    /// fed from a heredoc. `None` when it is not written plainly.
    pub replace: Option<Range<usize>>,
    /// What the replacement starts with: `- ` for the value of `-F`, nothing
    /// for a file read by a redirect or a `cat`.
    pub opener: &'static str,
}

/// A message in a file: standard input for `-`, the heredoc that an earlier
/// command in the script wrote the file from, or the file on disk, whose text
/// is read from a heredoc in its place.
pub fn from_path(call: &Call, named: &Named, label: &str, findings: &mut Findings) -> Vec<Source> {
    let path = named.path;
    if is_stdin_path(path) {
        return from_stdin(call, label, findings);
    }
    if let Some(sources) = written_earlier(call, path, label) {
        return sources;
    }
    let described = format!("{label} in {path}");
    let expanded = if named.dynamic {
        expand_vars(path)
    } else {
        Some(path.to_string())
    };
    let Some(file) = expanded.map(|p| resolve(&p, call.dir)) else {
        findings.unread.push(described);
        return Vec::new();
    };
    match fed_from(call, &file, named) {
        Ok(text) => {
            let replace = named.replace.clone().unwrap_or_default();
            vec![feed(
                call,
                described,
                text,
                replace,
                named.opener.to_string(),
            )]
        }
        Err(why) => {
            findings.unread.push(format!("{described} ({why})"));
            Vec::new()
        }
    }
}

/// The text of `file`, or why it is left to the backstop: it must be a
/// regular UTF-8 file under the size cap, and no earlier command in the script
/// may touch it, since its text when the command runs is then not what is read
/// here.
fn fed_from(call: &Call, file: &Path, named: &Named) -> Result<String, &'static str> {
    let text = read_message(file)?;
    if touched_before(call, file, call.index) {
        return Err("it is changed earlier in the same command");
    }
    named.replace.as_ref().ok_or("its name is partly quoted")?;
    Ok(text)
}

/// A source whose text is read from a heredoc placed after the command's line.
/// A non-empty `opener` makes the heredoc the command's standard input.
pub fn feed(
    call: &Call,
    label: String,
    text: String,
    replace: Range<usize>,
    opener: String,
) -> Source {
    let slot = slot_after_line(call, &text, !opener.is_empty());
    Source {
        label,
        text,
        target: Target::Feed {
            replace,
            opener,
            slot,
        },
    }
}

/// Whether any of the first `before` commands of the script names `file`.
fn touched_before(call: &Call, file: &Path, before: usize) -> bool {
    call.cmds[..before].iter().any(|cmd| {
        cmd.words
            .iter()
            .map(|word| word.trim_start_matches(['<', '>']))
            .any(|word| !word.is_empty() && resolve(word, call.dir) == file)
    })
}

/// Where the heredoc that stands in for `text` goes, or why none can: a
/// heredoc that is the command's standard input needs the command to have no
/// other input and no carrier such as `xargs` to take it, and the text must
/// survive the heredoc it is written into, if any.
fn slot_after_line(call: &Call, text: &str, takes_stdin: bool) -> Result<Slot, &'static str> {
    if takes_stdin
        && (!call.cmd().heredocs.is_empty() || redirects_stdin(call) || call.under_stdin_carrier())
    {
        return Err("standard input is already in use or taken by a carrier such as xargs");
    }
    let avoid = host_delimiter(call.inline, text)?;
    let (at, lead) = line_slot(call).ok_or("another heredoc is open on its line")?;
    Ok(Slot { at, lead, avoid })
}

/// The delimiter of the heredoc the fed text would sit in, when there is one
/// and the text can sit there: its shell must not expand it, no line may end
/// that heredoc early, and `<<-` must have no tab to strip.
fn host_delimiter(inline: Inline, text: &str) -> Result<Option<String>, &'static str> {
    match inline {
        Inline::Anywhere => Ok(None),
        Inline::Nowhere => Err("it would sit in a heredoc inside an outer heredoc"),
        Inline::InHeredoc(host) => {
            let changed = text.lines().any(|line| {
                line.trim_end_matches('\r') == host.delimiter
                    || (host.strip_tabs && line.starts_with('\t'))
            });
            if !host.quoted || changed {
                return Err("it would sit in an outer heredoc that expands it or could end early");
            }
            Ok(Some(host.delimiter.clone()))
        }
    }
}

/// Whether the command redirects its standard input with `<` or `<<<`, as
/// opposed to a heredoc, which the lexer keeps apart.
fn redirects_stdin(call: &Call) -> bool {
    call.cmd().spans.iter().any(|span| {
        call.raw(span.start..span.end)
            .trim_start_matches(|c: char| c.is_ascii_digit())
            .starts_with('<')
    })
}

/// Where the body of a heredoc the command opens goes, as an empty range: right
/// after the line break that ends the command's logical line, found outside
/// quotes, substitutions and line continuations, with a line break of its own
/// first when the script ends there. `None` when another heredoc is already
/// open on that line.
fn line_slot(call: &Call) -> Option<(Range<usize>, &'static str)> {
    let line_end = call.cmd().line_end;
    let busy = call
        .cmds
        .iter()
        .any(|c| c.line_end == line_end && !c.heredocs.is_empty());
    if busy {
        return None;
    }
    Some(match line_end {
        Some(at) => (at + 1..at + 1, ""),
        None => (call.chars.len()..call.chars.len(), "\n"),
    })
}

/// A heredoc terminator that no line of `text` equals, and that is not `avoid`.
fn unique_delimiter(text: &str, avoid: Option<&str>) -> String {
    let mut delimiter = "PLAYBOOK_MESSAGE_END".to_string();
    while text.lines().any(|line| line == delimiter) || avoid == Some(delimiter.as_str()) {
        delimiter.push('_');
    }
    delimiter
}

/// The heredoc of an earlier command that redirects its output to `path`.
fn written_earlier(call: &Call, path: &str, label: &str) -> Option<Vec<Source>> {
    call.cmds[..call.index].iter().rev().find_map(|cmd| {
        if cmd.heredocs.is_empty() || !writes_to(cmd, path) {
            return None;
        }
        let sources = cmd
            .heredocs
            .iter()
            .map(|h| Source {
                label: format!("{label} in {path}"),
                text: h.text.clone(),
                target: Target::Body(h.body.clone()),
            })
            .collect();
        Some(sources)
    })
}

fn writes_to(cmd: &Command, path: &str) -> bool {
    let words = &cmd.words;
    let tee = program_index(words).is_some_and(|at| program_name(&words[at]) == "tee");
    words.iter().enumerate().any(|(i, w)| {
        let attached = w.strip_prefix(">>").or_else(|| w.strip_prefix('>'));
        attached == Some(path)
            || ((w == ">" || w == ">>") && words.get(i + 1).map(String::as_str) == Some(path))
            || (tee && w == path)
    })
}

/// The text of a regular, UTF-8 file within the size cap.
pub fn read_message(path: &Path) -> Result<String, &'static str> {
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

/// The text of `sources` as one message, the way git joins repeated `-m`.
pub fn joined(sources: &[Source]) -> String {
    sources
        .iter()
        .map(|s| s.text.trim_end_matches('\n'))
        .collect::<Vec<_>>()
        .join("\n\n")
}

/// Sanitises `sources` and queues the rewrite of each one that changes. With
/// `joined`, the sources are one message, the way git joins repeated `-m`.
pub fn apply(
    sources: &[Source],
    rule: Rule,
    joined: bool,
    plan: &mut Plan,
    findings: &mut Findings,
) {
    let trimmed: Vec<&str> = sources
        .iter()
        .map(|s| s.text.trim_end_matches('\n'))
        .collect();
    let groups: Vec<Vec<usize>> = if joined {
        vec![(0..sources.len()).collect()]
    } else {
        (0..sources.len()).map(|i| vec![i]).collect()
    };
    for group in groups {
        let text = group
            .iter()
            .map(|&i| trimmed[i])
            .collect::<Vec<_>>()
            .join("\n\n");
        let removed = match rule {
            Rule::Commit => problems(&text),
            Rule::Prose => prose_problems(&text),
        };
        let mut first_line = 1;
        for &i in &group {
            let lines = trimmed[i].split('\n').count();
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
                rewrite(&sources[i], &local, &numbered, plan, findings);
            }
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
    if let Target::Feed { slot: Err(why), .. } = &source.target {
        findings.unread.push(format!("{} ({why})", source.label));
        return;
    }
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
            opener,
            slot: Ok(slot),
        } => {
            let body = if text.is_empty() || text.ends_with('\n') {
                text
            } else {
                format!("{text}\n")
            };
            let delimiter = unique_delimiter(&body, slot.avoid.as_deref());
            plan.edits
                .push((replace.clone(), format!("{opener}<<'{delimiter}'")));
            plan.edits
                .push((slot.at.clone(), format!("{}{body}{delimiter}\n", slot.lead)));
        }
        Target::Feed { slot: Err(_), .. } => {}
        Target::Escaped { range, percent } => {
            let mut escaped = text.replace('\\', "\\\\");
            if *percent {
                escaped = escaped.replace('%', "%%");
            }
            plan.edits.push((range.clone(), quote(&escaped)));
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
