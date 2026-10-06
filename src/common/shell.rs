// SPDX-FileCopyrightText: 2026 Igor Santos
// SPDX-License-Identifier: MIT

//! A small, quote-aware reader for shell command lines, shared by the hooks
//! that need to see which program a Bash call runs and with what words.
//!
//! It splits on unquoted separators into simple commands, removes quotes, and
//! keeps each command's heredoc body apart from its words. It does not expand
//! variables, aliases, functions or substitutions, so every caller is a
//! guardrail against drift rather than a security boundary.

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

/// One simple command: its words with quotes removed, and the text of any
/// heredoc bodies opened on its line.
pub struct Command {
    pub words: Vec<String>,
    pub heredoc: String,
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
/// `cat <<EOF | git commit -F -` is `cat`, not `git`.
pub fn commands(command: &str) -> Vec<Command> {
    Lexer {
        chars: command.chars().collect(),
        at: 0,
        commands: Vec::new(),
        words: Vec::new(),
        word: None,
        heredocs: Vec::new(),
    }
    .run()
}

struct Heredoc {
    delimiter: String,
    strip_tabs: bool,
    /// Index the opening command takes in `Lexer::commands` once it ends.
    owner: usize,
}

struct Lexer {
    chars: Vec<char>,
    at: usize,
    commands: Vec<Command>,
    words: Vec<String>,
    /// The word being read. `Some("")` after an empty quoted string.
    word: Option<String>,
    /// Heredocs opened on the current line, whose bodies start at its end.
    heredocs: Vec<Heredoc>,
}

impl Lexer {
    fn run(mut self) -> Vec<Command> {
        while let Some(c) = self.take() {
            match c {
                '\'' => self.single_quoted(),
                '"' => self.double_quoted(),
                '\\' => self.escaped(),
                '#' if self.word.is_none() => self.skip_comment(),
                '<' if self.looking_at("<<") => {
                    self.at += 2;
                    self.push_str("<<<");
                }
                '<' if self.looking_at("<") => self.heredoc(),
                ';' | '&' | '|' | '(' | ')' | '`' => self.end_command(),
                '\n' | '\r' => {
                    self.end_command();
                    if c == '\n' {
                        self.skip_heredoc_bodies();
                    }
                }
                c if c.is_whitespace() => self.end_word(),
                c => self.push_str(c.encode_utf8(&mut [0; 4])),
            }
        }
        self.end_command();
        self.commands
    }

    fn take(&mut self) -> Option<char> {
        let c = self.chars.get(self.at).copied();
        self.at += 1;
        c
    }

    fn looking_at(&self, text: &str) -> bool {
        text.chars()
            .enumerate()
            .all(|(i, c)| self.chars.get(self.at + i) == Some(&c))
    }

    fn push_str(&mut self, text: &str) {
        self.word.get_or_insert_with(String::new).push_str(text);
    }

    fn end_word(&mut self) {
        if let Some(word) = self.word.take() {
            self.words.push(word);
        }
    }

    fn end_command(&mut self) {
        self.end_word();
        if !self.words.is_empty() {
            self.commands.push(Command {
                words: std::mem::take(&mut self.words),
                heredoc: String::new(),
            });
        }
    }

    fn single_quoted(&mut self) {
        let mut text = String::new();
        while let Some(c) = self.take().filter(|&c| c != '\'') {
            text.push(c);
        }
        self.push_str(&text);
    }

    fn double_quoted(&mut self) {
        let mut text = String::new();
        while let Some(c) = self.take().filter(|&c| c != '"') {
            match c {
                '\\' => text.extend(self.take()),
                c => text.push(c),
            }
        }
        self.push_str(&text);
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
        while self.chars.get(self.at).is_some_and(|&c| c != '\n') {
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
        let delimiter = self.delimiter();
        if !delimiter.is_empty() {
            self.heredocs.push(Heredoc {
                delimiter,
                strip_tabs,
                owner: self.commands.len(),
            });
        }
    }

    fn delimiter(&mut self) -> String {
        let mut delimiter = String::new();
        let mut quote = None;
        while let Some(&c) = self.chars.get(self.at) {
            match (quote, c) {
                (None, '\'' | '"') => quote = Some(c),
                (Some(open), c) if c == open => quote = None,
                (None, '\\') => {}
                (None, c) if c.is_whitespace() || ";&|()<>".contains(c) => break,
                (_, c) => delimiter.push(c),
            }
            self.at += 1;
        }
        delimiter
    }

    fn skip_heredoc_bodies(&mut self) {
        for heredoc in std::mem::take(&mut self.heredocs) {
            let mut body = String::new();
            while self.at < self.chars.len() {
                let end = (self.at..self.chars.len())
                    .find(|&i| self.chars[i] == '\n')
                    .unwrap_or(self.chars.len());
                let line: String = self.chars[self.at..end].iter().collect();
                self.at = (end + 1).min(self.chars.len());
                let line = line.trim_end_matches('\r');
                let line = match heredoc.strip_tabs {
                    true => line.trim_start_matches('\t'),
                    false => line,
                };
                if line == heredoc.delimiter {
                    break;
                }
                body.push_str(line);
                body.push('\n');
            }
            if let Some(owner) = self.commands.get_mut(heredoc.owner) {
                owner.heredoc.push_str(&body);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_heredoc_body_belongs_to_the_command_that_opened_it() {
        let got = commands("git commit -F - <<'EOF'\nfeat: x\n\nRefs: 1\nEOF\ngit status");

        assert_eq!(got.len(), 2);
        assert_eq!(got[0].words, ["git", "commit", "-F", "-"]);
        assert_eq!(got[0].heredoc, "feat: x\n\nRefs: 1\n");
        assert_eq!(got[1].words, ["git", "status"]);
        assert_eq!(got[1].heredoc, "");
    }

    #[test]
    fn a_piped_heredoc_belongs_to_the_producer_not_the_consumer() {
        let got = commands("cat <<EOF | git commit -F -\nbody\nEOF");

        assert_eq!(got[0].words, ["cat"]);
        assert_eq!(got[0].heredoc, "body\n");
        assert_eq!(got[1].words, ["git", "commit", "-F", "-"]);
        assert_eq!(got[1].heredoc, "");
    }

    #[test]
    fn a_tab_stripped_heredoc_drops_leading_tabs_from_its_body() {
        let got = commands("cat <<-EOF\n\tone\n\tEOF\n");

        assert_eq!(got[0].heredoc, "one\n");
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
