// SPDX-FileCopyrightText: 2026 Igor Santos
// SPDX-License-Identifier: Apache-2.0

//! The optional parts of `playbook init` and how each one gets decided: a flag
//! wins, else a question in a terminal, else the default.

use clap::Args;
use std::io::{self, BufRead, IsTerminal, Write};

/// The flags, each part as a `--x` and `--no-x` pair. The last one given wins.
#[derive(Args, Debug, Default, Clone)]
pub struct Flags {
    /// Wire the hook entries into settings.json
    #[arg(long, overrides_with = "no_hooks")]
    pub hooks: bool,
    /// Do not wire hooks
    #[arg(long, overrides_with = "hooks")]
    pub no_hooks: bool,
    /// Merge playbook's shared settings (permissions, status line) into settings.json
    #[arg(long, overrides_with = "no_settings")]
    pub settings: bool,
    /// Do not touch the shared settings
    #[arg(long, overrides_with = "settings")]
    pub no_settings: bool,
    /// Put the playbook binary on PATH for every shell start
    #[arg(long, overrides_with = "no_path")]
    pub path: bool,
    /// Do not edit shell files for PATH
    #[arg(long, overrides_with = "path")]
    pub no_path: bool,
    /// Install the `ccc` and `ccd` launcher shortcuts and wire your rc file
    #[arg(long, overrides_with = "no_aliases")]
    pub aliases: bool,
    /// Do not install the launcher
    #[arg(long, overrides_with = "aliases")]
    pub no_aliases: bool,
    /// Install the playbook system prompt (an existing copy is refreshed either way)
    #[arg(long, overrides_with = "no_system_prompt")]
    pub system_prompt: bool,
    /// Do not install the system prompt
    #[arg(long, overrides_with = "system_prompt")]
    pub no_system_prompt: bool,
    /// Never ask: use the default for every part you did not answer with a flag
    #[arg(long, short = 'y')]
    pub yes: bool,
}

/// What `init` will do for each optional part.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Choices {
    pub hooks: bool,
    pub settings: bool,
    pub path: bool,
    pub aliases: bool,
    pub system_prompt: bool,
}

fn pick(on: bool, off: bool) -> Option<bool> {
    if on {
        Some(true)
    } else if off {
        Some(false)
    } else {
        None
    }
}

/// Decide every part. `ask(question, default)` is called only for a part with
/// no flag, and only when `interactive` is true.
pub fn resolve(
    flags: &Flags,
    interactive: bool,
    ask: &mut dyn FnMut(&str, bool) -> bool,
) -> Choices {
    let mut decide = |on: bool, off: bool, question: &str, default: bool| {
        pick(on, off).unwrap_or_else(|| {
            if interactive && !flags.yes {
                ask(question, default)
            } else {
                default
            }
        })
    };
    Choices {
        hooks: decide(
            flags.hooks,
            flags.no_hooks,
            "Wire the playbook hooks into ~/.claude/settings.json? (safety guards, session hooks)",
            true,
        ),
        settings: decide(
            flags.settings,
            flags.no_settings,
            "Merge playbook's shared settings (permissions, status line) into settings.json?",
            true,
        ),
        path: decide(
            flags.path,
            flags.no_path,
            "Put the playbook binary on PATH in your shell startup files?",
            true,
        ),
        aliases: decide(
            flags.aliases,
            flags.no_aliases,
            "Install the ccc and ccd launcher shortcuts?",
            false,
        ),
        system_prompt: decide(
            flags.system_prompt,
            flags.no_system_prompt,
            "Install the playbook system prompt?",
            false,
        ),
    }
}

/// Whether both ends are a terminal, so a question can be shown and answered.
pub fn stdio_is_interactive() -> bool {
    io::stdin().is_terminal() && io::stdout().is_terminal()
}

/// Ask on the terminal. An empty answer or end of input gives `default`.
pub fn ask_on_terminal(question: &str, default: bool) -> bool {
    let hint = if default { "[Y/n]" } else { "[y/N]" };
    print!("{question} {hint} ");
    let _ = io::stdout().flush();
    let mut line = String::new();
    if io::stdin().lock().read_line(&mut line).unwrap_or(0) == 0 {
        println!();
        return default;
    }
    parse_answer(&line, default)
}

fn parse_answer(line: &str, default: bool) -> bool {
    match line.trim().to_ascii_lowercase().as_str() {
        "y" | "yes" => true,
        "n" | "no" => false,
        _ => default,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn never(_: &str, _: bool) -> bool {
        panic!("asked when it should not have");
    }

    #[test]
    fn without_a_terminal_every_part_uses_its_default() {
        let c = resolve(&Flags::default(), false, &mut never);
        assert_eq!(
            c,
            Choices {
                hooks: true,
                settings: true,
                path: true,
                aliases: false,
                system_prompt: false
            }
        );
    }

    #[test]
    fn a_flag_wins_and_is_never_asked() {
        let flags = Flags {
            no_hooks: true,
            aliases: true,
            yes: false,
            ..Flags::default()
        };
        let mut asked = Vec::new();
        let c = resolve(&flags, true, &mut |q, d| {
            asked.push(q.to_string());
            d
        });
        assert!(!c.hooks);
        assert!(c.aliases);
        assert_eq!(asked.len(), 3, "{asked:?}");
        assert!(asked
            .iter()
            .all(|q| !q.contains("hooks") && !q.contains("ccc")));
    }

    #[test]
    fn a_terminal_asks_and_the_answer_decides() {
        let c = resolve(&Flags::default(), true, &mut |q, _| q.contains("hooks"));
        assert!(c.hooks);
        assert!(!c.settings && !c.path && !c.aliases && !c.system_prompt);
    }

    #[test]
    fn yes_skips_every_question() {
        let flags = Flags {
            yes: true,
            ..Flags::default()
        };
        let c = resolve(&flags, true, &mut never);
        assert!(c.hooks && c.settings && c.path && !c.aliases && !c.system_prompt);
    }

    #[test]
    fn answers_parse_with_a_default_for_anything_else() {
        assert!(parse_answer("Y\n", false));
        assert!(!parse_answer("no\n", true));
        assert!(parse_answer("\n", true));
        assert!(!parse_answer("maybe", false));
    }
}
