// SPDX-FileCopyrightText: 2026 Igor Santos
// SPDX-License-Identifier: Apache-2.0

//! Finds what a Bash command deletes, from its real command words.
//!
//! The command is read with the shared shell lexer, so quotes, heredoc bodies
//! and comments are data and only a word in command position can be `rm`. A
//! deletion is seen after `&&`, `;`, `|` and line breaks, inside `( ... )`,
//! `$( ... )` and backticks, behind wrappers and runners (`sudo`, `env`,
//! `command`, `builtin`, `nohup`, `doas`, `xargs`), inside `sh -c`, `eval`
//! and here-strings, in a heredoc fed to a shell, and as `find -delete` or
//! `find -exec rm`. A word inside a quoted grep pattern, a commit message, an
//! `echo` or a heredoc read by another program is never a command.
//!
//! What it cannot resolve it reports as unresolvable, so the guard blocks it:
//! a target holding a variable other than `$HOME` or `$TMPDIR`, a relative
//! target after a change of directory, a deletion inside or next to a command
//! substitution, a shell reading a pipe, and `xargs` deleting what anything
//! other than a plain `find` prints.

use crate::common::shell::{
    self, program_index, program_name, redirect_len, runner_value_options, shell_script_at,
    skip_options, Span, SHELLS,
};

/// Reported when `rm` is reached through, or takes words from, `$(...)` or a
/// backtick pair.
pub const SUBSTITUTION_LABEL: &str = "<command substitution>";

/// Reported when `xargs` deletes paths the guard cannot see.
pub const XARGS_LABEL: &str = "<xargs input>";

/// Reported when strings nest deeper than the scan follows.
const DEPTH_LABEL: &str = "<nested shell>";

/// Reported when a shell runs a script read from a pipe.
const SHELL_INPUT_LABEL: &str = "<shell input>";

/// Reported when `find` follows links, so its paths can leave its start paths.
const FIND_LINKS_LABEL: &str = "find -L";

const MAX_DEPTH: usize = 4;

/// One thing a command deletes.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Target {
    /// A path as the shell passes it. `tilde` is set when a leading `~` is
    /// unquoted, so the shell expands it to the home folder.
    Path { text: String, tilde: bool },
    /// Something only the running shell knows, named for the denial.
    Unresolvable(String),
}

/// The values the guard may expand itself.
pub struct Env<'a> {
    pub home: &'a str,
    pub tmpdir: Option<&'a str>,
}

/// Everything `command` deletes, in order.
pub fn targets(command: &str, env: &Env) -> Vec<Target> {
    let mut scan = Scan {
        env,
        out: Vec::new(),
        saw_cd: false,
        tainted: false,
    };
    scan.script(command, 0, false);
    scan.out
}

/// The words of one command with their spans, kept in step.
#[derive(Clone, Copy)]
struct Words<'w> {
    words: &'w [String],
    spans: &'w [Span],
}

impl<'w> Words<'w> {
    fn of(command: &'w shell::Command) -> Self {
        Words {
            words: &command.words,
            spans: &command.spans,
        }
    }

    fn range(self, from: usize, to: usize) -> Self {
        let to = to.min(self.words.len());
        let from = from.min(to);
        Words {
            words: &self.words[from..to],
            spans: &self.spans[from..to],
        }
    }

    fn tail(self, from: usize) -> Self {
        self.range(from, self.words.len())
    }
}

/// Whether the `{}` of `find -exec` and `xargs -I` is an operand.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Placeholder {
    Operand,
    Skip,
}

struct Scan<'a> {
    env: &'a Env<'a>,
    out: Vec<Target>,
    /// The directory changed earlier, so a relative path no longer means the
    /// cwd.
    saw_cd: bool,
    /// The command sets or unsets `HOME` or `TMPDIR`, so neither is expanded.
    tainted: bool,
}

/// Where a simple command sits.
struct Ctx<'c> {
    source: &'c [char],
    depth: usize,
    /// The command runs inside, or meets, a command substitution.
    substitution: bool,
    /// The command is a piece a substitution split off, so its own words do
    /// not show the whole command.
    fragment: bool,
}

impl Scan<'_> {
    /// Records `target`, naming each unresolvable label once.
    fn push(&mut self, target: Target) {
        if matches!(target, Target::Unresolvable(_)) && self.out.contains(&target) {
            return;
        }
        self.out.push(target);
    }

    fn unresolvable(&mut self, label: &str) {
        self.push(Target::Unresolvable(label.to_string()));
    }

    fn script(&mut self, text: &str, depth: usize, in_substitution: bool) {
        if depth > MAX_DEPTH {
            self.unresolvable(DEPTH_LABEL);
            return;
        }
        let source: Vec<char> = text.chars().collect();
        let commands = shell::commands(text);
        self.tainted |= commands
            .iter()
            .flat_map(|c| &c.words)
            .any(|w| names_expanded(w));
        for (i, command) in commands.iter().enumerate() {
            let fragment = command.joined || command.opens_substitution;
            let ctx = Ctx {
                source: &source,
                depth,
                substitution: in_substitution || command.substitution || fragment,
                fragment,
            };
            let before = i.checked_sub(1).map(|p| &commands[p]);
            let producer = before.filter(|p| command.piped && same_pipeline(p, command));
            self.command(Words::of(command), &ctx, producer);

            if command.piped
                && reads_script_from_stdin(&command.words)
                && before.is_none_or(|p| p.heredocs.is_empty())
            {
                self.unresolvable(SHELL_INPUT_LABEL);
            }
            let feeds_shell = reads_script_from_stdin(&command.words)
                || commands
                    .get(i + 1)
                    .is_some_and(|n| n.piped && reads_script_from_stdin(&n.words));
            for heredoc in &command.heredocs {
                if feeds_shell {
                    self.script(&heredoc.text, depth + 1, ctx.substitution);
                } else if !heredoc.quoted {
                    // An unquoted body still runs its substitutions.
                    for inner in shell::substitutions(&heredoc.text, false) {
                        self.script(&inner, depth + 1, true);
                    }
                }
            }
            // A substitution inside double quotes stays in one word.
            for span in command.spans.iter().filter(|s| s.dynamic) {
                for inner in shell::substitutions(&raw_text(&source, span), true) {
                    self.script(&inner, depth + 1, true);
                }
            }
        }
    }

    fn command(&mut self, cmd: Words, ctx: &Ctx, producer: Option<&shell::Command>) {
        let Some(at) = program_at(cmd.words) else {
            return;
        };
        if changes_directory(&cmd.words[..at]) {
            self.saw_cd = true;
        }
        let args = cmd.tail(at + 1);
        match program_name(&cmd.words[at]).as_str() {
            "cd" | "pushd" | "popd" => self.saw_cd = true,
            "rm" => {
                self.substitution_note(ctx);
                self.rm_args(args, ctx, Placeholder::Operand);
            }
            "find" => self.find(args, ctx),
            "xargs" => self.xargs(args, ctx, producer),
            "eval" => {
                let text = args.words.join(" ");
                self.nested(&text, args.spans.iter().any(|s| s.dynamic), ctx);
            }
            name if SHELLS.contains(&name) => self.shell(args, ctx),
            _ => {}
        }
    }

    /// The script a shell runs from `-c` or a here-string.
    fn shell(&mut self, args: Words, ctx: &Ctx) {
        let at = shell_script_at(args.words).or_else(|| {
            let here = args.words.iter().position(|w| w == "<<<")?;
            (here + 1 < args.words.len()).then_some(here + 1)
        });
        if let Some(at) = at {
            self.nested(&args.words[at], args.spans[at].dynamic, ctx);
        }
    }

    /// Scans a script given as a string. One made only of a variable is
    /// unknown until it runs.
    fn nested(&mut self, text: &str, dynamic: bool, ctx: &Ctx) {
        let trimmed = text.trim();
        if dynamic && trimmed.starts_with('$') && !trimmed.contains(char::is_whitespace) {
            self.push(Target::Unresolvable(trimmed.to_string()));
            return;
        }
        self.script(text, ctx.depth + 1, ctx.substitution);
    }

    fn substitution_note(&mut self, ctx: &Ctx) {
        if ctx.substitution {
            self.unresolvable(SUBSTITUTION_LABEL);
        }
    }

    /// The operands of an `rm`. Options stop at `--`, and redirections are not
    /// operands.
    fn rm_args(&mut self, args: Words, ctx: &Ctx, placeholder: Placeholder) {
        let mut options = true;
        let mut i = 0;
        while i < args.words.len() {
            let (word, span) = (&args.words[i], &args.spans[i]);
            let redirect = redirect_len(&raw_text(ctx.source, span));
            if redirect > 0 {
                i += redirect;
                continue;
            }
            i += 1;
            if options && word == "--" {
                options = false;
                continue;
            }
            if options && word.len() > 1 && word.starts_with('-') && !span.dynamic {
                continue;
            }
            if placeholder == Placeholder::Skip && word.contains("{}") {
                continue;
            }
            let target = self.classify(word, span, ctx);
            self.push(target);
        }
    }

    fn classify(&self, word: &str, span: &Span, ctx: &Ctx) -> Target {
        let raw = raw_text(ctx.source, span);
        if ctx.source.get(span.end) == Some(&'(') {
            // `$(` opens a substitution. Any other word that runs into `(` is
            // a zsh glob qualifier such as `x(:h)`, which can name a parent.
            if !word.ends_with('$') {
                return Target::Unresolvable(format!("{raw}("));
            }
            return Target::Unresolvable(SUBSTITUTION_LABEL.to_string());
        }
        if span.dynamic {
            if word.contains("$(") || word.contains('`') {
                return Target::Unresolvable(SUBSTITUTION_LABEL.to_string());
            }
            return match self.expand(word, &raw) {
                Some(text) => Target::Path { text, tilde: false },
                None => Target::Unresolvable(raw),
            };
        }
        // A `$` left by single quotes is literal, but a path naming one is
        // more likely a mistake than a file, so it is not guessed at.
        if word.contains('$') {
            return Target::Unresolvable(raw);
        }
        let tilde = raw.starts_with('~');
        if self.saw_cd && !word.starts_with('/') && !tilde {
            return Target::Unresolvable(word.to_string());
        }
        Target::Path {
            text: word.to_string(),
            tilde,
        }
    }

    /// `$HOME`, `${HOME}`, `$TMPDIR` or `${TMPDIR}` at the start of `word`,
    /// expanded with the values this hook sees, when nothing else in the word
    /// expands and the command never redefines them.
    fn expand(&self, word: &str, raw: &str) -> Option<String> {
        if self.tainted || raw.starts_with('\'') {
            return None;
        }
        for (name, value) in [("HOME", Some(self.env.home)), ("TMPDIR", self.env.tmpdir)] {
            for prefix in [format!("${name}"), format!("${{{name}}}")] {
                let Some(rest) = word.strip_prefix(&prefix) else {
                    continue;
                };
                if !(rest.is_empty() || rest.starts_with('/')) || rest.contains(['$', '`']) {
                    return None;
                }
                let value = value.filter(|v| v.starts_with('/'))?;
                let joined = format!("{}{rest}", value.trim_end_matches('/'));
                return Some(if joined.is_empty() {
                    "/".to_string()
                } else {
                    joined
                });
            }
        }
        None
    }

    fn find(&mut self, args: Words, ctx: &Ctx) {
        // A substitution split this `find`, so its expression is out of sight.
        if ctx.fragment {
            self.unresolvable(SUBSTITUTION_LABEL);
        }
        let parts = FindParts::read(args.words);
        let mut deletes = args.words[parts.expression..]
            .iter()
            .any(|w| w == "-delete");
        let mut j = parts.expression;
        while j < args.words.len() {
            if !matches!(
                args.words[j].as_str(),
                "-exec" | "-execdir" | "-ok" | "-okdir"
            ) {
                j += 1;
                continue;
            }
            let begin = j + 1;
            let end = (begin..args.words.len())
                .find(|&e| args.words[e] == ";" || args.words[e] == "+")
                .unwrap_or(args.words.len());
            deletes |= self.runs_a_deletion(args.range(begin, end), ctx);
            j = end + 1;
        }
        if deletes {
            self.substitution_note(ctx);
            self.find_inputs(args, &parts, ctx);
        }
    }

    /// Scans the command a `find -exec` or `xargs` runs, and says whether it
    /// deletes anything.
    fn runs_a_deletion(&mut self, inner: Words, ctx: &Ctx) -> bool {
        match program_at(inner.words) {
            Some(p) if program_name(&inner.words[p]) == "rm" => {
                self.substitution_note(ctx);
                self.rm_args(inner.tail(p + 1), ctx, Placeholder::Skip);
                true
            }
            Some(_) => {
                let before = self.out.len();
                self.command(inner, ctx, None);
                self.out.len() > before
            }
            None => false,
        }
    }

    /// The start paths of a `find` judged as deletion targets, since every
    /// path it reports sits under one. No start path means `.`.
    fn find_inputs(&mut self, args: Words, parts: &FindParts, ctx: &Ctx) {
        if parts.follows_links {
            self.unresolvable(FIND_LINKS_LABEL);
            return;
        }
        if parts.starts.is_empty() {
            let dot = Target::Path {
                text: ".".to_string(),
                tilde: false,
            };
            self.push(if self.saw_cd {
                Target::Unresolvable(".".to_string())
            } else {
                dot
            });
            return;
        }
        for &k in &parts.starts {
            let target = self.classify(&args.words[k], &args.spans[k], ctx);
            self.push(target);
        }
    }

    fn xargs(&mut self, args: Words, ctx: &Ctx, producer: Option<&shell::Command>) {
        let from = skip_options(args.words, 0, runner_value_options("xargs").unwrap_or(&[]));
        if from >= args.words.len() || !self.runs_a_deletion(args.tail(from), ctx) {
            return;
        }
        let find = producer.and_then(|c| {
            let at = program_at(&c.words)?;
            lists_paths(&c.words, at).then(|| Words::of(c).tail(at + 1))
        });
        match find {
            Some(find) => {
                let parts = FindParts::read(find.words);
                self.find_inputs(find, &parts, ctx);
            }
            None => self.unresolvable(XARGS_LABEL),
        }
    }
}

/// The parts of a `find` command line, by index into its arguments.
struct FindParts {
    starts: Vec<usize>,
    /// Where the expression starts.
    expression: usize,
    follows_links: bool,
}

impl FindParts {
    /// Reads past the leading options of GNU and BSD `find` (`-H`, `-L`, `-P`,
    /// `-D x`, `-O3`, `-E`, `-X`, `-d`, `-s`, `-x`, `-f path`) to the start
    /// paths.
    fn read(args: &[String]) -> Self {
        let mut starts = Vec::new();
        let mut follows_links = false;
        let mut i = 0;
        while let Some(word) = args.get(i) {
            match word.as_str() {
                "-H" | "-P" | "-E" | "-X" | "-d" | "-s" | "-x" => i += 1,
                "-L" => {
                    follows_links = true;
                    i += 1;
                }
                "-D" => i += 2,
                "-f" => {
                    if i + 1 < args.len() {
                        starts.push(i + 1);
                    }
                    i += 2;
                }
                w if w.starts_with("-O") => i += 1,
                _ => break,
            }
        }
        let mut end = i.min(args.len());
        while args
            .get(end)
            .is_some_and(|w| !w.starts_with('-') && !matches!(w.as_str(), "(" | ")" | "!" | ","))
        {
            starts.push(end);
            end += 1;
        }
        FindParts {
            starts,
            expression: end,
            follows_links: follows_links || args[end..].iter().any(|w| w == "-follow"),
        }
    }
}

/// The program of a command, past wrappers (`program_index`), runners such
/// as `nohup` with their options, and `builtin`. `xargs` is a program here,
/// since its input matters.
fn program_at(words: &[String]) -> Option<usize> {
    let mut from = 0;
    loop {
        let at = from + program_index(words.get(from..)?)?;
        let name = program_name(&words[at]);
        let next = match runner_value_options(&name) {
            Some(takes_value) if name != "xargs" => skip_options(words, at + 1, takes_value),
            _ if name == "builtin" => at + 1,
            _ => return Some(at),
        };
        if next >= words.len() {
            return None;
        }
        from = next;
    }
}

/// Whether wrapper words change the directory the program runs in, as
/// `env -C dir` and `sudo -D dir` do.
fn changes_directory(wrappers: &[String]) -> bool {
    wrappers
        .iter()
        .any(|w| matches!(program_name(w).as_str(), "env" | "sudo"))
        && wrappers
            .iter()
            .any(|w| matches!(w.as_str(), "-C" | "-D" | "--chdir") || w.starts_with("--chdir="))
}

/// Whether `word`, with its quotes removed, names `HOME` or `TMPDIR` other
/// than to read it, as in `HOME=/`, `export TMPDIR=x` or `unset HOME`.
fn names_expanded(word: &str) -> bool {
    ["HOME", "TMPDIR"].iter().any(|name| {
        word.strip_prefix(name)
            .is_some_and(|rest| rest.is_empty() || rest.starts_with('=') || rest.starts_with("+="))
    })
}

/// The source text of one word, quotes included.
fn raw_text(source: &[char], span: &Span) -> String {
    source
        .get(span.start..span.end.min(source.len()))
        .map(|s| s.iter().collect())
        .unwrap_or_default()
}

/// Whether `producer` is the whole command before the `|` that feeds
/// `consumer`, so its output is all `consumer` reads.
fn same_pipeline(producer: &shell::Command, consumer: &shell::Command) -> bool {
    producer.depth == consumer.depth
        && producer.group == consumer.group
        && !producer.substitution
        && !producer.joined
        && !producer.opens_substitution
}

/// A `find` that only prints paths, so its output is its start paths and
/// what lies under them.
fn lists_paths(words: &[String], at: usize) -> bool {
    program_name(&words[at]) == "find"
        && !words[at + 1..].iter().any(|w| {
            matches!(
                w.as_str(),
                "-exec" | "-execdir" | "-ok" | "-okdir" | "-delete" | "-printf" | "-fprint"
            )
        })
}

/// A shell that reads its script from standard input: no `-c`, and no script
/// file unless `-s` says to read standard input anyway.
fn reads_script_from_stdin(words: &[String]) -> bool {
    let Some(at) = program_at(words) else {
        return false;
    };
    if !SHELLS.contains(&program_name(&words[at]).as_str()) {
        return false;
    }
    let args = &words[at + 1..];
    if shell_script_at(args).is_some() || args.iter().any(|w| w == "<<<") {
        return false;
    }
    let mut operands = false;
    let mut i = 0;
    while let Some(word) = args.get(i) {
        let redirect = redirect_len(word);
        if redirect > 0 {
            i += redirect;
            continue;
        }
        operands |= !word.starts_with('-');
        i += 1;
    }
    !operands || args.iter().any(|w| w == "-s")
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Scans `case` with `RM` replaced at run time, so this file never holds a
    /// literal deletion that the live guard would judge.
    fn scan(case: &str) -> (String, Vec<Target>) {
        let command = case.replace("RM", &("r".to_string() + "m"));
        let env = Env {
            home: "/Users/me",
            tmpdir: Some("/var/folders/ab/xyz/T/"),
        };
        let got = targets(&command, &env);
        (command, got)
    }

    /// `p:/a` a path, `t:~/a` a path with an unquoted `~`, `u:x` unresolvable
    /// with label `x`; `,` between targets.
    fn spec(want: &str) -> Vec<Target> {
        let one = |item: &str| {
            let (kind, text) = item.split_at(2);
            match kind {
                "p:" | "t:" => Target::Path {
                    text: text.to_string(),
                    tilde: kind == "t:",
                },
                _ => Target::Unresolvable(text.to_string()),
            }
        };
        want.split(',').filter(|i| !i.is_empty()).map(one).collect()
    }

    const S: &str = "u:<command substitution>";

    #[rustfmt::skip]
    const FOUND: &[(&str, &str)] = &[
        // Plain forms.
        ("RM -rf /a/b", "p:/a/b"),
        ("RM a b", "p:a,p:b"),
        ("RM -f -- -weird", "p:-weird"),
        ("RM -f /a/x 2>/dev/null", "p:/a/x"),
        ("RM -f /a/x > /dev/null", "p:/a/x"),
        ("RM -rf \"/a/with space\"", "p:/a/with space"),
        ("RM -rf ~/x", "t:~/x"),
        ("RM -rf \"~/x\"", "p:~/x"),
        ("RM -rf /a/b/", "p:/a/b/"),
        // After separators and in groups.
        ("ls && RM /a", "p:/a"),
        ("ls; RM /a", "p:/a"),
        ("ls | RM /a", "p:/a"),
        ("ls || RM /a", "p:/a"),
        ("ls &\nRM /a", "p:/a"),
        ("(ls; RM /a)", "p:/a"),
        ("{ RM /a; }", "p:/a"),
        ("if true; then RM /a; fi", "p:/a"),
        // Wrappers and spellings.
        ("sudo RM -rf /a", "p:/a"),
        ("sudo -u root RM /a", "p:/a"),
        ("command RM /a", "p:/a"),
        ("\\RM /a", "p:/a"),
        ("/bin/RM /a", "p:/a"),
        ("env X=1 RM /a", "p:/a"),
        ("X=1 RM /a", "p:/a"),
        ("nohup RM /a", "p:/a"),
        ("timeout 5 RM /a", "p:/a"),
        ("r'm' /a", "p:/a"),
        // Nested shells.
        ("sh -c 'RM /a'", "p:/a"),
        ("bash -lc \"ls && RM /a\"", "p:/a"),
        ("eval 'RM /a'", "p:/a"),
        ("bash <<EOF\nRM /a\nEOF", "p:/a"),
        ("cat <<EOF | sh\nRM /a\nEOF", "p:/a"),
        // find and xargs.
        ("find /a -name x -delete", "p:/a"),
        ("find -delete", "p:."),
        ("find /a -exec RM -f {} \\;", "p:/a"),
        ("find /a /b -exec RM {} +", "p:/a,p:/b"),
        ("find /a -print0 | xargs -0 RM -f", "p:/a"),
        ("ls | xargs RM", "u:<xargs input>"),
        ("xargs -n 1 RM /a < list", "p:/a,u:<xargs input>"),
        ("find -L /a -delete", "u:find -L"),
        // Substitutions.
        ("RM -rf $(cat list)", S),
        ("RM -rf `cat list`", S),
        ("X=$(pwd) RM /a", "u:<command substitution>,p:/a"),
        ("echo $(date; RM /a)", "u:<command substitution>,p:/a"),
        ("echo \"$(RM /a)\"", "u:<command substitution>,p:/a"),
        ("echo `RM /a`", "u:<command substitution>,p:/a"),
        ("echo $(date) && RM /a", "p:/a"),
        ("RM /a && echo $(date)", "p:/a"),
        // Variables, zsh qualifiers and cd.
        ("RM -rf $HOME/x", "p:/Users/me/x"),
        ("RM -rf \"${HOME}/x\"", "p:/Users/me/x"),
        ("RM -rf $TMPDIR/x", "p:/var/folders/ab/xyz/T/x"),
        ("RM -rf \"$TMPDIR\"", "p:/var/folders/ab/xyz/T"),
        ("RM -rf '$HOME/x'", "u:'$HOME/x'"),
        ("RM -rf $REPO/x", "u:$REPO/x"),
        ("RM -rf $HOME$X", "u:$HOME$X"),
        ("RM -rf /a/x(:h)", "u:/a/x("),
        ("HOME=/ RM -rf $HOME/x", "u:$HOME/x"),
        ("export TMPDIR=/; RM -rf $TMPDIR/x", "u:$TMPDIR/x"),
        ("cd /x && RM -rf build", "u:build"),
        ("cd /x && RM -rf /x/build ~/y", "p:/x/build,t:~/y"),
        ("builtin cd /x && RM b", "u:b"),
        ("env -C /x RM b", "u:b"),
        ("sudo -D /x RM b", "u:b"),
        ("export HO''ME=/; RM -rf $HOME/x", "u:$HOME/x"),
        // Runners with option values.
        ("ionice -c 3 RM /a", "p:/a"),
        ("doas -u root RM /a", "p:/a"),
        ("busybox RM /a", "p:/a"),
        // Shells reading a script from somewhere else.
        ("bash <<< 'RM /a'", "p:/a"),
        ("bash -s -- x <<EOF\nRM /a\nEOF", "p:/a"),
        ("echo 'RM /a' | sh", "u:<shell input>"),
        ("eval \"$cmd\"", "u:$cmd"),
        ("bash -c \"$cmd\"", "u:$cmd"),
        ("eval eval eval eval eval eval RM /a", "u:<nested shell>"),
        ("cat <<EOF\n$(RM /a)\nEOF", "u:<command substitution>,p:/a"),
        ("echo \"$(printf ')'; RM /a)\"", "u:<command substitution>,p:/a"),
        // More find and xargs forms.
        ("find /a -execdir RM {} +", "p:/a"),
        ("find /a -ok RM {} \\;", "p:/a"),
        ("find /a -exec sh -c 'RM -rf {}' \\;", "p:{},p:/a"),
        ("find /a -exec nohup RM -rf {} +", "p:/a"),
        ("find -D tree /a -delete", "p:/a"),
        ("find -O3 -P /a -delete", "p:/a"),
        ("find -x / -name x -delete", "p:/"),
        ("find -f /a -delete", "p:/a"),
        ("cd /x && find -delete", "u:."),
        ("find $(pwd) -name x -delete", S),
        ("find /a -exec ls {} + | xargs RM", "u:<xargs input>"),
        ("(echo /b; find /a) | xargs RM", "u:<xargs input>"),
        ("ls | xargs nohup RM -rf", "u:<xargs input>"),
        ("ls | xargs -I{} sh -c 'RM -rf {}'", "p:{},u:<xargs input>"),
    ];

    /// Commands that only read, search or mention a deletion as data.
    #[rustfmt::skip]
    const DATA: &[&str] = &[
        "grep -E \"a | RM b\" file.txt",
        "grep -rn \"RM -rf\" ~/.config/playbook/memory",
        "rg \"hooks\\.rmGuard|RM -rf\" crates/",
        "echo \"x; RM -rf /a\"",
        "echo RM -rf /a",
        "git commit -m \"fix: guard; RM /a note\"",
        "git commit -m 'docs: RM on /a is fatal'",
        "git commit -F - <<'EOF'\nfix: stop RM -rf /a\nRM -rf /b\nEOF",
        "cat <<EOF\nRM -rf /a\nEOF",
        "cat <<EOF\nfoo RM -rf /a",
        "git commit -m \"$(cat <<'EOF'\nfix: RM -rf /a (twice)\nit's done\nEOF\n)\"",
        "echo hi # RM -rf /a",
        "printf '%s\\n' \"RM /a\" > notes.txt",
        "git RM --cached /a",
        "docker RM box",
        "command -v RM",
        "man RM",
        "sqlite3 ~/.config/playbook/data/config.db .tables",
        "ls ~/.config/playbook/memory 2>/dev/null",
        "cat ~/.config/playbook/config.json",
        "grep -n rmGuard ~/.claude/settings.json",
        "playbook migrate --status",
        "find /a -name '*.md'",
        "echo $((1 << 2))",
        "wc -l $(find . -name '*.rs')",
        "git commit -m \"$(cat <<'EOF'\n1) step\nuse `RM -rf build`\nEOF\n)\"",
        "cat <<'EOF'\n$(RM /a)\nEOF",
        "bash script.sh",
    ];

    #[test]
    fn the_parser_finds_every_real_deletion() {
        for (case, want) in FOUND {
            let (command, got) = scan(case);

            assert_eq!(got, spec(want), "{command:?}");
        }
    }

    #[test]
    fn data_that_mentions_a_deletion_is_never_a_command() {
        for case in DATA {
            let (command, got) = scan(case);

            assert_eq!(got, Vec::<Target>::new(), "{command:?}");
        }
    }

    #[test]
    fn only_a_write_or_unset_of_home_or_tmpdir_stops_expansion() {
        assert!(names_expanded("HOME=/"));
        assert!(names_expanded("TMPDIR"));
        assert!(names_expanded("HOME+=x"));
        assert!(!names_expanded("$HOME"));
        assert!(!names_expanded("HOMEBREW_PREFIX=x"));
    }

    #[test]
    fn an_unset_tmpdir_is_not_expanded() {
        let command = format!("{} -rf $TMPDIR/x", "r".to_string() + "m");
        let env = Env {
            home: "/Users/me",
            tmpdir: None,
        };

        assert_eq!(targets(&command, &env), spec("u:$TMPDIR/x"));
    }
}
