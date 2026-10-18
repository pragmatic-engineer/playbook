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
//! The folder the shell is in is followed through `cd`, `pushd` and `popd`,
//! so a relative target after a change of directory is judged where it
//! lands. A `cd` inside `( ... )` or `$( ... )` ends with the group. A `cd`
//! that may fail leaves the old folder current too: unless `&&` joins the
//! `cd` to the deletion, the target is judged in both folders. A folder only
//! the running shell knows (`cd "$X"`, `popd` with nothing pushed, `cd name`
//! while `CDPATH` is set) makes a relative target unresolvable.
//!
//! Variables the command sets to something the guard can name are expanded:
//! `S=/x; rm $S/y`, `export`, a chain such as `N=$S/z`, a `for` loop over
//! literal words, `$$`, `$PWD`, `$OLDPWD`, `$(pwd)`, `$TMPDIR` (with this
//! hook's value, the same one the shell inherits) and a folder made by a
//! plain `$(mktemp -d)`. An assignment that may not run (after `&&` or `||`,
//! inside `if` or a loop) keeps the old value possible too, so one with no
//! old value is unknown. An assignment in a pipeline or a background job, or
//! in a child shell, leaves the variable unknown. A value with whitespace is
//! unknown, since the shell would split it. A command that defines a
//! function or reads a file with `source` expands nothing it set.
//!
//! What it cannot resolve it reports as unresolvable, so the guard blocks it:
//! a target holding any other variable, a relative target in a
//! folder it cannot name, a deletion inside or next to a command
//! substitution, a shell reading a pipe, and `xargs` deleting what anything
//! other than a plain `find` prints.

use super::expand::{self, Piece};
use crate::common::shell::{
    self, is_assignment, program_index, program_name, redirect_len, runner_value_options,
    shell_script_at, skip_options, Span, SHELLS,
};
use std::collections::BTreeMap;
use std::path::{Component, Path, PathBuf};

/// Reported when `rm` is reached through, or takes words from, `$(...)` or a
/// backtick pair.
pub(crate) const SUBSTITUTION_LABEL: &str = "<command substitution>";

/// Reported when `xargs` deletes paths the guard cannot see.
pub(crate) const XARGS_LABEL: &str = "<xargs input>";

/// Reported when strings nest deeper than the scan follows.
const DEPTH_LABEL: &str = "<nested shell>";

/// Reported when a shell runs a script read from a pipe.
const SHELL_INPUT_LABEL: &str = "<shell input>";

/// Reported when `find` follows links, so its paths can leave its start paths.
const FIND_LINKS_LABEL: &str = "find -L";

const MAX_DEPTH: usize = 4;

/// One thing a command deletes.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum Target {
    /// A path as the shell passes it. `tilde` is set when a leading `~` is
    /// unquoted, so the shell expands it to the home folder.
    Path { text: String, tilde: bool },
    /// Something only the running shell knows, named for the denial.
    Unresolvable(String),
}

/// What the guard knows about the shell that runs the command. `$HOME` and
/// `$TMPDIR` are taken from this hook, which Claude Code starts with the
/// same environment as its shell. A shell profile that exports another
/// `$TMPDIR` is an accepted risk: the judge still checks where it points.
pub(crate) struct Env<'a> {
    pub home: &'a str,
    /// The folder the command starts in, absolute.
    pub cwd: &'a Path,
    /// `CDPATH` is set, so `cd name` may land somewhere other than `./name`.
    pub cdpath: bool,
    /// `TMPDIR`, when set to an absolute path.
    pub tmpdir: Option<&'a Path>,
}

/// What a command deletes, and where it goes.
pub(crate) struct Found {
    pub targets: Vec<Target>,
    /// Every folder the command changes into that the guard can name.
    pub dirs: Vec<PathBuf>,
}

/// Everything `command` deletes, in order, and the folders it changes into.
pub(crate) fn scan(command: &str, env: &Env) -> Found {
    let start = Place {
        cwds: vec![Some(env.cwd.to_path_buf())],
        before_cd: Vec::new(),
        oldpwd: vec![None],
        stack: Vec::new(),
        vars: BTreeMap::new(),
    };
    let mut scan = Scan {
        env,
        out: Vec::new(),
        place: start,
        dirs: Vec::new(),
        joined_by: String::new(),
        maybe: false,
        isolated: false,
        no_vars: false,
        sourced: false,
        tainted: false,
    };
    scan.script(command, 0, false);
    Found {
        targets: scan.out,
        dirs: scan.dirs,
    }
}

/// Everything `command` deletes, in order.
#[cfg(test)]
pub(crate) fn targets(command: &str, env: &Env) -> Vec<Target> {
    scan(command, env).targets
}

/// A folder the shell may be in, or `None` when only the running shell
/// knows it.
type Dir = Option<PathBuf>;

/// Where the shell is, as far as the guard can follow it.
#[derive(Clone, Debug, PartialEq, Eq)]
struct Place {
    /// Every folder the shell may be in.
    cwds: Vec<Dir>,
    /// The folders the shell was in before a `cd`, still current when the
    /// `cd` failed and the next command runs anyway.
    before_cd: Vec<Dir>,
    /// Where `cd -` goes.
    oldpwd: Vec<Dir>,
    /// The folders `pushd` saved, last on top.
    stack: Vec<Vec<Dir>>,
    /// Variables the command sets: every value each may hold, or `None`
    /// when only the running shell knows.
    vars: BTreeMap<String, Option<Vec<String>>>,
}

/// The most values one word may expand to before it counts as unknown.
const MAX_VALUES: usize = 32;

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
    place: Place,
    dirs: Vec<PathBuf>,
    /// The operator between the command being read and the one before it,
    /// such as `&&`, `||` or `;`, empty for the first.
    joined_by: String,
    /// The command being read may not run: it follows `&&` or `||`, or sits
    /// inside `if`, `case` or a loop.
    maybe: bool,
    /// The command being read runs in a child of this shell: a pipeline
    /// part or a background job.
    isolated: bool,
    /// The command defines a function, which can set anything when called,
    /// so no assignment is trusted.
    no_vars: bool,
    /// A file was read with `source`, which can set anything.
    sourced: bool,
    /// The command sets or unsets `HOME`, so it is not expanded.
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
        self.no_vars |= defines_function(text);
        let entry = self.place.clone();
        // The place in each enclosing parenthesis group, with the group's
        // number: what a group changes ends with it.
        let mut frames: Vec<(usize, Place)> = vec![(0, entry.clone())];
        // How many `if`, `case` and loop bodies enclose the command.
        let mut blocks = 0usize;
        for (i, command) in commands.iter().enumerate() {
            self.enter(&mut frames, command.depth, command.group);
            self.joined_by = i
                .checked_sub(1)
                .map(|p| separator(&source, &commands[p], command))
                .unwrap_or_default();
            let next = commands.get(i + 1);
            let joined_after = next.map(|n| separator(&source, command, n));
            self.maybe = blocks > 0 || matches!(self.joined_by.as_str(), "&&" | "||");
            self.isolated = command.piped
                || next.is_some_and(|n| n.piped && same_pipeline(command, n))
                || joined_after.as_deref() == Some("&");
            blocks = track_blocks(&command.words, blocks);
            self.settle();
            let substituted = substituted_assignment(&commands, i);
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
            if let Some(word) = substituted {
                self.assign_text(&word, &word, true);
            }

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
            if let Some(top) = frames.last_mut() {
                top.1 = self.place.clone();
            }
        }
        let out = frames.swap_remove(0).1;
        if depth == 0 {
            self.place = out;
            return;
        }
        // A nested script: `eval` keeps what it changes, a child shell does
        // not, so either folder may be current and a variable it set is
        // unknown.
        let mut place = entry.clone();
        if out.cwds != entry.cwds {
            add(&mut place.cwds, None);
        }
        for (name, value) in &out.vars {
            if entry.vars.get(name) != Some(value) {
                place.vars.insert(name.clone(), None);
            }
        }
        self.place = place;
    }

    /// Makes the place of the parenthesis group a command at `depth` in
    /// `group` runs in current, leaving the groups that ended.
    fn enter(&mut self, frames: &mut Vec<(usize, Place)>, depth: usize, group: usize) {
        frames.truncate(depth + 1);
        if depth > 0 && frames.len() == depth + 1 && frames[depth].0 != group {
            frames.pop();
        }
        while frames.len() < depth + 1 {
            let top = frames
                .last()
                .map(|f| f.1.clone())
                .unwrap_or_else(|| self.place.clone());
            let own = if frames.len() == depth {
                group
            } else {
                usize::MAX
            };
            frames.push((own, top));
        }
        if let Some(top) = frames.last() {
            self.place = top.1.clone();
        }
    }

    /// After a `cd`, the next command not joined by `&&` may run where the
    /// shell was before, if the `cd` failed.
    fn settle(&mut self) {
        if self.joined_by == "&&" {
            return;
        }
        for dir in std::mem::take(&mut self.place.before_cd) {
            add(&mut self.place.cwds, dir);
        }
    }

    /// Follows `cd`, `pushd` or `popd`.
    fn change_dir(&mut self, name: &str, args: Words, ctx: &Ctx) {
        let operands = operands(args, ctx);
        // `-P` resolves links before `..`, so the path is left for the judge
        // to resolve on disk instead of by name.
        let physical =
            args.words.iter().take_while(|w| *w != "--").any(|w| {
                w.len() > 1 && w.starts_with('-') && !w.starts_with("--") && w.contains('P')
            });
        let old = self.place.cwds.clone();
        let mut new = match (name, operands.first()) {
            ("popd", _) => self.place.stack.pop().unwrap_or_else(|| vec![None]),
            ("pushd", None) => vec![None],
            (_, None) => vec![self.home_dir()],
            (_, Some(&k)) => self.destination(&args.words[k], &args.spans[k], ctx, physical),
        };
        // After `||` the `cd` runs only when the command before it failed,
        // and in a pipeline part or a background job it moves a child shell,
        // so the shell may not move at all.
        if self.joined_by == "||" || self.isolated {
            for dir in &old {
                add(&mut new, dir.clone());
            }
        }
        if name == "pushd" {
            self.place.stack.push(old.clone());
        }
        for dir in &old {
            add(&mut self.place.before_cd, dir.clone());
        }
        self.place.oldpwd = old;
        for dir in new.iter().flatten() {
            if !self.dirs.contains(dir) {
                self.dirs.push(dir.clone());
            }
        }
        self.place.cwds = new;
    }

    fn home_dir(&self) -> Dir {
        self.env
            .home
            .starts_with('/')
            .then(|| PathBuf::from(self.env.home))
    }

    /// The folders `cd word` may land in.
    fn destination(&self, word: &str, span: &Span, ctx: &Ctx, physical: bool) -> Vec<Dir> {
        let normalize = |path: &Path| {
            if physical {
                path.to_path_buf()
            } else {
                normalize(path)
            }
        };
        if word == "-" {
            return self.place.oldpwd.clone();
        }
        let raw = raw_text(ctx.source, span);
        let text = if span.dynamic {
            match self.expand(word, &raw).as_deref() {
                Some([one]) => one.clone(),
                _ => return vec![None],
            }
        } else {
            word.to_string()
        };
        if text.contains(['$', '`', '*', '?', '[', '{']) {
            return vec![None];
        }
        let prefix = raw.split('/').next().unwrap_or_default();
        let tilde = !span.dynamic && raw.starts_with('~') && !prefix.contains(['"', '\'', '\\']);
        if tilde {
            let Some(home) = self.home_dir() else {
                return vec![None];
            };
            return match text.strip_prefix('~') {
                Some("") => vec![Some(home)],
                Some(rest) if rest.starts_with('/') => {
                    vec![Some(normalize(&home.join(rest.trim_start_matches('/'))))]
                }
                _ => vec![None],
            };
        }
        if text.starts_with('/') {
            return vec![Some(normalize(Path::new(&text)))];
        }
        let explicit =
            text == "." || text == ".." || text.starts_with("./") || text.starts_with("../");
        if self.env.cdpath && !explicit {
            return vec![None];
        }
        self.place
            .cwds
            .iter()
            .map(|dir| dir.as_ref().map(|d| normalize(&d.join(&text))))
            .collect()
    }

    /// A relative target, judged in every folder the shell may be in. It is
    /// kept as written while the shell has not moved.
    fn relative(&self, word: &str) -> Vec<Target> {
        if self.place.cwds.len() == 1 && self.place.cwds[0].as_deref() == Some(self.env.cwd) {
            return vec![Target::Path {
                text: word.to_string(),
                tilde: false,
            }];
        }
        self.place
            .cwds
            .iter()
            .map(|dir| match dir {
                Some(dir) => Target::Path {
                    text: dir.join(word).to_string_lossy().into_owned(),
                    tilde: false,
                },
                None => Target::Unresolvable(word.to_string()),
            })
            .collect()
    }

    fn command(&mut self, cmd: Words, ctx: &Ctx, producer: Option<&shell::Command>) {
        let appends = cmd.words.first().is_some_and(|w| is_append(w));
        let Some(at) = program_at(cmd.words).filter(|_| !appends) else {
            self.assignments(cmd, ctx);
            return;
        };
        if changes_directory(&cmd.words[..at]) {
            // `env -C dir` and `sudo -D dir` move this one command only.
            let saved = std::mem::replace(&mut self.place.cwds, vec![None]);
            self.program(cmd, at, ctx, producer);
            self.place.cwds = saved;
            return;
        }
        self.program(cmd, at, ctx, producer);
    }

    fn program(&mut self, cmd: Words, at: usize, ctx: &Ctx, producer: Option<&shell::Command>) {
        let args = cmd.tail(at + 1);
        // `.` has no file name, so `program_name` would read it as empty.
        let name = match cmd.words[at].as_str() {
            "." => ".".to_string(),
            word => program_name(word),
        };
        match name.as_str() {
            name @ ("cd" | "pushd" | "popd") => self.change_dir(name, args, ctx),
            "export" | "readonly" | "local" | "declare" | "typeset" => self.declare(args, ctx),
            "for" => self.for_loop(args, ctx),
            "unset" | "read" | "mapfile" | "readarray" | "getopts" => {
                self.forget(args.words.iter().filter(|w| is_name(w)));
            }
            "printf" => {
                let named = args
                    .words
                    .windows(2)
                    .filter(|w| w[0] == "-v")
                    .map(|w| &w[1]);
                self.forget(named);
            }
            "source" | "." => {
                self.sourced = true;
                self.place.vars.clear();
                add(&mut self.place.cwds, None);
            }
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
            for target in self.classify(word, span, ctx) {
                self.push(target);
            }
        }
    }

    /// What one deletion operand names: one target, or one per folder the
    /// shell may be in for a relative path.
    fn classify(&self, word: &str, span: &Span, ctx: &Ctx) -> Vec<Target> {
        let raw = raw_text(ctx.source, span);
        if ctx.source.get(span.end) == Some(&'(') {
            // `$(` opens a substitution. Any other word that runs into `(` is
            // a zsh glob qualifier such as `x(:h)`, which can name a parent.
            if !word.ends_with('$') {
                return vec![Target::Unresolvable(format!("{raw}("))];
            }
            return vec![Target::Unresolvable(SUBSTITUTION_LABEL.to_string())];
        }
        if span.dynamic {
            return match self.expand(word, &raw) {
                Some(values) => values
                    .into_iter()
                    .flat_map(|text| {
                        if text.starts_with('/') {
                            vec![Target::Path { text, tilde: false }]
                        } else {
                            self.relative(&text)
                        }
                    })
                    .collect(),
                None if word.contains("$(") || word.contains('`') => {
                    vec![Target::Unresolvable(SUBSTITUTION_LABEL.to_string())]
                }
                None => vec![Target::Unresolvable(raw)],
            };
        }
        // A `$` left by single quotes is literal, but a path naming one is
        // more likely a mistake than a file, so it is not guessed at.
        if word.contains('$') {
            return vec![Target::Unresolvable(raw)];
        }
        // `~` expands only when nothing up to the first `/` is quoted.
        let prefix = raw.split('/').next().unwrap_or_default();
        let tilde = raw.starts_with('~') && !prefix.contains(['"', '\'', '\\']);
        if !word.starts_with('/') && !tilde {
            return self.relative(word);
        }
        vec![Target::Path {
            text: word.to_string(),
            tilde,
        }]
    }

    /// Every value `word`, with its quotes removed, may expand to, or `None`
    /// when any part of it is unknown. `raw` is the word as written: a single
    /// quote or backslash in it could make a `$` literal, so that is unknown
    /// too.
    fn expand(&self, word: &str, raw: &str) -> Option<Vec<String>> {
        if raw.contains(['\'', '\\']) {
            return None;
        }
        let mut values = vec![String::new()];
        for piece in expand::pieces(word)? {
            let options = match piece {
                Piece::Text(text) => vec![text],
                Piece::Var { name, default } => match (self.lookup(&name), default) {
                    (Some(found), Some(default)) => found
                        .into_iter()
                        .map(|v| if v.is_empty() { default.clone() } else { v })
                        .collect(),
                    (Some(found), None) => found,
                    // An unset `TMPDIR` falls back to the default, as the
                    // shell's does.
                    (None, Some(default)) if name == "TMPDIR" && self.env.tmpdir.is_none() => {
                        vec![default]
                    }
                    (None, _) => return None,
                },
                Piece::Pid => vec!["12345".to_string()],
                Piece::Pwd => self.lookup("PWD")?,
                Piece::Mktemp(args) => vec![expand::mktemp_path(&args, &self.temp())?],
            };
            if values.len() * options.len() > MAX_VALUES {
                return None;
            }
            values = values
                .iter()
                .flat_map(|v| options.iter().map(move |o| format!("{v}{o}")))
                .collect();
        }
        // The shell splits an unquoted value on whitespace, and the guard
        // does not follow that.
        if values.iter().any(|v| v.contains(char::is_whitespace)) {
            return None;
        }
        Some(values)
    }

    /// Every value variable `name` may hold, or `None` when it is unknown.
    fn lookup(&self, name: &str) -> Option<Vec<String>> {
        if name == "HOME" && self.tainted {
            return None;
        }
        if let Some(value) = self.place.vars.get(name) {
            return value.clone();
        }
        if self.sourced {
            return None;
        }
        let one = |dirs: &[Dir]| {
            dirs.iter()
                .map(|d| d.as_ref().map(|d| d.to_string_lossy().into_owned()))
                .collect::<Option<Vec<_>>>()
        };
        match name {
            "HOME" => self
                .home_dir()
                .map(|h| vec![h.to_string_lossy().into_owned()]),
            "PWD" => one(&self.place.cwds),
            "OLDPWD" => one(&self.place.oldpwd),
            "TMPDIR" => self
                .env
                .tmpdir
                .map(|t| vec![t.to_string_lossy().into_owned()]),
            _ => None,
        }
    }

    /// The folder `mktemp` uses: `TMPDIR`, or `/tmp`.
    fn temp(&self) -> String {
        self.env
            .tmpdir
            .map_or_else(|| "/tmp".to_string(), |t| t.to_string_lossy().into_owned())
    }

    /// The assignments of a command with no program, such as `S=/x`.
    fn assignments(&mut self, cmd: Words, ctx: &Ctx) {
        for (word, span) in cmd.words.iter().zip(cmd.spans) {
            if is_assignment(word) || is_append(word) {
                let raw = raw_text(ctx.source, span);
                self.assign_text(word, &raw, span.dynamic);
            }
        }
    }

    /// Records `NAME=value` (or `NAME+=value`): `word` without its quotes,
    /// `raw` as written.
    fn assign_text(&mut self, word: &str, raw: &str, dynamic: bool) {
        let Some((name, value)) = word.split_once('=') else {
            return;
        };
        let (name, append) = match name.strip_suffix('+') {
            Some(name) => (name, true),
            None => (name, false),
        };
        if !is_name(name) {
            return;
        }
        let raw_value = raw.split_once('=').map_or("", |(_, v)| v);
        let new = if self.no_vars || self.isolated || value.starts_with('(') {
            None
        } else if dynamic {
            self.expand(value, raw_value)
        } else if raw_value == "~" || raw_value.starts_with("~/") {
            self.home_dir()
                .map(|h| vec![format!("{}{}", h.to_string_lossy(), &value[1..])])
        } else {
            Some(vec![value.to_string()])
        };
        let old = self.lookup(name);
        let new = match (append, new) {
            (true, Some(tail)) => old.clone().map(|o| {
                o.iter()
                    .flat_map(|v| tail.iter().map(move |t| format!("{v}{t}")))
                    .collect()
            }),
            (_, new) => new,
        };
        let value = if self.maybe {
            // The assignment may not run, so the old value stays possible.
            match (old, new) {
                (Some(mut old), Some(new)) => {
                    for v in new {
                        if !old.contains(&v) {
                            old.push(v);
                        }
                    }
                    Some(old)
                }
                _ => None,
            }
        } else {
            new
        };
        let value = value.filter(|v| v.len() <= MAX_VALUES);
        self.place.vars.insert(name.to_string(), value);
    }

    /// The assignments of `export`, `local` and the like. An option other
    /// than `-x`, `-r`, `-g` or `-l` (an array, a reference, an integer)
    /// makes each name it sets unknown.
    fn declare(&mut self, args: Words, ctx: &Ctx) {
        let plain = args
            .words
            .iter()
            .filter(|w| w.starts_with('-') && w.len() > 1)
            .all(|w| w[1..].chars().all(|c| matches!(c, 'x' | 'r' | 'g' | 'l')));
        for (word, span) in args.words.iter().zip(args.spans) {
            if !is_assignment(word) {
                continue;
            }
            if plain {
                let raw = raw_text(ctx.source, span);
                self.assign_text(word, &raw, span.dynamic);
            } else if let Some((name, _)) = word.split_once('=') {
                self.place.vars.insert(name.to_string(), None);
            }
        }
    }

    /// `for NAME in words`: `NAME` takes each word in turn.
    fn for_loop(&mut self, args: Words, ctx: &Ctx) {
        let Some(name) = args.words.first().filter(|n| is_name(n)) else {
            return;
        };
        let mut values = Some(Vec::new());
        if args.words.get(1).map(String::as_str) != Some("in") || args.words.len() < 3 {
            values = None;
        }
        for (word, span) in args.words.iter().zip(args.spans).skip(2) {
            let Some(list) = values.as_mut() else {
                break;
            };
            if span.dynamic {
                match self.expand(word, &raw_text(ctx.source, span)) {
                    Some(found) => list.extend(found),
                    None => values = None,
                }
            } else {
                list.push(word.clone());
            }
        }
        let value = values.filter(|v| !self.no_vars && v.len() <= MAX_VALUES);
        let value = if self.maybe {
            match (self.lookup(name), value) {
                (Some(mut old), Some(new)) => {
                    old.extend(new);
                    Some(old)
                }
                _ => None,
            }
        } else {
            value
        };
        self.place.vars.insert(name.clone(), value);
    }

    /// Makes each named variable unknown.
    fn forget<'n>(&mut self, names: impl Iterator<Item = &'n String>) {
        for name in names {
            self.place.vars.insert(name.clone(), None);
        }
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
                // A command `find` or `xargs` runs is a child process: what it
                // changes does not reach this shell.
                let saved = self.place.clone();
                let before = self.out.len();
                self.command(inner, ctx, None);
                self.place = saved;
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
        let targets: Vec<Target> = if parts.starts.is_empty() {
            self.relative(".")
        } else {
            parts
                .starts
                .iter()
                .flat_map(|&k| self.classify(&args.words[k], &args.spans[k], ctx))
                .collect()
        };
        for target in targets {
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

/// Whether `word` appends to a variable, as `NAME+=value` does.
fn is_append(word: &str) -> bool {
    word.split_once("+=").is_some_and(|(name, _)| is_name(name))
}

/// Whether `word` is a shell variable name.
fn is_name(word: &str) -> bool {
    !word.is_empty() && expand::identifier_len(word) == word.len()
}

/// Whether `text` may define a shell function: `name()` or `function`.
fn defines_function(text: &str) -> bool {
    text.contains("()")
        || text
            .split(|c: char| c.is_whitespace() || matches!(c, ';' | '&' | '|' | '('))
            .any(|w| w == "function")
}

/// The `if`, `case` and loop bodies open after `words`, from `open` before.
fn track_blocks(words: &[String], open: usize) -> usize {
    let mut open = open;
    for word in words {
        match word.as_str() {
            "if" | "while" | "until" => open += 1,
            "for" | "select" | "case" => return open + 1,
            "fi" | "done" | "esac" => open = open.saturating_sub(1),
            "then" | "else" | "elif" | "do" | "!" | "{" | "}" => {}
            _ => break,
        }
    }
    open
}

/// `NAME=$(...)` written without quotes, which the lexer splits around the
/// substitution: the assignment put back as one word, when the substitution
/// is a lone `pwd` or `mktemp` and nothing is joined after it.
fn substituted_assignment(commands: &[shell::Command], i: usize) -> Option<String> {
    let command = &commands[i];
    let [word] = command.words.as_slice() else {
        return None;
    };
    let name = word.strip_suffix("=$")?;
    if !command.opens_substitution || !is_name(name) {
        return None;
    }
    let inner = commands.get(i + 1)?;
    let alone = inner.substitution
        && inner.depth == command.depth + 1
        && commands
            .get(i + 2)
            .is_none_or(|n| !n.joined && n.depth <= command.depth);
    let program = inner.words.first()?;
    if !alone || !matches!(program.as_str(), "pwd" | "mktemp") {
        return None;
    }
    Some(format!("{name}=$({})", inner.words.join(" ")))
}

/// Adds `dir` to `dirs` unless it is there already.
fn add(dirs: &mut Vec<Dir>, dir: Dir) {
    if !dirs.contains(&dir) {
        dirs.push(dir);
    }
}

/// `path` with `.` and `..` applied by name, the way `cd` reads a folder
/// before it changes into it.
fn normalize(path: &Path) -> PathBuf {
    let mut out = PathBuf::from("/");
    for part in path.components() {
        match part {
            Component::ParentDir => {
                out.pop();
            }
            Component::Normal(name) => out.push(name),
            Component::RootDir | Component::CurDir | Component::Prefix(_) => {}
        }
    }
    out
}

/// The operands of `cd`, `pushd` or `popd`: options such as `-P` and
/// redirections left out.
fn operands(args: Words, ctx: &Ctx) -> Vec<usize> {
    let mut found = Vec::new();
    let mut options = true;
    let mut i = 0;
    while i < args.words.len() {
        let redirect = redirect_len(&raw_text(ctx.source, &args.spans[i]));
        if redirect > 0 {
            i += redirect;
            continue;
        }
        let word = &args.words[i];
        if options && word == "--" {
            options = false;
        } else if options && word.len() > 1 && word.starts_with('-') {
            // `-` alone is an operand: the previous folder.
        } else {
            found.push(i);
        }
        i += 1;
    }
    found
}

/// The operator between `before` and `after`, such as `&&`, `||`, `;` or
/// `|`, with whitespace and parentheses left out, or `;` for the first
/// command. A line break alone reads as empty, which is not `&&`.
fn separator(source: &[char], before: &shell::Command, after: &shell::Command) -> String {
    let (Some(end), Some(start)) = (before.spans.last(), after.spans.first()) else {
        return ";".to_string();
    };
    source
        .get(end.end..start.start)
        .unwrap_or_default()
        .iter()
        .filter(|c| !c.is_whitespace() && !matches!(c, '(' | ')'))
        .collect()
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

/// Whether `word`, with its quotes removed, names `HOME` other than to read
/// it, as in `HOME=/`, `export HOME`, `unset HOME` or `${HOME:=x}`.
fn names_expanded(word: &str) -> bool {
    word.contains("{HOME:=")
        || word.contains("{HOME=")
        || word
            .strip_prefix("HOME")
            .is_some_and(|rest| rest.is_empty() || rest.starts_with('=') || rest.starts_with("+="))
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
        let got = targets(
            &command,
            &Env {
                home: "/Users/me",
                cwd: Path::new("/w"),
                cdpath: false,
                tmpdir: None,
            },
        );
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
        ("RM -rf $TMPDIR/x", "u:$TMPDIR/x"),
        ("RM -rf ~\"/x\"", "p:~/x"),
        (": ${HOME:=/}; RM -rf $HOME/x", "u:$HOME/x"),
        ("RM -rf '$HOME/x'", "u:'$HOME/x'"),
        ("RM -rf $REPO/x", "u:$REPO/x"),
        ("RM -rf $HOME$X", "u:$HOME$X"),
        ("RM -rf /a/x(:h)", "u:/a/x("),
        ("HOME=/ RM -rf $HOME/x", "u:$HOME/x"),
        ("export HOME=/; RM -rf $HOME/x", "u:$HOME/x"),
        ("cd /x && RM -rf build", "p:/x/build"),
        ("cd /x && RM -rf /x/build ~/y", "p:/x/build,t:~/y"),
        ("builtin cd /x && RM b", "p:/x/b"),
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
        ("cd /x && find -delete", "p:/x/."),
        ("find $(pwd) -name x -delete", S),
        ("find /a -exec ls {} + | xargs RM", "u:<xargs input>"),
        ("(echo /b; find /a) | xargs RM", "u:<xargs input>"),
        ("ls | xargs nohup RM -rf", "u:<xargs input>"),
        ("ls | xargs -I{} sh -c 'RM -rf {}'", "p:{},u:<xargs input>"),
        // The folder the shell is in.
        ("cd /x; RM b", "p:/x/b,p:/w/b"),
        ("cd /x\nRM b", "p:/x/b,p:/w/b"),
        ("cd /x && RM a; RM b", "p:/x/a,p:/x/b,p:/w/b"),
        ("cd /x && cd y && RM b", "p:/x/y/b"),
        ("cd /x/y/../z && RM b", "p:/x/z/b"),
        ("cd sub && RM ../b", "p:/w/sub/../b"),
        ("cd && RM b", "p:/Users/me/b"),
        ("cd ~ && RM b", "p:/Users/me/b"),
        ("cd ~/src && RM b", "p:/Users/me/src/b"),
        ("cd $HOME/src && RM b", "p:/Users/me/src/b"),
        ("cd -P /x && RM b", "p:/x/b"),
        ("cd \"$X\" && RM b", "u:b"),
        ("cd ~other && RM b", "u:b"),
        ("cd /x && cd - && RM b", "p:b"),
        ("(cd /x && RM a); RM b", "p:/x/a,p:b"),
        ("(cd /x); (RM b)", "p:b"),
        ("X=$(cd /x; pwd) RM b", "u:<command substitution>,p:b"),
        ("pushd /x && RM a && popd && RM b", "p:/x/a,p:b"),
        ("popd && RM b", "u:b"),
        ("eval 'cd /x'; RM b", "p:/w/b,u:b"),
        ("sh -c 'cd /x && RM a' && RM b", "p:/x/a,p:/w/b,u:b"),
        ("cd /x && RM /abs ~/y", "p:/abs,t:~/y"),
        ("cd /x && find . -delete", "p:/x/."),
        ("cd /x &&\n  RM b", "p:/x/b"),
        ("true || cd /x && RM b", "p:/x/b,p:/w/b"),
        ("cd /x || exit 1; RM b", "p:/x/b,p:/w/b"),
        ("cd -P /x/l/.. && RM b", "p:/x/l/../b"),
        ("cd /x | true; RM b", "p:/x/b,p:/w/b"),
        // Variables the command sets.
        ("S=/x; RM -rf $S/y", "p:/x/y"),
        ("S=\"/x\" ; RM -rf \"$S\"", "p:/x"),
        ("S=/x; RM -rf \"${S}/y\"", "p:/x/y"),
        ("export O=/x/o; RM -f $O/r", "p:/x/o/r"),
        ("S=/x; N=$S/n; RM -rf $N", "p:/x/n"),
        ("S=/x; S+=/y; RM $S", "p:/x/y"),
        ("S=~/x; RM $S", "p:/Users/me/x"),
        ("S='~/x'; RM $S", "p:~/x"),
        ("S=rel; RM $S", "p:rel"),
        ("cd /x && S=rel; RM $S", "u:$S"),
        ("cd /x; S=rel; RM $S", "p:/x/rel,p:/w/rel"),
        ("d=/x; for f in a b; do RM \"$d/$f\"; done", "p:/x/a,p:/x/b"),
        ("for f in $(ls); do RM $f; done", "u:$f"),
        ("for f; do RM $f; done", "u:$f"),
        ("S=/x; S=$(cat w); RM $S/y", "u:$S/y"),
        ("S=/x; unset S; RM $S/y", "u:$S/y"),
        ("S=/x; read S; RM $S/y", "u:$S/y"),
        ("S=/x; printf -v S y; RM $S/y", "u:$S/y"),
        ("S=/x; source env.sh; RM $S/y", "u:$S/y"),
        ("S=/x; . ./env.sh; RM $HOME/y", "u:$HOME/y"),
        ("true && S=/x; RM $S/y", "u:$S/y"),
        ("S=/a; true && S=/b; RM $S", "p:/a,p:/b"),
        ("if c; then S=/x; fi; RM $S", "u:$S"),
        ("S=/x; if c; then S=/y; fi; RM $S", "p:/x,p:/y"),
        ("S=/x | cat; RM $S", "u:$S"),
        ("S=/x & RM $S", "u:$S"),
        ("f() { S=/x; }; RM $S", "u:$S"),
        ("S=/x; f() { :; }; RM $S", "u:$S"),
        ("sh -c 'S=/x'; RM $S", "u:$S"),
        ("eval 'S=/x'; RM $S", "u:$S"),
        ("X=1 RM $X", "u:$X"),
        ("S='/a /b'; RM $S", "u:$S"),
        ("declare -a S=(/x); RM $S", "u:$S"),
        ("declare -x S=/x; RM $S", "p:/x"),
        ("S=/x; RM '$S'", "u:'$S'"),
        ("S=/x; RM \\$S", "u:\\$S"),
        ("RM /tmp/st.$$", "p:/tmp/st.12345"),
        ("RM \"$PWD/x\"", "p:/w/x"),
        ("cd /x; RM \"$PWD/y\"", "p:/x/y,p:/w/y"),
        ("cd \"$Z\"; RM \"$PWD/y\"", "u:\"$PWD/y\""),
        ("cd /x && cd /y && RM \"$OLDPWD/z\"", "p:/x/z"),
        ("RM \"$OLDPWD/z\"", "u:\"$OLDPWD/z\""),
        ("RM -rf \"$(pwd)/t\"", "p:/w/t"),
        ("RM -rf \"$(cat x)/t\"", S),
        ("d=$(mktemp -d); RM -r $d", "p:/tmp/tmp.mktemp"),
        ("d=\"$(mktemp)\"; RM -f \"$d\"", "p:/tmp/tmp.mktemp"),
        ("d=$(mktemp -d ./x.XXXX); RM -rf $d", "u:$d"),
        ("d=$(mktemp -d)x; RM -rf $d", "u:$d"),
        ("RM \"${TMPDIR:-/tmp}/p.json\"", "p:/tmp/p.json"),
        ("RM ${X:-/tmp}/p", "u:${X:-/tmp}/p"),
        ("RM $1/x", "u:$1/x"),
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
        let mut wrong = Vec::new();
        for (case, want) in FOUND {
            let (command, got) = scan(case);

            if got != spec(want) {
                wrong.push(format!("{command:?}: got {got:?}"));
            }
        }

        assert!(wrong.is_empty(), "{}", wrong.join("\n"));
    }

    #[test]
    fn data_that_mentions_a_deletion_is_never_a_command() {
        for case in DATA {
            let (command, got) = scan(case);

            assert_eq!(got, Vec::<Target>::new(), "{command:?}");
        }
    }

    #[test]
    fn tmpdir_expands_to_the_value_this_hook_sees() {
        let env = Env {
            home: "/Users/me",
            cwd: Path::new("/w"),
            cdpath: false,
            tmpdir: Some(Path::new("/var/T")),
        };
        let rm = "r".to_string() + "m";
        let cases = [
            (format!("{rm} $TMPDIR/x"), "p:/var/T/x"),
            (format!("{rm} \"${{TMPDIR:-/tmp}}/x\""), "p:/var/T/x"),
            (format!("d=$(mktemp -d); {rm} -r $d"), "p:/var/T/tmp.mktemp"),
        ];
        for (command, want) in cases {
            assert_eq!(targets(&command, &env), spec(want), "{command}");
        }
    }

    #[test]
    fn a_relative_cd_with_cdpath_set_lands_in_an_unknown_folder() {
        let env = Env {
            home: "/Users/me",
            cwd: Path::new("/w"),
            cdpath: true,
            tmpdir: None,
        };
        let rm = "r".to_string() + "m";

        let named = targets(&format!("cd sub && {rm} b"), &env);
        let explicit = targets(&format!("cd ./sub && {rm} b"), &env);

        assert_eq!(named, spec("u:b"));
        assert_eq!(explicit, spec("p:/w/sub/b"));
    }

    #[test]
    fn the_folders_a_command_changes_into_are_reported() {
        let env = Env {
            home: "/Users/me",
            cwd: Path::new("/w"),
            cdpath: false,
            tmpdir: None,
        };
        let rm = "r".to_string() + "m";

        let found = super::scan(&format!("cd /x && cd y; cd \"$Z\"; {rm} a"), &env);

        assert_eq!(found.dirs, vec![PathBuf::from("/x"), PathBuf::from("/x/y")]);
    }

    #[test]
    fn only_a_write_or_unset_of_home_stops_expansion() {
        assert!(names_expanded("HOME=/"));
        assert!(names_expanded("HOME"));
        assert!(names_expanded("HOME+=x"));
        assert!(names_expanded("${HOME:=/}"));
        assert!(!names_expanded("$HOME"));
        assert!(!names_expanded("${HOME:-/x}"));
        assert!(!names_expanded("HOMEBREW_PREFIX=x"));
    }
}
