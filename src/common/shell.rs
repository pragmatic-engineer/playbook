// SPDX-FileCopyrightText: 2026 Igor Santos
// SPDX-License-Identifier: MIT

//! A small, quote-aware reader for shell command lines, shared by the hooks
//! that need to see which program a Bash call runs and with what words.
//!
//! It splits on unquoted separators into simple commands, removes quotes and
//! keeps where each word sits in the original text, so a caller can rewrite
//! one word in place. Each command also keeps the heredoc bodies opened on its
//! line. It does not expand variables, aliases, functions or substitutions,
//! so every caller is a guardrail against drift rather than a security
//! boundary.

use std::ops::Range;
use std::path::Path;

const SHELL_KEYWORDS: [&str; 10] = [
    "if", "then", "else", "elif", "while", "until", "do", "!", "{", "}",
];
const WRAPPERS: [&str; 7] = ["env", "command", "exec", "time", "nice", "timeout", "sudo"];

/// The index of the word that names the program: past leading assignments,
/// redirections, shell keywords and wrappers such as `env` or `sudo`.
pub fn program_index(words: &[String]) -> Option<usize> {
    let mut at = 0;
    while let Some(word) = words.get(at) {
        let redirect = redirect_len(word);
        if redirect > 0 {
            at += redirect;
        } else if is_assignment(word) || SHELL_KEYWORDS.contains(&word.as_str()) {
            at += 1;
        } else if WRAPPERS.contains(&program_name(word).as_str()) {
            at = after_wrapper(words, at);
        } else {
            return Some(at);
        }
    }
    None
}

/// The index just past the wrapper at `at` and its own options. Always moves
/// forward.
fn after_wrapper(words: &[String], at: usize) -> usize {
    let wrapper = program_name(&words[at]);
    let takes_value: &[&str] = match wrapper.as_str() {
        "env" => &["-u", "-C", "--unset", "--chdir"],
        "exec" => &["-a"],
        "nice" => &["-n", "--adjustment"],
        "timeout" => &["-s", "-k", "--signal", "--kill-after"],
        "sudo" => &[
            "-u", "-g", "-h", "-p", "-C", "-D", "-R", "-T", "-r", "-t", "-U",
        ],
        _ => &[],
    };
    let mut next = at + 1;
    while let Some(word) = words.get(next) {
        if takes_value.contains(&word.as_str()) {
            next += 2;
        } else if word == "--" {
            next += 1;
            break;
        } else if (word.len() > 1 && word.starts_with('-'))
            || (wrapper == "env" && is_assignment(word))
        {
            next += 1;
        } else {
            break;
        }
    }
    if wrapper == "timeout" {
        next += 1;
    }
    next.min(words.len())
}

/// The words a redirection takes: the operator word, plus its target when
/// the operator stands alone.
fn redirect_len(word: &str) -> usize {
    let operator = word.trim_start_matches(|c: char| c.is_ascii_digit());
    if !operator.starts_with(['<', '>']) {
        0
    } else if operator.chars().all(|c| c == '<' || c == '>') {
        2
    } else {
        1
    }
}

/// The lowercase file name of `word`, so `/usr/bin/Playbook` is `playbook`.
pub fn program_name(word: &str) -> String {
    Path::new(word)
        .file_name()
        .map_or_else(String::new, |name| name.to_string_lossy().to_lowercase())
}

pub fn is_assignment(word: &str) -> bool {
    word.split_once('=').is_some_and(|(name, _)| {
        !name.is_empty() && name.chars().all(|c| c.is_ascii_alphanumeric() || c == '_')
    })
}

/// Where a word sits in the text it was read from, in characters.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Span {
    pub start: usize,
    pub end: usize,
    /// The word holds `$` or a backtick outside single quotes, so its value is
    /// only known to the shell that runs it.
    pub dynamic: bool,
}

/// A heredoc body opened on a command's line.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Heredoc {
    pub delimiter: String,
    /// The body is not expanded, because the delimiter was quoted.
    pub quoted: bool,
    /// The body's lines, with leading tabs removed for `<<-`.
    pub text: String,
    /// The body's lines in the source, up to the delimiter line.
    pub body: Range<usize>,
}

/// One simple command.
#[derive(Debug)]
pub struct Command {
    pub words: Vec<String>,
    /// One span per word.
    pub spans: Vec<Span>,
    pub heredocs: Vec<Heredoc>,
    /// The command reads the output of the one before it, as after `|`.
    pub piped: bool,
}

/// The words of each simple command in `command`. Heredoc bodies and comments
/// are dropped, since neither is run by this shell.
pub fn simple_commands(command: &str) -> Vec<Vec<String>> {
    commands(command)
        .into_iter()
        .map(|command| command.words)
        .collect()
}

/// Splits `command` on unquoted separators into simple commands. A heredoc
/// body belongs to the command whose line opened it, which for
/// `cat <<EOF | git commit -F -` is `cat`, not `git`. A heredoc opened on a
/// line with no command words belongs to no command.
pub fn commands(command: &str) -> Vec<Command> {
    Lexer {
        chars: command.chars().collect(),
        at: 0,
        commands: Vec::new(),
        words: Vec::new(),
        spans: Vec::new(),
        word: None,
        word_start: 0,
        word_end: 0,
        word_dynamic: false,
        opened: Vec::new(),
        opened_here: Vec::new(),
        piped_next: false,
    }
    .run()
}

/// Replaces the character ranges of `text` named by `edits`. A range that
/// overlaps an earlier one is skipped.
pub fn apply_edits(text: &str, edits: &mut [(Range<usize>, String)]) -> String {
    edits.sort_by_key(|(range, _)| range.start);
    let chars: Vec<char> = text.chars().collect();
    let mut out = String::new();
    let mut at = 0;
    for (range, replacement) in edits.iter() {
        if range.start < at || range.end > chars.len() {
            continue;
        }
        out.extend(&chars[at..range.start]);
        out.push_str(replacement);
        at = range.end;
    }
    out.extend(&chars[at..]);
    out
}

/// `text` as one shell word that reads back as exactly `text`.
pub fn quote(text: &str) -> String {
    format!("'{}'", text.replace('\'', "'\\''"))
}

/// Resolves backslash escapes the way `$'...'`, `printf` and `echo -e` do.
/// An unknown escape is kept as written.
pub fn unescape(text: &str) -> String {
    let chars: Vec<char> = text.chars().collect();
    let mut out = String::new();
    let mut at = 0;
    while at < chars.len() {
        if chars[at] != '\\' || at + 1 == chars.len() {
            out.push(chars[at]);
            at += 1;
            continue;
        }
        let escape = chars[at + 1];
        at += 2;
        let simple = match escape {
            'a' => Some('\u{7}'),
            'b' => Some('\u{8}'),
            'e' | 'E' => Some('\u{1b}'),
            'f' => Some('\u{c}'),
            'n' => Some('\n'),
            'r' => Some('\r'),
            't' => Some('\t'),
            'v' => Some('\u{b}'),
            '\\' | '\'' | '"' | '?' => Some(escape),
            _ => None,
        };
        if let Some(c) = simple {
            out.push(c);
            continue;
        }
        let (radix, max) = match escape {
            'x' => (16, 2),
            'u' => (16, 4),
            'U' => (16, 8),
            '0'..='7' => {
                at -= 1;
                (8, 3)
            }
            _ => {
                out.push('\\');
                out.push(escape);
                continue;
            }
        };
        let digits: String = chars[at..]
            .iter()
            .take(max)
            .take_while(|c| c.is_digit(radix))
            .collect();
        match u32::from_str_radix(&digits, radix)
            .ok()
            .and_then(char::from_u32)
        {
            Some(c) => {
                out.push(c);
                at += digits.len();
            }
            None => {
                out.push('\\');
                out.push(escape);
            }
        }
    }
    out
}

/// A heredoc operator read on the current line, whose body starts at its end.
struct Opened {
    delimiter: String,
    strip_tabs: bool,
    quoted: bool,
    /// Index the opening command takes in `Lexer::commands`, once it is known.
    owner: Option<usize>,
}

struct Lexer {
    chars: Vec<char>,
    at: usize,
    commands: Vec<Command>,
    words: Vec<String>,
    spans: Vec<Span>,
    /// The word being read. `Some("")` after an empty quoted string.
    word: Option<String>,
    word_start: usize,
    word_end: usize,
    word_dynamic: bool,
    /// Heredocs opened on the current line, whose bodies start at its end.
    opened: Vec<Opened>,
    /// The ones in `opened` that belong to the command being read.
    opened_here: Vec<usize>,
    /// The next command reads the output of the one before it.
    piped_next: bool,
}

impl Lexer {
    fn run(mut self) -> Vec<Command> {
        while let Some(c) = self.peek() {
            let start = self.at;
            self.at += 1;
            match c {
                '\'' => {
                    self.begin(start);
                    self.single_quoted();
                }
                '"' => {
                    self.begin(start);
                    self.double_quoted();
                }
                '\\' => {
                    self.begin(start);
                    self.escaped();
                }
                '$' if self.looking_at("'") => {
                    self.begin(start);
                    self.at += 1;
                    self.ansi_quoted();
                }
                '$' => {
                    self.begin(start);
                    self.word_dynamic = true;
                    self.push_str("$");
                }
                '#' if self.word.is_none() => self.skip_comment(),
                '<' if self.looking_at("<<") => {
                    self.begin(start);
                    self.at += 2;
                    self.push_str("<<<");
                }
                '<' if self.looking_at("<") => self.heredoc(),
                '|' => {
                    let or_list =
                        self.looking_at("|") || (self.at >= 2 && self.chars[self.at - 2] == '|');
                    self.end_command();
                    self.piped_next = !or_list;
                }
                ';' | '&' | '(' | ')' | '`' => {
                    self.end_command();
                    self.piped_next = false;
                }
                '\n' | '\r' => {
                    self.end_command();
                    if c == '\n' {
                        self.read_heredoc_bodies();
                    }
                }
                c if c.is_whitespace() => self.end_word(),
                c => {
                    self.begin(start);
                    self.push_str(c.encode_utf8(&mut [0; 4]));
                }
            }
            if self.word.is_some() {
                self.word_end = self.at;
            }
        }
        self.end_command();
        self.commands
    }

    fn peek(&self) -> Option<char> {
        self.chars.get(self.at).copied()
    }

    fn take(&mut self) -> Option<char> {
        let c = self.peek();
        if c.is_some() {
            self.at += 1;
        }
        c
    }

    fn looking_at(&self, text: &str) -> bool {
        text.chars()
            .enumerate()
            .all(|(i, c)| self.chars.get(self.at + i) == Some(&c))
    }

    /// Starts a word at `start` unless one is already being read.
    fn begin(&mut self, start: usize) {
        if self.word.is_none() {
            self.word = Some(String::new());
            self.word_start = start;
            self.word_dynamic = false;
        }
    }

    fn push_str(&mut self, text: &str) {
        self.word.get_or_insert_with(String::new).push_str(text);
    }

    fn end_word(&mut self) {
        if let Some(word) = self.word.take() {
            self.words.push(word);
            self.spans.push(Span {
                start: self.word_start,
                end: self.word_end.min(self.chars.len()),
                dynamic: self.word_dynamic,
            });
        }
    }

    fn end_command(&mut self) {
        self.end_word();
        let here = std::mem::take(&mut self.opened_here);
        if self.words.is_empty() {
            return;
        }
        let index = self.commands.len();
        for at in here {
            self.opened[at].owner = Some(index);
        }
        self.commands.push(Command {
            words: std::mem::take(&mut self.words),
            spans: std::mem::take(&mut self.spans),
            heredocs: Vec::new(),
            piped: std::mem::take(&mut self.piped_next),
        });
    }

    fn single_quoted(&mut self) {
        let mut text = String::new();
        while let Some(c) = self.take().filter(|&c| c != '\'') {
            text.push(c);
        }
        self.push_str(&text);
    }

    /// Inside double quotes a backslash only escapes `$`, a backtick, `"`,
    /// another backslash and a line break, and `$` or a backtick still expand.
    fn double_quoted(&mut self) {
        let mut text = String::new();
        while let Some(c) = self.take().filter(|&c| c != '"') {
            match c {
                '\\' => match self.peek() {
                    Some(next @ ('$' | '`' | '"' | '\\')) => {
                        self.at += 1;
                        text.push(next);
                    }
                    Some('\n') => self.at += 1,
                    _ => text.push('\\'),
                },
                '$' | '`' => {
                    self.word_dynamic = true;
                    text.push(c);
                }
                c => text.push(c),
            }
        }
        self.push_str(&text);
    }

    /// The body of `$'...'`, after the opening quote.
    fn ansi_quoted(&mut self) {
        let mut raw = String::new();
        while let Some(c) = self.take().filter(|&c| c != '\'') {
            raw.push(c);
            if c == '\\' {
                raw.extend(self.take());
            }
        }
        self.push_str(&unescape(&raw));
    }

    /// A backslash makes the next character part of the word, and joins a
    /// line break to the next line.
    fn escaped(&mut self) {
        match self.take() {
            Some('\n') | None => {}
            Some(c) => self.push_str(c.encode_utf8(&mut [0; 4])),
        }
    }

    fn skip_comment(&mut self) {
        while self.peek().is_some_and(|c| c != '\n') {
            self.at += 1;
        }
    }

    /// Reads the delimiter after `<<` and queues the body for the next line
    /// break. The delimiter is not an argument of the command.
    fn heredoc(&mut self) {
        self.at += 1;
        let strip_tabs = self.looking_at("-");
        if strip_tabs {
            self.at += 1;
        }
        while self.looking_at(" ") || self.looking_at("\t") {
            self.at += 1;
        }
        let (delimiter, quoted) = self.delimiter();
        if !delimiter.is_empty() {
            self.opened_here.push(self.opened.len());
            self.opened.push(Opened {
                delimiter,
                strip_tabs,
                quoted,
                owner: None,
            });
        }
    }

    fn delimiter(&mut self) -> (String, bool) {
        let mut delimiter = String::new();
        let mut quote = None;
        let mut quoted = false;
        while let Some(c) = self.peek() {
            match (quote, c) {
                (None, '\'' | '"') => {
                    quote = Some(c);
                    quoted = true;
                }
                (Some(open), c) if c == open => quote = None,
                (None, '\\') => quoted = true,
                (None, c) if c.is_whitespace() || ";&|()<>".contains(c) => break,
                (_, c) => delimiter.push(c),
            }
            self.at += 1;
        }
        (delimiter, quoted)
    }

    fn read_heredoc_bodies(&mut self) {
        for opened in std::mem::take(&mut self.opened) {
            let start = self.at;
            let mut end = start;
            let mut text = String::new();
            while self.at < self.chars.len() {
                let line_start = self.at;
                let stop = (self.at..self.chars.len())
                    .find(|&i| self.chars[i] == '\n')
                    .unwrap_or(self.chars.len());
                let line: String = self.chars[self.at..stop].iter().collect();
                self.at = (stop + 1).min(self.chars.len());
                let line = line.trim_end_matches('\r');
                let line = match opened.strip_tabs {
                    true => line.trim_start_matches('\t'),
                    false => line,
                };
                if line == opened.delimiter {
                    end = line_start;
                    break;
                }
                text.push_str(line);
                text.push('\n');
                end = self.at;
            }
            if let Some(owner) = opened.owner.and_then(|at| self.commands.get_mut(at)) {
                owner.heredocs.push(Heredoc {
                    delimiter: opened.delimiter,
                    quoted: opened.quoted,
                    text,
                    body: start..end,
                });
            }
        }
    }
}
#[cfg(test)]
mod tests {
    use super::*;

    fn slice(text: &str, range: Range<usize>) -> String {
        text.chars().skip(range.start).take(range.len()).collect()
    }

    #[test]
    fn a_heredoc_body_belongs_to_the_command_that_opened_it() {
        let got = commands("git commit -F - <<'EOF'\nfeat: x\n\nRefs: 1\nEOF\ngit status");

        assert_eq!(got.len(), 2);
        assert_eq!(got[0].words, ["git", "commit", "-F", "-"]);
        assert_eq!(got[0].heredocs[0].text, "feat: x\n\nRefs: 1\n");
        assert!(got[0].heredocs[0].quoted);
        assert_eq!(got[1].words, ["git", "status"]);
        assert!(got[1].heredocs.is_empty());
    }

    #[test]
    fn a_piped_heredoc_belongs_to_the_producer_not_the_consumer() {
        let got = commands("cat <<EOF | git commit -F -\nbody\nEOF");

        assert_eq!(got[0].words, ["cat"]);
        assert_eq!(got[0].heredocs[0].text, "body\n");
        assert!(!got[0].heredocs[0].quoted);
        assert_eq!(got[1].words, ["git", "commit", "-F", "-"]);
        assert!(got[1].heredocs.is_empty());
        assert!(got[1].piped);
        assert!(!got[0].piped);
    }

    #[test]
    fn a_heredoc_on_a_line_with_no_command_words_belongs_to_no_command() {
        let got = commands("<<EOF\nbody\nEOF\ngit status");

        assert_eq!(got.len(), 1);
        assert_eq!(got[0].words, ["git", "status"]);
        assert!(got[0].heredocs.is_empty());
    }

    #[test]
    fn the_heredoc_body_range_covers_the_body_lines_only() {
        let script = "cat <<'EOF'\none\ntwo\nEOF\necho done";

        let got = commands(script);

        assert_eq!(slice(script, got[0].heredocs[0].body.clone()), "one\ntwo\n");
    }

    #[test]
    fn a_tab_stripped_heredoc_drops_leading_tabs_from_its_text() {
        let got = commands("cat <<-EOF\n\tone\n\tEOF\n");

        assert_eq!(got[0].heredocs[0].text, "one\n");
    }

    #[test]
    fn only_a_single_pipe_marks_the_next_command_as_piped() {
        let got = commands("a | b || c ; d |\n e");

        let piped: Vec<bool> = got.iter().map(|c| c.piped).collect();

        assert_eq!(piped, [false, true, false, false, true]);
    }

    #[test]
    fn each_word_keeps_its_span_in_the_source() {
        let script = "git commit -m \"a b\" 'c'  d";

        let got = commands(script);

        let words: Vec<String> = got[0]
            .spans
            .iter()
            .map(|s| slice(script, s.start..s.end))
            .collect();
        assert_eq!(words, ["git", "commit", "-m", "\"a b\"", "'c'", "d"]);
    }

    #[test]
    fn a_word_is_dynamic_only_when_the_shell_would_expand_it() {
        let got = commands(r#"x "$A" '$B' \$C $D "a\$b" $'e' "$(f)""#);

        let dynamic: Vec<bool> = got[0].spans.iter().map(|s| s.dynamic).collect();

        assert_eq!(
            dynamic,
            [false, true, false, false, true, false, false, true]
        );
    }

    #[test]
    fn ansi_c_quoting_is_decoded() {
        let got = commands(r"git commit -m $'a\nb\tc \'q\' \x41'");

        assert_eq!(got[0].words[3], "a\nb\tc 'q' A");
    }

    #[test]
    fn a_double_quoted_backslash_keeps_unknown_escapes() {
        let got = commands(r#"printf "a\nb \"q\" \$x \\ c""#);

        assert_eq!(got[0].words[1], r#"a\nb "q" $x \ c"#);
    }

    #[test]
    fn unescape_resolves_the_printf_escapes() {
        assert_eq!(unescape(r"a\nb\tc\\d\x41\101é\q"), "a\nb\tc\\dAA\u{e9}\\q");
        assert_eq!(unescape("trailing\\"), "trailing\\");
    }

    #[test]
    fn quote_reads_back_as_the_same_word() {
        let text = "it's a \"test\"\nwith $x and `y`";

        let got = commands(&format!("echo {}", quote(text)));

        assert_eq!(got[0].words[1], text);
    }

    #[test]
    fn apply_edits_replaces_character_ranges_and_skips_overlaps() {
        let mut edits = vec![
            (4..7, "XY".to_string()),
            (0..1, "z".to_string()),
            (5..6, "!".to_string()),
        ];

        assert_eq!(apply_edits("a \u{e9} bcd e", &mut edits), "z \u{e9} XY e");
    }

    #[test]
    fn simple_commands_keep_only_the_words() {
        let got = simple_commands("env A=1 git commit -m 'a b' && echo <<X\nignored\nX\n");

        assert_eq!(
            got,
            vec![
                vec!["env", "A=1", "git", "commit", "-m", "a b"],
                vec!["echo"],
            ]
        );
    }
}
