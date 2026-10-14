// SPDX-FileCopyrightText: 2026 Igor Santos
// SPDX-License-Identifier: Apache-2.0

//! Bash commands that write a Claude Code settings file. Settings change only
//! through the `playbook` binary (`init`, `uninstall`, `config set`, `doctor`),
//! which asks for consent, merges and marks what it adds. A shell write
//! (`>`, `>>`, `tee`, `sed -i`, `cp`, `mv`, an interpreter one-liner) skips
//! all of that, so it is denied.
//!
//! The check is lexical and best effort, like the rest of the guard, and not
//! a security boundary. It reads a file with `cat` or `jq` without trouble.
//! Known gaps: heredoc and script-file interpreters, `nohup` and `xargs`
//! wrappers around a writer, `git checkout`, `git restore` and `git update-index`
//! of the file, archive extraction and a path built at run time.

use super::{
    expand_home, is_claude_settings, is_short_cluster_with, lexical, settings_path,
    wrapped_commands,
};
use crate::common::shell::program_index;
use crate::common::shell::program_name;
use std::path::Path;

/// Programs that run a script given as a word.
const INTERPRETERS: [&str; 9] = [
    "python",
    "python3",
    "node",
    "deno",
    "bun",
    "ruby",
    "perl",
    "php",
    "osascript",
];

/// Text in a script that means it writes.
const WRITE_MARKERS: [&str; 14] = [
    "writefile",
    "write_text",
    "write_bytes",
    "appendfile",
    "copyfile",
    "shutil.copy",
    "os.replace",
    "rename",
    "'w'",
    "\"w\"",
    "'wb'",
    "\"wb\"",
    "'w+'",
    "'a'",
];

/// What a check knows about the command around the word it judges.
#[derive(Clone, Copy)]
struct Scene<'a> {
    home: &'a str,
    /// An earlier `cd` in the same call went into a `.claude` folder, so a bare
    /// `settings.json` is the settings file.
    in_claude_dir: bool,
}

/// Whether `word` is a settings file path, with `~`, `$HOME` and `${HOME}`
/// expanded and `.` and `..` resolved by text. An absolute path is also
/// followed through a link, as the Edit and Write check does.
fn names_settings(word: &str, scene: Scene) -> bool {
    if word.is_empty() || word.starts_with('-') {
        return false;
    }
    let expanded = expand_home(word, scene.home);
    if is_claude_settings(&lexical(Path::new(&expanded))) {
        return true;
    }
    if scene.in_claude_dir {
        let name = word.to_lowercase();
        if name == "settings.json" || name == "settings.local.json" {
            return true;
        }
    }
    expanded.starts_with('/') && settings_path(&expanded, scene.home).is_some()
}

/// Whether `word` is a directory named `.claude`.
fn names_claude_dir(word: &str, home: &str) -> bool {
    let expanded = expand_home(word, home);
    lexical(Path::new(expanded.trim_end_matches('/')))
        .file_name()
        .is_some_and(|n| n.to_string_lossy().eq_ignore_ascii_case(".claude"))
}

/// Whether a `cd` command moves into a `.claude` folder.
pub(super) fn enters_claude_dir(words: &[String], program_at: usize, home: &str) -> bool {
    program_name(&words[program_at]) == "cd"
        && words[program_at + 1..]
            .iter()
            .find(|w| !w.starts_with('-'))
            .is_some_and(|w| names_claude_dir(w, home))
}

/// Whether a script text names a settings file anywhere in it.
fn script_names_settings(word: &str) -> bool {
    let lower = word.to_lowercase();
    let mut rest = lower.as_str();
    while let Some(at) = rest.find(".claude/settings") {
        let tail = &rest[at..];
        if let Some(end) = tail.find(".json") {
            let after = &tail[end + 5..];
            if !after.starts_with(|c: char| c.is_alphanumeric() || matches!(c, '.' | '_' | '-')) {
                return true;
            }
        }
        rest = &rest[at + 1..];
    }
    false
}

/// The target of each redirect that writes, in the words of one command. A
/// redirect can be a word of its own (`>`), start a word (`>file`, `2>file`),
/// or sit inside one (`{}>file`). `>&2` and `2>&1` write to a descriptor.
fn redirect_targets(words: &[String]) -> Vec<&str> {
    let mut targets = Vec::new();
    for (i, word) in words.iter().enumerate() {
        let Some(at) = word.rfind('>') else { continue };
        let after = word[at + 1..].trim_start_matches(['|', '&']);
        if word[at + 1..].starts_with('&') && after.chars().all(|c| c.is_ascii_digit() || c == '-')
        {
            continue;
        }
        if !after.is_empty() {
            targets.push(after);
        } else if let Some(next) = words.get(i + 1) {
            targets.push(next.as_str());
        }
    }
    targets
}

/// Whether the previous command ended in a bare `>` (the lexer splits `>|`
/// and `>&` from their target) and this one starts with a settings file.
pub(super) fn is_open_redirect_target(
    previous: Option<&[String]>,
    words: &[String],
    home: &str,
    in_claude_dir: bool,
) -> bool {
    let scene = Scene {
        home,
        in_claude_dir,
    };
    previous
        .and_then(|p| p.last())
        .is_some_and(|w| w.ends_with('>'))
        && words.first().is_some_and(|w| names_settings(w, scene))
}

/// Whether a command with or without a program redirects output into a
/// settings file, as in `> ~/.claude/settings.json`.
pub(super) fn redirects_to_settings(words: &[String], home: &str, in_claude_dir: bool) -> bool {
    let scene = Scene {
        home,
        in_claude_dir,
    };
    redirect_targets(words)
        .iter()
        .any(|t| names_settings(t, scene))
}

/// Whether this simple command writes a Claude Code settings file.
pub(super) fn writes_claude_settings(
    words: &[String],
    program_at: usize,
    home: &str,
    in_claude_dir: bool,
) -> bool {
    let scene = Scene {
        home,
        in_claude_dir,
    };
    writes_at(words, program_at, scene, 0)
}

/// Whether `operand` is a `.claude` folder and a source operand is a settings
/// file, as in `cp /tmp/settings.json ~/.claude/`.
fn copies_into_claude_dir(operands: &[&String], target_dir: Option<&str>, home: &str) -> bool {
    let sources: &[&String] = if target_dir.is_some() {
        operands
    } else {
        match operands.split_last() {
            Some((_, rest)) => rest,
            None => return false,
        }
    };
    let dir = target_dir.or_else(|| operands.last().map(|w| w.as_str()));
    dir.is_some_and(|d| names_claude_dir(d, home))
        && sources.iter().any(|s| {
            let name = s.rsplit('/').next().unwrap_or(s).to_lowercase();
            name == "settings.json" || name == "settings.local.json"
        })
}

fn writes_at(words: &[String], program_at: usize, scene: Scene, depth: usize) -> bool {
    if depth < 3
        && wrapped_commands(words, program_at).iter().any(|inner| {
            program_index(inner).is_some_and(|at| writes_at(inner, at, scene, depth + 1))
        })
    {
        return true;
    }
    if redirect_targets(words)
        .iter()
        .any(|t| names_settings(t, scene))
    {
        return true;
    }
    let program = program_name(&words[program_at]);
    let args = &words[program_at + 1..];
    let operands = || args.iter().filter(|w| !w.starts_with('-'));
    let target_dir = args.iter().enumerate().find_map(|(i, w)| {
        w.strip_prefix("--target-directory=").or_else(|| {
            (w == "-t" || w == "--target-directory").then(|| args.get(i + 1).map(String::as_str))?
        })
    });
    match program.as_str() {
        "tee" | "sponge" | "patch" => operands().any(|w| names_settings(w, scene)),
        "mv" | "cp" | "install" | "rsync" | "ln" => {
            let all: Vec<&String> = operands().collect();
            let moved_away = program == "mv" && all.iter().any(|w| names_settings(w, scene));
            moved_away
                || all.last().is_some_and(|w| names_settings(w, scene))
                || target_dir.is_some_and(|d| names_settings(d, scene))
                || copies_into_claude_dir(&all, target_dir, scene.home)
        }
        "dd" => args
            .iter()
            .filter_map(|w| w.strip_prefix("of="))
            .any(|w| names_settings(w, scene)),
        "sed" | "perl" | "ruby" | "awk" | "gawk"
            if args
                .iter()
                .any(|w| is_short_cluster_with(w, 'i') || w.starts_with("--in-place")) =>
        {
            operands().any(|w| names_settings(w, scene))
        }
        "truncate" | "rm" | "unlink" | "shred" => operands().any(|w| names_settings(w, scene)),
        name if INTERPRETERS.contains(&name) || name.starts_with("python") => {
            args.iter().any(|w| {
                script_names_settings(w) && {
                    let lower = w.to_lowercase();
                    WRITE_MARKERS.iter().any(|m| lower.contains(m))
                }
            })
        }
        _ => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::common::shell::{program_index, simple_commands};

    const HOME: &str = "/Users/me";

    fn denied(command: &str) -> bool {
        let mut in_claude = false;
        let mut previous: Option<Vec<String>> = None;
        for words in simple_commands(command) {
            if is_open_redirect_target(previous.as_deref(), &words, HOME, in_claude) {
                return true;
            }
            previous = Some(words.clone());
            let Some(at) = program_index(&words) else {
                if redirects_to_settings(&words, HOME, in_claude) {
                    return true;
                }
                continue;
            };
            if writes_claude_settings(&words, at, HOME, in_claude) {
                return true;
            }
            in_claude |= enters_claude_dir(&words, at, HOME);
        }
        false
    }

    #[test]
    fn shell_writes_to_a_settings_file_are_denied() {
        for command in [
            "echo '{}' > ~/.claude/settings.json",
            "echo '{}' >> $HOME/.claude/settings.json",
            "echo '{}' > \"${HOME}/.claude/settings.local.json\"",
            "echo '{}' >.claude/settings.json",
            "cat x 2> .claude/settings.json",
            "printf x | tee ~/.claude/settings.json",
            "printf x | tee -a '/work/repo/.claude/settings.local.json'",
            "sed -i 's/a/b/' ~/.claude/settings.json",
            "sed -i.bak s/a/b/ .claude/settings.json",
            "perl -pi -e 's/a/b/' ~/.claude/settings.json",
            "cp /tmp/new.json ~/.claude/settings.json",
            "mv /tmp/new.json ./.claude/settings.json",
            "install -m 600 /tmp/new.json $HOME/.claude/settings.json",
            "dd if=/tmp/new.json of=~/.claude/settings.json",
            "jq '.a=1' ~/.claude/settings.json > ~/.claude/settings.json",
            "python3 -c \"import json;json.dump({}, open('/Users/me/.claude/settings.json','w'))\"",
            "node -e \"require('fs').writeFileSync(process.env.HOME+'/.claude/settings.json','{}')\"",
            "rm ~/.claude/settings.json",
            "echo '{}' > ~/.claude/../.claude/settings.json",
            "echo '{}' > ~/.CLAUDE/Settings.json",
            "echo '{}'>~/.claude/settings.json",
            "echo x >| ~/.claude/settings.json",
            "echo x &> ~/.claude/settings.json",
            "> ~/.claude/settings.json",
            "cp /tmp/settings.json ~/.claude/",
            "mv x/settings.local.json .claude",
            "cp -t ~/.claude /tmp/settings.json",
            "mv ~/.claude/settings.json /tmp/old.json",
            "sed -Ei s/a/b/ ~/.claude/settings.json",
            "sed --in-place=.bak s/a/b/ ~/.claude/settings.json",
            "perl -0pi -e 's/a/b/' ~/.claude/settings.json",
            "jq '.a=1' ~/.claude/settings.json | sponge ~/.claude/settings.json",
            "cd ~/.claude && echo '{}' > settings.json",
            "cd .claude && cp /tmp/x.json settings.local.json",
        ] {
            assert!(denied(command), "{command}");
        }
    }

    #[test]
    fn reading_a_settings_file_or_writing_another_file_is_allowed() {
        for command in [
            "cat ~/.claude/settings.json",
            "jq . ~/.claude/settings.json",
            "jq '.hooks' .claude/settings.json > /tmp/hooks.json",
            "git checkout -- .claude/settings.json",
            "git diff .claude/settings.json",
            "cp ~/.claude/settings.json /tmp/backup.json",
            "grep hooks ~/.claude/settings.json",
            "echo hi > /tmp/settings.json",
            "echo hi > notes/settings.json",
            "sed -i s/a/b/ src/settings.json",
            "python3 -c \"print(open('/Users/me/.claude/settings.json').read())\"",
            "node -e \"console.log(require('/Users/me/.claude/settings.json'))\"",
            "cp a ~/.claude/settings.json.bak",
            "echo x > ~/.claude/CLAUDE.md",
            "echo x > ~/.claude/settings.example.json",
            "cp ~/.claude/settings.json ~/.claude/settings.backup.json",
            "echo x 2>&1 | cat",
            "echo x >&2",
            "cd ~/.claude && cat settings.json",
            "cd src && echo x > settings.json",
            "python3 -c \"import json,sys;json.dump(json.load(open('/Users/me/.claude/settings.json')),sys.stdout)\"",
            "cp /tmp/notes.md ~/.claude/",
        ] {
            assert!(!denied(command), "{command}");
        }
    }
}
