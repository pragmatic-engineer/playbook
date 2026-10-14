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
    /// Add the `playbook` cd wrapper (follows `playbook cc` into a worktree) to your rc file
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
    /// Answer yes to the telemetry question: write the OpenTelemetry variables for the local usage collector into settings.json. Given alone it means `--only telemetry`
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
    /// Run only these parts (comma separated: hooks, statusline, security, path, aliases, system-prompt, telemetry, claude-override, short-name), ask nothing and leave everything else untouched. `playbook init --only telemetry` turns telemetry on
    #[arg(long, value_name = "PART", value_delimiter = ',', num_args = 1..)]
    pub only: Vec<String>,
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

fn part_pair(flags: &Flags, id: &str) -> Option<(bool, bool)> {
    Some(match id {
        "hooks" => (flags.hooks, flags.no_hooks),
        "statusline" => (flags.statusline, flags.no_statusline),
        "security" => (flags.security, flags.no_security),
        "path" => (flags.path, flags.no_path),
        "aliases" => (flags.aliases, flags.no_aliases),
        "system-prompt" => (flags.system_prompt, flags.no_system_prompt),
        "telemetry" => (flags.telemetry, flags.no_telemetry),
        "claude-override" => (flags.claude_override, flags.no_claude_override),
        "short-name" => (flags.short_name, flags.no_short_name),
        _ => return None,
    })
}

/// The parts `--only` selects. A bare `--telemetry` (no other part flag and
/// no `--yes`) means `--only telemetry`, so it never runs unrelated parts.
pub fn only_parts(flags: &Flags) -> Vec<String> {
    if !flags.only.is_empty() {
        return flags.only.clone();
    }
    let others = catalog::ids()
        .iter()
        .any(|id| *id != "telemetry" && part_pair(flags, id).is_some_and(|(on, off)| on || off));
    if flags.telemetry && !flags.no_telemetry && !others && !flags.yes {
        return vec!["telemetry".to_string()];
    }
    Vec::new()
}

/// The choices for an `--only` run: the named parts on, every other part off.
/// `Ok(None)` when `--only` is not in play.
pub fn only_choices(flags: &Flags) -> Result<Option<Choices>, String> {
    let named = only_parts(flags);
    if named.is_empty() {
        return Ok(None);
    }
    let valid = catalog::ids();
    if let Some(bad) = named.iter().find(|n| !valid.contains(&n.as_str())) {
        return Err(format!(
            "unknown part `{bad}`. Choose one of: {}",
            valid.join(", ")
        ));
    }
    for id in &valid {
        let chosen = named.iter().any(|n| n == id);
        let (on, off) = part_pair(flags, id).unwrap_or((false, false));
        if chosen && off {
            return Err(format!("--only {id} conflicts with --no-{id}"));
        }
        if !chosen && on && !flags.only.is_empty() {
            return Err(format!(
                "--{id} is not in --only {}; add it to --only or drop the flag",
                named.join(",")
            ));
        }
    }
    let has = |id: &str| named.iter().any(|n| n == id);
    Ok(Some(Choices {
        hooks: has("hooks"),
        statusline: has("statusline"),
        security: has("security"),
        path: has("path"),
        aliases: has("aliases"),
        system_prompt: has("system-prompt"),
        telemetry: has("telemetry"),
        claude_override: has("claude-override"),
        short_name: has("short-name"),
    }))
}

/// True when there is no terminal to ask on and the user did not pass `--yes`.
pub fn unattended(flags: &Flags, terminal: bool) -> bool {
    !terminal && !flags.yes
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
    resolve_with(
        flags,
        interactive,
        !unattended(flags, interactive),
        security_config,
        ask,
    )
}

/// Like `resolve`, with the default for the parts that change Claude Code's
/// settings given by the caller. A dry run in a terminal does not ask but
/// still previews the answers a real run would give.
pub fn resolve_with(
    flags: &Flags,
    interactive: bool,
    settings_default: bool,
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
    // Hooks and the status line are entries in Claude Code's settings. With
    // nobody to ask and no --yes, they stay off unless a flag turns them on.
    Choices {
        hooks: decide(
            flags.hooks,
            flags.no_hooks,
            catalog::question("hooks"),
            settings_default,
        ),
        statusline: decide(
            flags.statusline,
            flags.no_statusline,
            catalog::question("statusline"),
            settings_default,
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
            settings_default,
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
        "init: output is not a terminal, so no questions were asked; the hooks and status line \
         stay off and the other defaults apply; pass --yes to accept all defaults or the \
         --x/--no-x flags (see --explain) to choose",
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
    fn with_yes_and_no_terminal_the_settings_parts_are_on() {
        let yes = Flags {
            yes: true,
            ..Flags::default()
        };
        let c = resolve(&yes, false, false, &mut never);
        assert!(c.hooks && c.statusline);
    }

    #[test]
    fn an_explicit_hooks_flag_is_consent_without_a_terminal() {
        let flags = Flags {
            hooks: true,
            ..Flags::default()
        };
        let c = resolve(&flags, false, false, &mut never);
        assert!(c.hooks && !c.statusline);
    }

    #[test]
    fn without_a_terminal_every_part_uses_its_default() {
        let c = resolve(&Flags::default(), false, false, &mut never);
        assert_eq!(
            c,
            Choices {
                hooks: false,
                statusline: false,
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
            .all(|q| !q.contains("hooks") && !q.contains("cd wrapper")));
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

    #[test]
    fn yes_turns_telemetry_on_and_no_terminal_without_yes_leaves_it_off() {
        let yes = Flags {
            yes: true,
            ..Flags::default()
        };

        let with_yes = resolve(&yes, false, false, &mut never);
        let without = resolve(&Flags::default(), false, false, &mut never);

        assert!(with_yes.telemetry);
        assert!(!without.telemetry);
    }

    #[test]
    fn only_runs_the_named_parts_and_nothing_else() {
        let flags = Flags {
            only: vec!["telemetry".into(), "hooks".into()],
            ..Flags::default()
        };

        let c = only_choices(&flags).unwrap().unwrap();

        assert!(c.telemetry && c.hooks);
        assert!(!c.path && !c.statusline && !c.aliases && !c.system_prompt && !c.security);
    }

    #[test]
    fn an_unknown_only_part_lists_the_valid_names() {
        let flags = Flags {
            only: vec!["nope".into()],
            ..Flags::default()
        };

        let err = only_choices(&flags).unwrap_err();

        assert!(
            err.contains("unknown part `nope`") && err.contains("telemetry"),
            "{err}"
        );
    }

    #[test]
    fn only_conflicts_with_the_no_flag_of_a_named_part_and_with_a_flag_for_an_unnamed_one() {
        let named = Flags {
            only: vec!["telemetry".into()],
            no_telemetry: true,
            ..Flags::default()
        };
        let unnamed = Flags {
            only: vec!["telemetry".into()],
            hooks: true,
            ..Flags::default()
        };

        let a = only_choices(&named).unwrap_err();
        let b = only_choices(&unnamed).unwrap_err();

        assert!(a.contains("conflicts with --no-telemetry"), "{a}");
        assert!(b.contains("--hooks is not in --only"), "{b}");
    }

    #[test]
    fn a_bare_telemetry_flag_means_only_telemetry_but_not_beside_other_flags() {
        let bare = Flags {
            telemetry: true,
            ..Flags::default()
        };
        let beside = Flags {
            telemetry: true,
            hooks: true,
            ..Flags::default()
        };
        let with_yes = Flags {
            telemetry: true,
            yes: true,
            ..Flags::default()
        };

        assert_eq!(only_parts(&bare), vec!["telemetry".to_string()]);
        assert!(only_parts(&beside).is_empty());
        assert!(only_parts(&with_yes).is_empty());
    }
}
