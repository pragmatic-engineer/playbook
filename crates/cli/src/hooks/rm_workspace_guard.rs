// SPDX-FileCopyrightText: 2026 Igor Santos
// SPDX-License-Identifier: Apache-2.0

//! Denies the plain `rm -rf /` family (ADR 0045, 2026-10-19 amendment): an
//! `rm`, `rmdir`, `unlink` or `shred` whose target is the filesystem root or
//! the user's home folder.
//!
//! The command is split on `;`, `&`, `|` and newlines, and each simple
//! command is read word by word. The guard does not follow `cd`, variables,
//! subshells, `find -delete`, `xargs`, here-docs or any other shell feature.
//! Anything it cannot read passes. A path inside home is allowed.
//!
//! Best-effort protection against an accidental slip, not a security
//! boundary.

use crate::common::emit_pre_deny;
use crate::common::payload::Payload;

/// The commands whose targets are checked.
const DELETERS: [&str; 4] = ["rm", "rmdir", "unlink", "shred"];

/// Words that run the next word as a command.
const WRAPPERS: [&str; 6] = ["sudo", "command", "env", "exec", "nohup", "time"];

/// Wrapper options whose next word is a value, not the command.
const VALUE_OPTIONS: [&str; 3] = ["-u", "-g", "-C"];

/// What the model sees when a command is denied.
const DENIAL: &str = "Blocked by the rm guard: rm of the filesystem root or your home folder is not run by the model. If you mean it, run the command yourself in the prompt with `!`.";

pub fn run(payload: &Payload) {
    let cmd = payload.field(".tool_input.command");
    let home = std::env::var("HOME").unwrap_or_default();
    if denies(&cmd, &home) {
        emit_pre_deny(DENIAL);
    }
}

/// Whether `command` deletes the filesystem root or `home`.
fn denies(command: &str, home: &str) -> bool {
    simple_commands(command)
        .iter()
        .any(|words| deletes_root_or_home(words, home))
}

fn deletes_root_or_home(words: &[String], home: &str) -> bool {
    let mut rest = words.iter().map(String::as_str).peekable();
    // Skip wrappers, their options and `NAME=value` words.
    while let Some(word) = rest.peek().copied() {
        let wrapper = WRAPPERS.contains(&base_name(word));
        let skipped = wrapper || word.starts_with('-') || is_assignment(word);
        if !skipped {
            break;
        }
        rest.next();
        if VALUE_OPTIONS.contains(&word) {
            rest.next();
        }
    }
    let Some(name) = rest.next() else {
        return false;
    };
    DELETERS.contains(&base_name(name)) && rest.any(|arg| is_root_or_home(arg, home))
}

fn base_name(word: &str) -> &str {
    word.rsplit('/').next().unwrap_or(word)
}

fn is_assignment(word: &str) -> bool {
    word.split_once('=').is_some_and(|(name, _)| {
        !name.is_empty() && name.chars().all(|c| c.is_ascii_alphanumeric() || c == '_')
    })
}

/// Whether `target` is `/`, `/*`, `~`, `$HOME` or the literal `home`, with
/// an optional trailing `/` or `/*`.
fn is_root_or_home(target: &str, home: &str) -> bool {
    let mut path = target;
    while let Some(trimmed) = path.strip_suffix("/*").or_else(|| path.strip_suffix('/')) {
        path = trimmed;
    }
    let home = home.trim_end_matches('/');
    !target.is_empty()
        && (path.is_empty()
            || matches!(path, "~" | "$HOME" | "${HOME}")
            || (!home.is_empty() && path == home))
}

/// The simple commands of `command` as lists of words, with quotes removed.
/// A `#` that starts a word ends the line.
fn simple_commands(command: &str) -> Vec<Vec<String>> {
    let mut commands = Vec::new();
    let mut words: Vec<String> = Vec::new();
    let mut word = String::new();
    let mut in_word = false;
    let mut quote: Option<char> = None;
    let mut chars = command.chars();
    while let Some(c) = chars.next() {
        match (quote, c) {
            (Some(q), c) if c == q => quote = None,
            (Some('"'), '\\') => {
                if let Some(next) = chars.next() {
                    if !matches!(next, '"' | '\\' | '$' | '`') {
                        word.push('\\');
                    }
                    word.push(next);
                }
            }
            (Some(_), c) => word.push(c),
            (None, '\'' | '"') => {
                quote = Some(c);
                in_word = true;
            }
            (None, '\\') => match chars.next() {
                Some('\n') => {}
                next => {
                    in_word = true;
                    word.extend(next);
                }
            },
            (None, '#') if !in_word => {
                for rest in chars.by_ref() {
                    if rest == '\n' {
                        break;
                    }
                }
                commands.push(std::mem::take(&mut words));
            }
            (None, ';' | '&' | '|' | '\n') => {
                if in_word {
                    words.push(std::mem::take(&mut word));
                    in_word = false;
                }
                commands.push(std::mem::take(&mut words));
            }
            (None, c) if c.is_whitespace() => {
                if in_word {
                    words.push(std::mem::take(&mut word));
                    in_word = false;
                }
            }
            (None, c) => {
                word.push(c);
                in_word = true;
            }
        }
    }
    if in_word {
        words.push(word);
    }
    commands.push(words);
    commands
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::common::test_support::scratch_dir;

    /// Command lines the guard denies. `{h}` is the scratch home.
    const DENIED: [&str; 33] = [
        "rm -rf /",
        "rm -rf /*",
        "rm -rf //",
        "rm -fr /",
        "rm -r -f /",
        "rm -rf --no-preserve-root /",
        "rm -rf -- /",
        "rm -rf ~",
        "rm -rf ~/",
        "rm -rf ~/*",
        "rm -rf $HOME",
        "rm -rf ${HOME}",
        "rm -rf \"$HOME\"",
        "rm -rf '$HOME'",
        "rm -rf $HOME/",
        "rm -rf $HOME/*",
        "rm -rf {h}",
        "rm -rf {h}/",
        "rm -rf \"{h}\"",
        "rm -rf {h}/*",
        "sudo rm -rf /",
        "sudo -n rm -rf ~",
        "command rm -rf /",
        "env FOO=1 rm -rf /",
        "/bin/rm -rf /",
        "/usr/bin/sudo rm -rf /",
        "sudo -u root rm -rf /",
        "env -u HOME rm -rf ~",
        "sudo \\\n rm -rf /",
        "rm -rf \\\n/",
        "ls && rm -rf /",
        "echo done; rm -rf ~ || true",
        "cat x | rm -rf $HOME\nls",
    ];

    /// Command lines the guard allows.
    const ALLOWED: [&str; 22] = [
        "rm -rf ./target",
        "rm -rf target",
        "rm -rf ~/projects/x/target",
        "rm -rf $HOME/projects/x",
        "rm -rf {h}/projects/x",
        "rm file",
        "rm -rf /tmp/x",
        "rm -rf /Users/other",
        "rm -rf /var/folders/ab/cd",
        "rm -rf ~other",
        "rm -rf ~/.cache/x/*",
        "ls /",
        "ls ~",
        "echo rm -rf /",
        "echo 'rm -rf /'",
        "git rm -r --cached /",
        "git commit -m \"rm -rf ~\"",
        "grep -r \"rm -rf /\" docs",
        "ls # rm -rf /",
        "find / -name x",
        "rm -rf $(echo /)",
        "",
    ];

    fn with_home(line: &str, home: &str) -> String {
        line.replace("{h}", home)
    }

    #[test]
    fn denies_the_root_and_home_family() {
        let home = scratch_dir("rm-guard-denied");
        let home = home.to_string_lossy();
        for line in DENIED {
            let line = with_home(line, &home);
            assert!(denies(&line, &home), "should deny: {line}");
        }
    }

    #[test]
    fn allows_everything_else() {
        let home = scratch_dir("rm-guard-allowed");
        let home = home.to_string_lossy();
        for line in ALLOWED {
            let line = with_home(line, &home);
            assert!(!denies(&line, &home), "should allow: {line}");
        }
    }

    #[test]
    fn an_empty_home_does_not_match_every_path() {
        assert!(!denies("rm -rf ./x", ""));
        assert!(denies("rm -rf /", ""));
    }
}
