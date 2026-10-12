// SPDX-FileCopyrightText: 2026 Igor Santos
// SPDX-License-Identifier: Apache-2.0

//! The optional parts of `playbook init` and how each one gets decided: a flag
//! wins, else a question in a terminal, else the default.

use crate::init::catalog;
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
    /// Use the playbook status line (sets statusLine.command to `playbook statusline`)
    #[arg(long, overrides_with = "no_statusline")]
    pub statusline: bool,
    /// Do not set or change statusLine in settings.json
    #[arg(long, overrides_with = "statusline")]
    pub no_statusline: bool,
    /// Also apply the security defaults (permissions rules); off unless asked or `security.defaults` is true
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
    /// Write the OpenTelemetry variables for the local usage collector into settings.json (only when you have no telemetry set up)
    #[arg(long, overrides_with = "no_telemetry")]
    pub telemetry: bool,
    /// Do not write telemetry variables into settings.json
    #[arg(long, overrides_with = "telemetry")]
    pub no_telemetry: bool,
    /// Define a `claude` shell function that loads the playbook system prompt for a plain session (sets `launcher.claudeOverride` in playbook's config)
    #[arg(long, overrides_with = "no_claude_override")]
    pub claude_override: bool,
    /// Do not turn on the `claude` shell function
    #[arg(long, overrides_with = "claude_override")]
    pub no_claude_override: bool,
    /// Add `pb` as a short name for `playbook`, only when `pb` is free (recorded in playbook own config)
    #[arg(long, overrides_with = "no_short_name")]
    pub short_name: bool,
    /// Do not add the `pb` short name
    #[arg(long, overrides_with = "short_name")]
    pub no_short_name: bool,
    /// Never ask: use the default for every part you did not answer with a flag
    #[arg(long, short = 'y', visible_alias = "defaults")]
    pub yes: bool,
    /// Print what would change for each part and change nothing
    #[arg(long, conflicts_with = "explain")]
    pub dry_run: bool,
    /// Describe a part, or every part when none is named, and change nothing
    #[arg(long, value_name = "PART", num_args = 0..=1, conflicts_with = "dry_run")]
    pub explain: Option<Option<String>>,
}

/// What `init` will do for each optional part.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Choices {
    pub hooks: bool,
    pub statusline: bool,
    pub security: bool,
    pub path: bool,
    pub aliases: bool,
    pub system_prompt: bool,
    pub telemetry: bool,
    pub claude_override: bool,
    pub short_name: bool,
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
            catalog::question("hooks"),
            true,
        ),
        statusline: decide(
            flags.statusline,
            flags.no_statusline,
            catalog::question("statusline"),
            true,
        ),
        security: pick(flags.security, flags.no_security).unwrap_or(security_config),
        path: decide(flags.path, flags.no_path, catalog::question("path"), true),
        aliases: decide(
            flags.aliases,
            flags.no_aliases,
            catalog::question("aliases"),
            false,
        ),
        system_prompt: decide(
            flags.system_prompt,
            flags.no_system_prompt,
            catalog::question("system-prompt"),
            false,
        ),
        telemetry: decide(
            flags.telemetry,
            flags.no_telemetry,
            catalog::question("telemetry"),
            false,
        ),
        claude_override: decide(
            flags.claude_override,
            flags.no_claude_override,
            catalog::question("claude-override"),
            false,
        ),
        short_name: decide(
            flags.short_name,
            flags.no_short_name,
            catalog::question("short-name"),
            false,
        ),
    }
}

/// Whether both ends are a terminal, so a question can be shown and answered.
pub fn stdio_is_interactive() -> bool {
    io::stdin().is_terminal() && io::stdout().is_terminal()
}

/// A line for stderr when a person at a terminal is not asked because output
/// is piped, so the defaults that apply are not a silent surprise. `None`
/// when the run is interactive, scripted with `--yes`, or a dry run.
pub fn defaults_notice(stdin_tty: bool, stdout_tty: bool, flags: &Flags) -> Option<&'static str> {
    let undecided = [
        (flags.hooks, flags.no_hooks),
        (flags.statusline, flags.no_statusline),
        (flags.path, flags.no_path),
        (flags.aliases, flags.no_aliases),
        (flags.system_prompt, flags.no_system_prompt),
        (flags.telemetry, flags.no_telemetry),
    ]
    .iter()
    .any(|&(on, off)| pick(on, off).is_none());
    (undecided && stdin_tty && !stdout_tty && !flags.yes && !flags.dry_run).then_some(
        "init: output is not a terminal, so no questions were asked and the defaults apply; \
         pass --yes to accept them or the --x/--no-x flags (see --explain) to choose",
    )
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
    #[test]
    fn piped_output_at_a_terminal_gets_a_defaults_notice() {
        let flags = Flags::default();

        assert!(defaults_notice(true, false, &flags).is_some());
        assert!(defaults_notice(true, true, &flags).is_none());
        assert!(defaults_notice(false, false, &flags).is_none());
        let yes = Flags {
            yes: true,
            ..Flags::default()
        };
        assert!(defaults_notice(true, false, &yes).is_none());
        let all_set = Flags {
            hooks: true,
            no_statusline: true,
            path: true,
            no_aliases: true,
            system_prompt: true,
            no_telemetry: true,
            ..Flags::default()
        };
        assert!(defaults_notice(true, false, &all_set).is_none());
        let dry = Flags {
            dry_run: true,
            ..Flags::default()
        };
        assert!(defaults_notice(true, false, &dry).is_none());
    }

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
                statusline: true,
                security: false,
                path: true,
                aliases: false,
                system_prompt: false,
                telemetry: false,
                claude_override: false,
                short_name: false
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
        assert_eq!(asked.len(), 6, "{asked:?}");
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
        assert!(!c.statusline && !c.path && !c.aliases && !c.system_prompt);
    }

    #[test]
    fn yes_skips_every_question() {
        let flags = Flags {
            yes: true,
            ..Flags::default()
        };
        let c = resolve(&flags, true, false, &mut never);
        assert!(c.hooks && c.statusline && c.path && !c.aliases && !c.system_prompt);
    }

    #[test]
    fn the_statusline_question_comes_after_hooks_and_before_path() {
        let mut order = Vec::new();
        resolve(&Flags::default(), true, false, &mut |q, d| {
            order.push(q.to_string());
            d
        });
        let pos = |id: &str| {
            order
                .iter()
                .position(|q| q == catalog::question(id))
                .unwrap()
        };
        assert!(pos("hooks") < pos("statusline"));
        assert!(pos("statusline") < pos("path"));
        assert!(order.iter().any(|q| q == catalog::question("statusline")));
    }

    #[test]
    fn the_statusline_flags_decide_without_a_question() {
        let on = Flags {
            statusline: true,
            ..Flags::default()
        };
        let off = Flags {
            no_statusline: true,
            ..Flags::default()
        };
        assert!(
            resolve(&on, true, false, &mut |q, d| {
                assert!(q != catalog::question("statusline"));
                d
            })
            .statusline
        );
        assert!(!resolve(&off, false, false, &mut never).statusline);
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
