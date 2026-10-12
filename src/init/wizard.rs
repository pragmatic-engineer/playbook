// SPDX-FileCopyrightText: 2026 Igor Santos
// SPDX-License-Identifier: Apache-2.0

//! The guided `playbook init` in a terminal: one screen per asked part with
//! what happens on yes and on no, then a summary and a confirm. The screens
//! are built from the part catalog, so a new part shows up here without a
//! change to this file. Nothing is written until the confirm.

use crate::init::catalog::{self, Part};
use crate::init::choices::Flags;
use crate::init::run::{StepReport, StepStatus};
use crate::init::shim::SOURCE_LINE;
use crate::init::wire::{wired_hooks, Group};
use crate::ui::render::{self, Choice, Counts, Status};
use crate::ui::wizard::{Item, MAX_ITEMS};
use crate::ui::Caps;

/// What a flag already decided for a part, by catalog id.
pub fn flag_for(flags: &Flags, id: &str) -> Option<bool> {
    let pair = match id {
        "hooks" => (flags.hooks, flags.no_hooks),
        "statusline" => (flags.statusline, flags.no_statusline),
        "path" => (flags.path, flags.no_path),
        "aliases" => (flags.aliases, flags.no_aliases),
        "system-prompt" => (flags.system_prompt, flags.no_system_prompt),
        "telemetry" => (flags.telemetry, flags.no_telemetry),
        "claude-override" => (flags.claude_override, flags.no_claude_override),
        _ => return None,
    };
    match pair {
        (true, _) => Some(true),
        (_, true) => Some(false),
        _ => None,
    }
}

fn hook_summary() -> String {
    let mut parts = Vec::new();
    for g in Group::ALL {
        let n = wired_hooks().filter(|h| h.group == g).count();
        if n > 0 {
            parts.push(format!("{n} {}", g.label().to_lowercase()));
        }
    }
    parts.join(", ")
}

/// Exact text the part adds, beyond its description.
fn extra_lines(id: &str) -> Vec<String> {
    match id {
        "hooks" => vec![format!(
            "Hooks added: {}. Run `playbook hooks list` to read each one.",
            hook_summary()
        )],
        "statusline" => vec![
            "Sets statusLine.command to `playbook statusline` (model, git branch, PR and cost)."
                .to_string(),
        ],
        "path" => vec![
            "Adds a block that starts with `# playbook binary` and one `export PATH=...` line."
                .to_string(),
        ],
        "aliases" => vec![format!("Adds this line to your rc file: {SOURCE_LINE}")],
        "claude-override" => {
            vec![
                "Runs the equivalent of `playbook config set launcher.claudeOverride true`."
                    .to_string(),
            ]
        }
        _ => Vec::new(),
    }
}

/// What happens when the part is declined.
fn decline_line(id: &str) -> &'static str {
    match id {
        "hooks" => "No hooks are added. The safety guards, session helpers and usage warm-up do not run.",
        "statusline" => "Your status line stays as it is.",
        "path" => "Your shell files are not touched. Hooks may not find playbook unless it is already on PATH.",
        "aliases" => "No line is added to your rc file, so ccc and ccd are not defined.",
        "system-prompt" => "No file is copied. The launchers start without the playbook prompt.",
        "telemetry" => "No variables are written. `playbook usage` reads transcripts instead.",
        "claude-override" => "`claude` stays the plain Claude Code command.",
        _ => "Nothing changes for this part.",
    }
}

fn title_of(p: &Part) -> String {
    let mut chars = p.id.replace('-', " ");
    if let Some(first) = chars.get_mut(0..1) {
        first.make_ascii_uppercase();
    }
    chars
}

/// The screen for one part.
pub fn choice_for(p: &Part) -> Choice {
    let mut accept = vec![p.does.to_string()];
    accept.extend(extra_lines(p.id));
    accept.push(format!("Edits: {}", p.changes));
    accept.push(format!("Undo: {}", p.undo));
    Choice {
        id: p.id.to_string(),
        title: title_of(p),
        explain: p.question.to_string(),
        accept,
        decline: vec![decline_line(p.id).to_string()],
        recommended: p.default_on,
        question: "Do this?".to_string(),
    }
}

/// One item per asked part, in catalog order. `security_on` is whether the
/// security part will run anyway, from its flag or `security.defaults`. It is
/// never asked about, so it appears as a fixed row in the summary.
pub fn items(flags: &Flags, security_on: bool) -> Vec<Item> {
    let mut list: Vec<Item> = catalog::PARTS
        .iter()
        .filter(|p| p.asked)
        .take(MAX_ITEMS)
        .map(|p| Item {
            choice: choice_for(p),
            default: p.default_on,
            fixed: flag_for(flags, p.id),
        })
        .collect();
    if let (true, Some(p)) = (security_on, catalog::part("security")) {
        list.push(Item {
            choice: choice_for(p),
            default: true,
            fixed: Some(true),
        });
    }
    list
}

/// The wizard's answer for the part a question belongs to, else `default`.
pub fn guided_answer(guided: &[(&str, bool)], question: &str, default: bool) -> bool {
    catalog::PARTS
        .iter()
        .find(|p| p.question == question)
        .and_then(|p| guided.iter().find(|(id, _)| *id == p.id))
        .map_or(default, |(_, on)| *on)
}

/// The words after the on/off mark in the summary.
pub fn summary_detail(item: &Item, on: bool) -> String {
    let base = if on {
        "will be applied"
    } else {
        "left as it is"
    };
    if item.fixed.is_some() {
        format!("{base} (set by a flag)")
    } else {
        base.to_string()
    }
}

/// What opens the guided run.
pub fn welcome(caps: &Caps) -> String {
    let mut out = render::heading(caps, "playbook init");
    out.push_str(&render::wrap(
        "This walks through each change playbook can make to your machine. Nothing is written until you confirm at the end. playbook never changes Claude Code's memory, and it edits settings.json only for the parts you accept.",
        caps.width,
        0,
    ));
    out.push_str("\n\nPress Enter to take the answer in capitals. Keys: y, n, b (back), s (default), q (quit).\n");
    out
}

/// The status a step report maps to in the progress phase.
pub fn status_of(step: &StepReport) -> Status {
    match step.status {
        StepStatus::Wired => Status::Done,
        StepStatus::AlreadyCorrect => Status::Unchanged,
        StepStatus::Skipped => Status::Skipped,
        StepStatus::Failed => Status::Failed,
    }
}

/// The progress phase: one line per step, the totals and what to do next.
pub fn progress(caps: &Caps, steps: &[StepReport]) -> String {
    let mut out = render::heading(caps, "Applying");
    let mut statuses = Vec::new();
    for s in steps {
        let st = status_of(s);
        statuses.push(st);
        out.push_str(&render::progress_line(caps, st, s.name, &s.detail));
        out.push('\n');
    }
    let counts = Counts::tally(&statuses);
    let mut next: Vec<&str> = vec![
        "Open a new terminal so PATH and launcher changes load.",
        "Run `playbook doctor` to check the result.",
        "Run `playbook uninstall --dry-run` to see how to undo everything.",
    ];
    if counts.failed > 0 {
        next.insert(
            0,
            "Fix the failed lines above, then run `playbook init` again. It is safe to repeat.",
        );
    }
    out.push_str(&render::next_steps(caps, counts, &next));
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_asked_part_has_a_flag_and_a_screen() {
        for p in catalog::PARTS.iter().filter(|p| p.asked) {
            let flags = Flags::default();
            assert!(flag_for(&flags, p.id).is_none());
            let c = choice_for(p);
            assert!(c.accept.len() >= 3, "{}", p.id);
            assert!(!c.decline[0].is_empty());
        }
        // Each asked part reacts to its own on flag.
        let on = Flags {
            hooks: true,
            statusline: true,
            path: true,
            aliases: true,
            system_prompt: true,
            telemetry: true,
            claude_override: true,
            ..Flags::default()
        };
        for p in catalog::PARTS.iter().filter(|p| p.asked) {
            assert_eq!(flag_for(&on, p.id), Some(true), "{}", p.id);
        }
    }

    #[test]
    fn an_off_flag_fixes_the_item_to_no() {
        let flags = Flags {
            no_aliases: true,
            ..Flags::default()
        };
        let list = items(&flags, false);
        let a = list.iter().find(|i| i.choice.id == "aliases");
        assert_eq!(a.map(|i| i.fixed), Some(Some(false)));
        assert_eq!(
            a.map(|i| summary_detail(i, false)),
            Some("left as it is (set by a flag)".to_string())
        );
    }

    #[test]
    fn the_aliases_screen_shows_the_exact_rc_line() {
        let list = items(&Flags::default(), false);
        let a = list.iter().find(|i| i.choice.id == "aliases");
        let shown = a.map(|i| i.choice.accept.join("\n")).unwrap_or_default();
        assert!(shown.contains("eval \"$(playbook shell-init)\""), "{shown}");
    }

    #[test]
    fn the_hooks_screen_counts_hooks_by_group() {
        let list = items(&Flags::default(), false);
        let h = list.iter().find(|i| i.choice.id == "hooks");
        let shown = h.map(|i| i.choice.accept.join("\n")).unwrap_or_default();
        assert!(shown.contains("safety guards"), "{shown}");
    }

    #[test]
    fn progress_maps_every_status_and_counts() {
        let steps = vec![
            StepReport::wired("hooks", "29 entries added"),
            StepReport::already_correct("path", "already set"),
            StepReport::skipped("aliases", "you chose off"),
            StepReport::failed("trust", "cannot write"),
        ];
        let got = progress(&Caps::plain(80), &steps);
        assert!(got.contains("[ok] hooks  29 entries added\n"));
        assert!(got.contains("[=] path  already set\n"));
        assert!(got.contains("[skip] aliases  you chose off\n"));
        assert!(got.contains("[fail] trust  cannot write\n"));
        assert!(got.contains("1 done, 1 unchanged, 1 skipped, 1 failed"));
        assert!(got.contains("Fix the failed lines"));
    }

    #[test]
    fn security_shows_as_a_fixed_row_only_when_it_will_run() {
        let flags = Flags::default();
        assert!(!items(&flags, false)
            .iter()
            .any(|i| i.choice.id == "security"));
        let on = items(&flags, true);
        let sec = on.iter().find(|i| i.choice.id == "security");
        assert_eq!(sec.map(|i| i.fixed), Some(Some(true)));
    }

    #[test]
    fn every_asked_part_flips_when_answered_against_its_default() {
        use crate::init::choices::resolve;
        let guided: Vec<(&str, bool)> = catalog::PARTS
            .iter()
            .filter(|p| p.asked)
            .map(|p| (p.id, !p.default_on))
            .collect();
        let c = resolve(&Flags::default(), true, false, &mut |q, d| {
            guided_answer(&guided, q, d)
        });
        assert!(!c.hooks && !c.statusline && !c.path);
        assert!(c.aliases && c.system_prompt && c.telemetry && c.claude_override);
    }

    #[test]
    fn the_welcome_promises_no_change_before_confirm() {
        let got = welcome(&Caps::plain(80))
            .split_whitespace()
            .collect::<Vec<_>>()
            .join(" ");
        assert!(got.contains("Nothing is written until you confirm"));
        assert!(got.contains("never changes Claude Code's memory"));
    }
}
