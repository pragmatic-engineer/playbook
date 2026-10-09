// SPDX-FileCopyrightText: 2026 Igor Santos
// SPDX-License-Identifier: Apache-2.0

//! The optional parts of `playbook init` and how each one gets decided: a flag
//! wins, else a question in a terminal, else the default.

use clap::Args;
use std::io::{self, BufRead, IsTerminal, Write};

/// The flags, each part as a `--x` and `--no-x` pair. The last one given wins.
#[derive(Args, Debug, Default, Clone)]
pub struct Flags {
    /// Wire every playbook hook into settings.json (guards, session and worktree hooks)
    #[arg(long, overrides_with = "no_hooks")]
    pub hooks: bool,
    /// Add no playbook hook entries, including the guards in the shared settings
    #[arg(long, overrides_with = "hooks")]
    pub no_hooks: bool,
    /// Merge playbook's shared settings (status line, options) into settings.json
    #[arg(long, overrides_with = "no_settings")]
    pub settings: bool,
    /// Do not touch the shared settings
    #[arg(long, overrides_with = "settings")]
    pub no_settings: bool,
    /// Also apply the security defaults (permissions rules, autoupdater off); off unless asked or `security.defaults` is true
    #[arg(long, overrides_with = "no_security")]
    pub security: bool,
    /// Do not apply the security defaults, even when `security.defaults` is true
    #[arg(long, overrides_with = "security")]
    pub no_security: bool,
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
    pub security: bool,
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
/// no flag, and only when `interactive` is true. `security_config` is the
/// resolved `security.defaults` key: the default for the security part, which
/// is never asked about, so a plain run stays off unless that key says on.
pub fn resolve(
    flags: &Flags,
    interactive: bool,
    security_config: bool,
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
            "Wire every playbook hook into ~/.claude/settings.json? (safety guards, session and worktree hooks)",
            true,
        ),
        settings: decide(
            flags.settings,
            flags.no_settings,
            "Merge playbook's shared settings (status line, options) into settings.json? Hooks are a separate question.",
            true,
        ),
        security: pick(flags.security, flags.no_security).unwrap_or(security_config),
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
        let c = resolve(&Flags::default(), false, false, &mut never);
        assert_eq!(
            c,
            Choices {
                hooks: true,
                settings: true,
                security: false,
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
        let c = resolve(&flags, true, false, &mut |q, d| {
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
        let c = resolve(&Flags::default(), true, false, &mut |q, _| {
            q.contains("hooks")
        });
        assert!(c.hooks);
        assert!(!c.settings && !c.path && !c.aliases && !c.system_prompt);
    }

    #[test]
    fn yes_skips_every_question() {
        let flags = Flags {
            yes: true,
            ..Flags::default()
        };
        let c = resolve(&flags, true, false, &mut never);
        assert!(c.hooks && c.settings && c.path && !c.aliases && !c.system_prompt);
    }

    #[test]
    fn security_is_off_unless_a_flag_or_the_config_key_turns_it_on() {
        let none = Flags::default();
        assert!(!resolve(&none, false, false, &mut never).security);
        assert!(resolve(&none, false, true, &mut never).security);
        let on = Flags {
            security: true,
            ..Flags::default()
        };
        assert!(resolve(&on, false, false, &mut never).security);
        let off = Flags {
            no_security: true,
            ..Flags::default()
        };
        assert!(!resolve(&off, false, true, &mut never).security);
    }

    #[test]
    fn answers_parse_with_a_default_for_anything_else() {
        assert!(parse_answer("Y\n", false));
        assert!(!parse_answer("no\n", true));
        assert!(parse_answer("\n", true));
        assert!(!parse_answer("maybe", false));
    }
}
