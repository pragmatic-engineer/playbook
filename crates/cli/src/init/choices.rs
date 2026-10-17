// SPDX-FileCopyrightText: 2026 Igor Santos
// SPDX-License-Identifier: Apache-2.0

//! The optional parts of `playbook setup` and how each one gets decided: a flag
//! wins, else a question in a terminal, else the default.

use crate::init::{attribution, catalog, collector_service, pb_link, system_prompt};
use clap::Args;
use std::io::{self, IsTerminal};

/// The flags, each part as a `--x` and `--no-x` pair. The last one given wins.
#[derive(Args, Debug, Default, Clone, PartialEq, Eq)]
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
    /// Retired: setup no longer writes a shell rc line (ADR 0044). Accepted and ignored until 1.0
    #[arg(long, hide = true, overrides_with = "no_aliases")]
    pub aliases: bool,
    /// Retired with `--aliases`
    #[arg(long, hide = true, overrides_with = "aliases")]
    pub no_aliases: bool,
    /// Install the playbook system prompt (an existing copy is refreshed either way)
    #[arg(long, overrides_with = "no_system_prompt")]
    pub system_prompt: bool,
    /// Do not install the system prompt, and store `systemPrompt.enabled` false so `pb cc` starts with Claude Code's own
    #[arg(long, overrides_with = "system_prompt")]
    pub no_system_prompt: bool,
    /// Answer yes to the telemetry question: write the OpenTelemetry variables for the local usage collector into settings.json. Given alone it means `--only telemetry`
    #[arg(long, overrides_with = "no_telemetry")]
    pub telemetry: bool,
    /// Do not write telemetry variables into settings.json
    #[arg(long, overrides_with = "telemetry")]
    pub no_telemetry: bool,
    /// Keep the usage collector running in a per-user background service (launchd agent or systemd user unit), when telemetry points at it
    #[arg(long, overrides_with = "no_collector_service")]
    pub collector_service: bool,
    /// Do not install the background service for the usage collector
    #[arg(long, overrides_with = "collector_service")]
    pub no_collector_service: bool,
    /// Retired: setup no longer defines a `claude` shell function (ADR 0044). Accepted and ignored until 1.0
    #[arg(long, hide = true, overrides_with = "no_claude_override")]
    pub claude_override: bool,
    /// Retired with `--claude-override`
    #[arg(long, hide = true, overrides_with = "claude_override")]
    pub no_claude_override: bool,
    /// Add `pb` as a short name for `playbook`: a `pb -> playbook` symlink in the install folder, only when `pb` is free
    #[arg(long, overrides_with = "no_short_name")]
    pub short_name: bool,
    /// Do not add the `pb` short name
    #[arg(long, overrides_with = "short_name")]
    pub no_short_name: bool,
    /// Turn off the commit, PR and session link attribution Claude Code adds (sets `attribution` in settings.json)
    #[arg(long, overrides_with = "no_attribution_off")]
    pub attribution_off: bool,
    /// Do not set or change attribution in settings.json
    #[arg(long, overrides_with = "attribution_off")]
    pub no_attribution_off: bool,
    /// Run only these parts (comma separated: hooks, statusline, security, path, system-prompt, telemetry, collector-service, short-name, attribution-off), ask nothing and leave everything else untouched. `playbook setup --only telemetry` turns telemetry on
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
    pub system_prompt: bool,
    pub telemetry: bool,
    pub collector_service: bool,
    pub short_name: bool,
    pub attribution_off: bool,
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

pub(crate) fn part_pair(flags: &Flags, id: &str) -> Option<(bool, bool)> {
    Some(match id {
        "hooks" => (flags.hooks, flags.no_hooks),
        "statusline" => (flags.statusline, flags.no_statusline),
        "security" => (flags.security, flags.no_security),
        "path" => (flags.path, flags.no_path),
        "system-prompt" => (flags.system_prompt, flags.no_system_prompt),
        "telemetry" => (flags.telemetry, flags.no_telemetry),
        "collector-service" => (flags.collector_service, flags.no_collector_service),
        "short-name" => (flags.short_name, flags.no_short_name),
        "attribution-off" => (flags.attribution_off, flags.no_attribution_off),
        _ => return None,
    })
}

/// The part names setup took before ADR 0044 and now ignores.
pub const RETIRED_PARTS: [&str; 2] = ["aliases", "claude-override"];

/// One stderr line per retired flag or `--only` name in use, for a terminal.
pub fn retired_notes(flags: &Flags) -> Vec<String> {
    let set_by_flag = [
        flags.aliases || flags.no_aliases,
        flags.claude_override || flags.no_claude_override,
    ];
    RETIRED_PARTS
        .iter()
        .zip(set_by_flag)
        .filter(|(id, flag)| *flag || flags.only.iter().any(|n| n == *id))
        .map(|(id, _)| {
            format!(
                "playbook setup: `{id}` is retired and does nothing; `playbook cc` and `pb cc` need no shell line (ADR 0044)"
            )
        })
        .collect()
}

/// Whether `--only` names only retired parts, so setup has nothing to run.
pub fn only_retired(flags: &Flags) -> bool {
    !flags.only.is_empty()
        && flags
            .only
            .iter()
            .all(|n| RETIRED_PARTS.contains(&n.as_str()))
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
    let named: Vec<String> = only_parts(flags)
        .into_iter()
        .filter(|n| !RETIRED_PARTS.contains(&n.as_str()))
        .collect();
    if named.is_empty() && flags.only.is_empty() {
        return Ok(None);
    }
    let valid: Vec<&'static str> = catalog::ids()
        .into_iter()
        .filter(|id| part_pair(flags, id).is_some())
        .collect();
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
                flags.only.join(",")
            ));
        }
    }
    let has = |id: &str| named.iter().any(|n| n == id);
    Ok(Some(Choices {
        hooks: has("hooks"),
        statusline: has("statusline"),
        security: has("security"),
        path: has("path"),
        system_prompt: has("system-prompt"),
        telemetry: has("telemetry"),
        collector_service: has("collector-service"),
        short_name: has("short-name"),
        attribution_off: has("attribution-off"),
    }))
}

/// Whether a lone off switch the user set may be replaced: an answer to the
/// question (a run with no `--yes`), `--only`, or an explicit `--telemetry`.
/// `--yes` alone never replaces it.
pub fn replace_off_ok(flags: &Flags, telemetry: bool, only_run: bool) -> bool {
    telemetry && (flags.telemetry || only_run || !flags.yes)
}

/// What the attribution part does. `on` is the resolved choice and `asked`
/// whether the user answered the question. A yes or `--attribution-off` may
/// replace a value the user set; `--yes` and `--only` only add missing keys.
/// A no or `--no-attribution-off` is recorded, a run that asked nothing is not.
pub fn attribution_choice(flags: &Flags, on: bool, asked: bool) -> attribution::Choice {
    match (
        on,
        flags.attribution_off || asked,
        flags.no_attribution_off || asked,
    ) {
        (true, true, _) => attribution::Choice::Replace,
        (true, false, _) => attribution::Choice::AddMissing,
        (false, _, true) => attribution::Choice::Declined,
        (false, _, false) => attribution::Choice::Skip,
    }
}

/// What the collector-service part does. A no or `--no-collector-service`
/// is recorded; a run that asked nothing is not.
pub fn collector_service_choice(flags: &Flags, on: bool, asked: bool) -> collector_service::Choice {
    match (on, flags.no_collector_service || asked) {
        (true, _) => collector_service::Choice::Install,
        (false, true) => collector_service::Choice::Declined,
        (false, false) => collector_service::Choice::Skip,
    }
}

/// What the short-name part does. A no or `--no-short-name` is recorded; a
/// run that asked nothing is not. A stored no (`declined`) wins over a default
/// that no question or flag confirmed, as under `--yes`.
pub fn short_name_choice(flags: &Flags, on: bool, asked: bool, declined: bool) -> pb_link::Choice {
    match (on, flags.no_short_name || asked) {
        (true, _) if declined && !asked && !flags.short_name => pb_link::Choice::Skip,
        (true, _) => pb_link::Choice::Link,
        (false, true) => pb_link::Choice::Declined,
        (false, false) => pb_link::Choice::Skip,
    }
}

/// The system prompt answer setup stores: a yes, an explicit no from the
/// flag or the wizard, or nothing asked.
pub fn system_prompt_answer(flags: &Flags, on: bool, asked: bool) -> system_prompt::Answer {
    match (on, flags.no_system_prompt || asked) {
        (true, _) => system_prompt::Answer::Yes,
        (false, true) => system_prompt::Answer::No,
        (false, false) => system_prompt::Answer::Unasked,
    }
}

/// Whether an earlier no or an install without the prompt may keep it off:
/// a run that asks nothing and took the default (ADR 0044 decision 5).
pub fn prompt_may_be_held_off(flags: &Flags, on: bool, asked: bool, only_run: bool) -> bool {
    on && !asked && !only_run && !flags.system_prompt
}

/// What an unattended run found about the parts it leaves off.
pub struct LeftOff {
    /// Setup already printed why telemetry is not asked about.
    pub telemetry_skipped: bool,
    /// `pb cc` would start without the playbook prompt.
    pub prompt_missing: bool,
    /// No `pb` command is on PATH.
    pub pb_missing: bool,
}

/// The stderr lines for a run with no terminal and no `--yes`, one per
/// part it left off that a flag did not decide.
pub fn unattended_lines(flags: &Flags, choices: &Choices, left: &LeftOff) -> Vec<String> {
    let undecided = |on: bool, yes: bool, no: bool| !on && !yes && !no;
    let mut lines = Vec::new();
    let only = |what: &str, part: &str| {
        format!("setup: no terminal and no --yes, so {what}. To add it, run: playbook setup --only {part}")
    };
    if undecided(choices.telemetry, flags.telemetry, flags.no_telemetry) && !left.telemetry_skipped
    {
        lines.push(only("telemetry was not turned on", "telemetry"));
    }
    if undecided(
        choices.system_prompt,
        flags.system_prompt,
        flags.no_system_prompt,
    ) && left.prompt_missing
    {
        lines.push(only("the system prompt was not added", "system-prompt"));
    }
    if undecided(choices.short_name, flags.short_name, flags.no_short_name) && left.pb_missing {
        lines.push(only("the pb link was not added", "short-name"));
    }
    let left_off: Vec<&str> = [
        (
            "hooks",
            undecided(choices.hooks, flags.hooks, flags.no_hooks),
        ),
        (
            "status line",
            undecided(choices.statusline, flags.statusline, flags.no_statusline),
        ),
        (
            "attribution-off",
            undecided(
                choices.attribution_off,
                flags.attribution_off,
                flags.no_attribution_off,
            ),
        ),
    ]
    .into_iter()
    .filter_map(|(name, off)| off.then_some(name))
    .collect();
    if let Some((last, rest)) = left_off.split_last() {
        let list = if rest.is_empty() {
            (*last).to_string()
        } else {
            format!("{} and {last}", rest.join(", "))
        };
        lines.push(format!(
            "setup: no terminal and no --yes, so these were left out of your Claude Code \
             settings: {list}. To add them, run: playbook setup --yes"
        ));
    }
    lines
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
        system_prompt: decide(
            flags.system_prompt,
            flags.no_system_prompt,
            catalog::question("system-prompt"),
            settings_default,
        ),
        telemetry: decide(
            flags.telemetry,
            flags.no_telemetry,
            catalog::question("telemetry"),
            settings_default,
        ),
        collector_service: decide(
            flags.collector_service,
            flags.no_collector_service,
            catalog::question("collector-service"),
            settings_default,
        ),
        short_name: decide(
            flags.short_name,
            flags.no_short_name,
            catalog::question("short-name"),
            settings_default,
        ),
        attribution_off: decide(
            flags.attribution_off,
            flags.no_attribution_off,
            catalog::question("attribution-off"),
            settings_default,
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
        (flags.system_prompt, flags.no_system_prompt),
        (flags.telemetry, flags.no_telemetry),
        (flags.collector_service, flags.no_collector_service),
        (flags.short_name, flags.no_short_name),
        (flags.attribution_off, flags.no_attribution_off),
    ]
    .iter()
    .any(|&(on, off)| pick(on, off).is_none());
    (undecided && stdin_tty && !stdout_tty && !flags.yes && !flags.dry_run).then_some(
        "setup: output is not a terminal, so no questions were asked; the hooks, status line, \
         system-prompt, telemetry, collector-service, short-name and attribution-off parts stay off and the other defaults apply; pass --yes to accept all defaults or the \
         --x/--no-x flags (see --explain) to choose",
    )
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
            system_prompt: true,
            no_telemetry: true,
            no_collector_service: true,
            no_short_name: true,
            no_attribution_off: true,
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
                system_prompt: false,
                telemetry: false,
                collector_service: false,
                short_name: false,
                attribution_off: false
            }
        );
    }

    #[test]
    fn a_flag_wins_and_is_never_asked() {
        let flags = Flags {
            no_hooks: true,
            path: true,
            yes: false,
            ..Flags::default()
        };
        let mut asked = Vec::new();
        let c = resolve(&flags, true, false, &mut |q, d| {
            asked.push(q.to_string());
            d
        });
        assert!(!c.hooks);
        assert!(c.path);
        assert_eq!(asked.len(), 6, "{asked:?}");
        assert!(asked
            .iter()
            .all(|q| !q.contains("hooks") && q != catalog::question("path")));
    }

    #[test]
    fn a_collector_service_no_is_recorded_only_when_someone_said_it() {
        use collector_service::Choice::*;
        let off = Flags {
            no_collector_service: true,
            ..Flags::default()
        };
        assert_eq!(
            collector_service_choice(&Flags::default(), true, false),
            Install
        );
        assert_eq!(
            collector_service_choice(&Flags::default(), false, true),
            Declined
        );
        assert_eq!(collector_service_choice(&off, false, false), Declined);
        assert_eq!(
            collector_service_choice(&Flags::default(), false, false),
            Skip
        );
    }

    #[test]
    fn a_short_name_no_is_recorded_only_when_said_and_a_stored_no_wins_over_a_default() {
        use pb_link::Choice::*;
        let flags = |on, off| Flags {
            short_name: on,
            no_short_name: off,
            ..Flags::default()
        };
        // (--short-name, --no-short-name, on, asked, stored no) -> choice
        for (on_flag, off_flag, on, asked, declined, want) in [
            (false, false, true, false, false, Link),
            (false, false, true, false, true, Skip),
            (true, false, true, false, true, Link),
            (false, false, true, true, true, Link),
            (false, false, false, true, false, Declined),
            (false, true, false, false, false, Declined),
            (false, false, false, false, false, Skip),
        ] {
            let got = short_name_choice(&flags(on_flag, off_flag), on, asked, declined);
            assert_eq!(got, want, "{on_flag} {off_flag} {on} {asked} {declined}");
        }
    }

    #[test]
    fn yes_turns_attribution_off_but_replaces_a_user_value_only_with_the_flag() {
        use attribution::Choice::*;
        let flags = |yes, on, off| Flags {
            yes,
            attribution_off: on,
            no_attribution_off: off,
            ..Flags::default()
        };
        assert!(resolve(&flags(true, false, false), false, false, &mut never).attribution_off);
        assert!(!resolve(&Flags::default(), false, false, &mut never).attribution_off);
        // (yes, --attribution-off, --no-attribution-off, on, asked) -> choice
        for (yes, on_flag, off_flag, on, asked, want) in [
            (true, false, false, true, false, AddMissing),
            (true, true, false, true, false, Replace),
            (false, false, false, true, true, Replace),
            (false, false, false, false, true, Declined),
            (false, false, true, false, false, Declined),
            (false, false, false, false, false, Skip),
        ] {
            let got = attribution_choice(&flags(yes, on_flag, off_flag), on, asked);
            assert_eq!(got, want, "{yes} {on_flag} {off_flag} {on} {asked}");
        }
    }

    #[test]
    fn a_terminal_asks_and_the_answer_decides() {
        let c = resolve(&Flags::default(), true, false, &mut |q, _| {
            q.contains("hooks")
        });
        assert!(c.hooks);
        assert!(!c.statusline && !c.path && !c.system_prompt);
    }

    #[test]
    fn yes_skips_every_question_and_turns_every_asked_part_on() {
        let flags = Flags {
            yes: true,
            ..Flags::default()
        };
        let c = resolve(&flags, true, false, &mut never);
        assert_eq!(
            c,
            Choices {
                hooks: true,
                statusline: true,
                security: false,
                path: true,
                system_prompt: true,
                telemetry: true,
                collector_service: true,
                short_name: true,
                attribution_off: true
            }
        );
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

        assert_eq!(
            c,
            Choices {
                hooks: true,
                statusline: false,
                security: false,
                path: false,
                system_prompt: false,
                telemetry: true,
                collector_service: false,
                short_name: false,
                attribution_off: false
            }
        );
    }

    #[test]
    fn the_retired_parts_are_accepted_ignored_and_noted() {
        let flags = Flags {
            only: vec!["aliases".into()],
            claude_override: true,
            ..Flags::default()
        };

        let c = only_choices(&flags).unwrap().unwrap();
        let notes = retired_notes(&flags);

        assert!(!c.hooks && !c.path && !c.system_prompt && !c.short_name);
        assert_eq!(notes.len(), 2, "{notes:?}");
        assert!(notes[0].contains("`aliases` is retired") && notes[0].contains("ADR 0044"));
        assert!(notes[1].contains("`claude-override` is retired"));
        assert!(retired_notes(&Flags::default()).is_empty());
        let no_flag = Flags {
            no_aliases: true,
            ..Flags::default()
        };
        assert_eq!(retired_notes(&no_flag).len(), 1);
        assert!(resolve(&no_flag, false, false, &mut never).path);
        assert!(only_retired(&flags));
        assert!(!only_retired(&Flags {
            only: vec!["aliases".into(), "hooks".into()],
            ..Flags::default()
        }));
    }

    #[test]
    fn a_part_flag_outside_a_retired_only_names_what_was_typed() {
        let flags = Flags {
            only: vec!["aliases".into()],
            hooks: true,
            ..Flags::default()
        };

        let err = only_choices(&flags).unwrap_err();

        assert!(err.contains("--hooks is not in --only aliases;"), "{err}");
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

    #[test]
    fn replacing_an_off_switch_needs_an_answer_an_explicit_flag_or_only_but_not_yes_alone() {
        let plain = Flags::default();
        let yes = Flags {
            yes: true,
            ..Flags::default()
        };
        let yes_flag = Flags {
            yes: true,
            telemetry: true,
            ..Flags::default()
        };

        assert!(replace_off_ok(&plain, true, false));
        assert!(!replace_off_ok(&yes, true, false));
        assert!(replace_off_ok(&yes_flag, true, false));
        assert!(replace_off_ok(&yes, true, true));
        assert!(!replace_off_ok(&plain, false, false));
    }

    fn all_off() -> Choices {
        Choices {
            hooks: false,
            statusline: false,
            security: false,
            path: false,
            system_prompt: false,
            telemetry: false,
            collector_service: false,
            short_name: false,
            attribution_off: false,
        }
    }

    #[test]
    fn the_prompt_answer_is_yes_when_on_no_when_declined_and_unasked_otherwise() {
        let plain = Flags::default();
        let declined = Flags {
            no_system_prompt: true,
            ..Flags::default()
        };

        assert_eq!(
            system_prompt_answer(&plain, true, false),
            system_prompt::Answer::Yes
        );
        assert_eq!(
            system_prompt_answer(&declined, false, false),
            system_prompt::Answer::No
        );
        assert_eq!(
            system_prompt_answer(&plain, false, true),
            system_prompt::Answer::No
        );
        assert_eq!(
            system_prompt_answer(&plain, false, false),
            system_prompt::Answer::Unasked
        );
    }

    #[test]
    fn an_unattended_run_names_each_undecided_part_it_left_off() {
        let left = LeftOff {
            telemetry_skipped: false,
            prompt_missing: true,
            pb_missing: true,
        };

        let lines = unattended_lines(&Flags::default(), &all_off(), &left);

        assert_eq!(lines.len(), 4, "{lines:?}");
        assert!(lines[0].contains("--only telemetry"), "{lines:?}");
        assert!(lines[1].contains("--only system-prompt"), "{lines:?}");
        assert!(lines[2].contains("--only short-name"), "{lines:?}");
        assert!(
            lines[3].contains("hooks, status line and attribution-off"),
            "{lines:?}"
        );
    }

    #[test]
    fn an_unattended_run_stays_quiet_about_parts_a_flag_or_the_disk_decided() {
        let flags = Flags {
            no_telemetry: true,
            no_hooks: true,
            no_statusline: true,
            no_attribution_off: true,
            ..Flags::default()
        };
        let left = LeftOff {
            telemetry_skipped: false,
            prompt_missing: false,
            pb_missing: false,
        };

        assert!(unattended_lines(&flags, &all_off(), &left).is_empty());
    }
}
