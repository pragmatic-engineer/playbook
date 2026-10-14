// SPDX-FileCopyrightText: 2026 Igor Santos
// SPDX-License-Identifier: Apache-2.0

//! Bash commands that write a Claude Code settings file. Settings change only
//! through the `playbook` binary (`init`, `uninstall`, `config set`, `doctor`),
//! which asks for consent, merges and marks what it adds. A shell write
//! (`>`, `>>`, `tee`, `sed -i`, `cp`, `mv`, an interpreter one-liner) skips
//! all of that, so it is denied.
//!
//! The check is lexical and best effort, like the rest of the guard. It reads
//! a file with `cat`, `jq` or `git checkout` without trouble.

use super::{expand_home, is_claude_settings, lexical, redirects, wrapped_commands};
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
const WRITE_MARKERS: [&str; 9] = [
    "write", "dump(", "'w'", "\"w\"", "'wb'", "\"wb\"", "'a'", "\"a\"", "rename",
];

/// Whether `word` is a settings file path, with `~`, `$HOME` and `${HOME}`
/// expanded and `.` and `..` resolved by text.
fn names_settings(word: &str, home: &str) -> bool {
    if word.is_empty() || word.starts_with('-') {
        return false;
    }
    let expanded = expand_home(word, home);
    is_claude_settings(&lexical(Path::new(&expanded)))
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

/// The target of a redirect that writes, in the words of one command.
fn redirect_targets(words: &[String]) -> Vec<&str> {
    let mut targets = Vec::new();
    for (i, word) in words.iter().enumerate() {
        if !redirects(word) {
            continue;
        }
        let after = word
            .trim_start_matches(|c: char| c.is_ascii_digit())
            .trim_start_matches(['>', '|', '&']);
        if !after.is_empty() {
            targets.push(after);
        } else if let Some(next) = words.get(i + 1) {
            targets.push(next.as_str());
        }
    }
    targets
}

/// Whether this simple command writes a Claude Code settings file.
pub(super) fn writes_claude_settings(words: &[String], program_at: usize, home: &str) -> bool {
    writes_at(words, program_at, home, 0)
}

fn writes_at(words: &[String], program_at: usize, home: &str, depth: usize) -> bool {
    if depth < 3
        && wrapped_commands(words, program_at).iter().any(|inner| {
            program_index(inner).is_some_and(|at| writes_at(inner, at, home, depth + 1))
        })
    {
        return true;
    }
    if redirect_targets(words)
        .iter()
        .any(|t| names_settings(t, home))
    {
        return true;
    }
    let program = program_name(&words[program_at]);
    let args = &words[program_at + 1..];
    let operands = || args.iter().filter(|w| !w.starts_with('-'));
    match program.as_str() {
        "tee" => operands().any(|w| names_settings(w, home)),
        "cp" | "install" | "rsync" | "ln" | "mv" => {
            operands().last().is_some_and(|w| names_settings(w, home))
        }
        "dd" => args
            .iter()
            .filter_map(|w| w.strip_prefix("of="))
            .any(|w| names_settings(w, home)),
        "sed" | "perl" | "ruby"
            if args
                .iter()
                .any(|w| w.starts_with("-i") || w.starts_with("-pi") || w == "--in-place") =>
        {
            operands().any(|w| names_settings(w, home))
        }
        "truncate" | "rm" | "unlink" | "shred" => operands().any(|w| names_settings(w, home)),
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
        simple_commands(command).iter().any(|words| {
            program_index(words).is_some_and(|at| writes_claude_settings(words, at, HOME))
        })
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
        ] {
            assert!(!denied(command), "{command}");
        }
    }
}
