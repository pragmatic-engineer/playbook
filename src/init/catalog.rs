// SPDX-FileCopyrightText: 2026 Igor Santos
// SPDX-License-Identifier: Apache-2.0

//! What `playbook init` is made of, in words a person reads before saying yes.
//! One table of parts lives here. The plan, the questions, `--explain` and
//! the summary after a run all draw from it, so a description is written once.
//! The hooks live in one table too, in `init::wire`.

use crate::init::wire::{wired_hooks, Group, HookSpec, PLUGIN_HOOKS};
use std::fmt::Write as _;

/// One part of `init`.
pub struct Part {
    /// The name used by `--explain <part>`.
    pub id: &'static str,
    /// Whether `init` asks about it. A part without a question runs every time.
    pub asked: bool,
    /// On or off when no flag, no answer and no config says otherwise.
    pub default_on: bool,
    /// The flag pair that decides it, empty when there is none.
    pub flags: &'static str,
    /// The question text. Names the files the part edits. Empty when not asked.
    pub question: &'static str,
    /// One line: what the part does.
    pub does: &'static str,
    /// The files it edits.
    pub changes: &'static str,
    /// How to undo it.
    pub undo: &'static str,
}

const SETTINGS_UNDO: &str =
    "`playbook uninstall` removes what init wrote. Backups of settings.json are saved next to it.";

/// Every part, in the order `init` shows and runs them.
pub const PARTS: &[Part] = &[
    Part {
        id: "hooks",
        asked: true,
        default_on: true,
        flags: "--hooks, --no-hooks",
        question: "Wire the playbook hooks into ~/.claude/settings.json? (see them with `playbook hooks list`)",
        does: "Adds playbook's hooks: safety guards, session helpers, memory and token hygiene checks.",
        changes: "~/.claude/settings.json (the hooks block) and ~/.claude/.settings.base.json",
        undo: SETTINGS_UNDO,
    },
    Part {
        id: "statusline",
        asked: true,
        default_on: true,
        flags: "--statusline, --no-statusline",
        question: "Set the status line in ~/.claude/settings.json to `playbook statusline`?",
        does: "Shows model, git, PR and cost info at the bottom of a session.",
        changes: "~/.claude/settings.json (the statusLine key)",
        undo: SETTINGS_UNDO,
    },
    Part {
        id: "security",
        asked: false,
        default_on: false,
        flags: "--security, --no-security",
        question: "",
        does: "Adds permission rules (allow, ask, deny). Off unless you pass --security or set `security.defaults`. It is never asked about. Update control stays with you, `playbook update` and Homebrew.",
        changes: "~/.claude/settings.json (the permissions block)",
        undo: "Run `playbook init --no-security`, then delete the permissions entries from settings.json by hand. Init never removes a value you have.",
    },
    Part {
        id: "path",
        asked: true,
        default_on: true,
        flags: "--path, --no-path",
        question: "Add the playbook binary's directory to PATH in your shell startup files (~/.zshenv for zsh, ~/.bashrc and a login file for bash)?",
        does: "Puts the playbook binary on PATH so hooks find it.",
        changes: "your shell startup files: ~/.zshenv (zsh), ~/.bashrc and a login file (bash), ~/.config/fish/conf.d/playbook.fish (fish), ~/.profile (other shells)",
        undo: "`playbook uninstall --remove-binary` removes the block, which starts with `# playbook binary`.",
    },
    Part {
        id: "aliases",
        asked: true,
        default_on: false,
        flags: "--aliases, --no-aliases",
        question: "Add one line to your shell rc file (~/.zshrc or ~/.bashrc) that defines the ccc and ccd launcher shortcuts?",
        does: "Defines the ccc and ccd shortcuts, which start Claude Code with the playbook prompt and model chain.",
        changes: "~/.zshrc or ~/.bashrc (one eval line)",
        undo: "`playbook uninstall` removes the line.",
    },
    Part {
        id: "system-prompt",
        asked: true,
        default_on: false,
        flags: "--system-prompt, --no-system-prompt",
        question: "Copy the playbook system prompt to ~/.config/playbook/prompts/SYSTEM_PROMPT.md?",
        does: "Copies the system prompt the ccc and ccd shortcuts use. An existing copy is refreshed on every run, unless you edited it.",
        changes: "~/.config/playbook/prompts/SYSTEM_PROMPT.md",
        undo: "`playbook uninstall` removes the file.",
    },
    Part {
        id: "telemetry",
        asked: true,
        default_on: false,
        flags: "--telemetry, --no-telemetry",
        question: "Write the OpenTelemetry variables for the local usage collector into the env block of ~/.claude/settings.json, so plain claude, the IDE and the desktop app feed `playbook usage`? (nothing is written if you already set telemetry up)",
        does: "Points Claude Code at a collector on 127.0.0.1 (`playbook usage collect`). Prompt text, tool content, emails and tokens are never sent. Written only when you have no telemetry variable in your environment or any Claude Code settings file. Without it, `cc` still does this for its own sessions and `playbook usage` reads transcripts.",
        changes: "~/.claude/settings.json (the env block) and ~/.config/playbook/telemetry-env.json (a record of what was written)",
        undo: "`playbook uninstall` removes exactly the keys init wrote, unless you changed them.",
    },
    Part {
        id: "memory-migration",
        asked: false,
        default_on: true,
        flags: "",
        question: "",
        does: "Moves playbook's old memory folder to its new place, once.",
        changes: "~/.claude/memory moves to ~/.config/playbook/memory",
        undo: "Move the folder back by hand. Claude Code's own memory is never touched.",
    },
    Part {
        id: "trust",
        asked: false,
        default_on: true,
        flags: "",
        question: "",
        does: "Marks ~/.config/playbook as trusted so Claude Code skips its trust dialog there.",
        changes: "~/.claude.json (only when the file exists)",
        undo: "Remove the entry for ~/.config/playbook from ~/.claude.json.",
    },
    Part {
        id: "config-hash",
        asked: false,
        default_on: true,
        flags: "",
        question: "",
        does: "Removes the config-hash script older installs placed. The hash now runs inside playbook.",
        changes: "~/.config/playbook/hooks/lib (removes one file, when present)",
        undo: "Nothing to undo. The script is no longer used.",
    },
];

/// A part by id.
pub fn part(id: &str) -> Option<&'static Part> {
    PARTS.iter().find(|p| p.id == id)
}

/// The question for an asked part. Panics on a bad id, which a test rules out.
pub fn question(id: &str) -> &'static str {
    part(id).map_or("", |p| p.question)
}

/// Every part id, for the flag's value list.
pub fn ids() -> Vec<&'static str> {
    PARTS.iter().map(|p| p.id).collect()
}

/// The short plan printed before the first question.
pub fn plan() -> String {
    let mut out = String::from("playbook init can do the following. You choose each part.\n\n");
    for p in PARTS.iter().filter(|p| p.asked) {
        let default = if p.default_on { "on" } else { "off" };
        let _ = writeln!(
            out,
            "  {}  ({default} by default)\n    {}\n    Edits: {}",
            p.id, p.does, p.changes
        );
    }
    let _ = writeln!(
        out,
        "\nThese run every time and are not asked about: {}.",
        PARTS
            .iter()
            .filter(|p| !p.asked)
            .map(|p| p.id)
            .collect::<Vec<_>>()
            .join(", ")
    );
    out.push_str(
        "See the details with `playbook init --explain`, or the changes for your machine with `playbook init --dry-run`.\n\n",
    );
    out
}

/// The part a `StepReport` name stands for.
fn part_for_step(step: &str) -> &str {
    match step {
        "shim" => "aliases",
        "memory" => "memory-migration",
        other => other,
    }
}

/// What to print after a real run: one line per part that changed, with how to
/// undo it, then where the backups are. Empty when nothing changed.
pub fn summary(changed_steps: &[&str], on_parts: &[&str]) -> String {
    let mut ids: Vec<&str> = Vec::new();
    for step in changed_steps {
        if *step == "settings" {
            // The settings merge carries the status line and the security defaults.
            ids.extend(
                ["statusline", "security"]
                    .iter()
                    .filter(|p| on_parts.contains(p)),
            );
        } else {
            ids.push(part_for_step(step));
        }
    }
    let parts: Vec<&Part> = ids.into_iter().filter_map(part).collect();
    if parts.is_empty() {
        return String::new();
    }
    let mut out = String::from("\nWhat changed and how to undo it:\n");
    for p in parts {
        let _ = writeln!(out, "  {}: edited {}. Undo: {}", p.id, p.changes, p.undo);
    }
    out.push_str(
        "Backups of settings.json are saved next to it as settings.json.bak.<time>. Run `playbook uninstall --dry-run` to preview a full removal.\n",
    );
    out
}

/// What one part is, for `--explain`. The hooks part also lists every hook.
pub fn explain_part(p: &Part) -> String {
    let mut out = format!(
        "{}\n  What it does: {}\n  What it edits: {}\n",
        p.id, p.does, p.changes
    );
    let default = if p.default_on { "on" } else { "off" };
    let _ = writeln!(out, "  Default: {default}");
    if !p.flags.is_empty() {
        let _ = writeln!(out, "  Flags: {}", p.flags);
    }
    let _ = writeln!(out, "  How to undo: {}", p.undo);
    if p.id == "hooks" {
        out.push_str("\n  The hooks it adds:\n");
        for line in hooks_list().lines() {
            let _ = writeln!(out, "  {line}");
        }
    }
    out
}

/// `--explain` with no part, or with one part id.
pub fn explain(which: Option<&str>) -> Result<String, String> {
    match which {
        None => Ok(PARTS
            .iter()
            .map(explain_part)
            .collect::<Vec<_>>()
            .join("\n")),
        Some(id) => part(id)
            .map(explain_part)
            .ok_or_else(|| format!("unknown part `{id}`. Choose one of: {}", ids().join(", "))),
    }
}

fn matcher_label(spec: &HookSpec) -> &'static str {
    spec.matcher.unwrap_or("any")
}

/// Every hook, grouped by purpose: event, matcher, name and one line of purpose.
/// Hooks the plugin registers are marked.
pub fn hooks_list() -> String {
    let mut out = String::new();
    for group in Group::ALL {
        let _ = writeln!(out, "{}", group.label());
        let wired = wired_hooks().filter(|h| h.group == group).map(|h| (h, ""));
        let plugin = PLUGIN_HOOKS
            .iter()
            .filter(|h| h.group == group)
            .map(|h| (h, " (plugin)"));
        for (h, mark) in wired.chain(plugin) {
            let _ = writeln!(
                out,
                "  {} [{}] {}{mark}: {}",
                h.event,
                matcher_label(h),
                h.name,
                h.purpose
            );
        }
        out.push('\n');
    }
    out.push_str("Details: docs/reference/hooks.md in the playbook repo.\n");
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_asked_part_has_a_question_that_names_a_file() {
        for p in PARTS.iter().filter(|p| p.asked) {
            assert!(!p.question.is_empty(), "{} has no question", p.id);
            assert!(
                p.question.contains("~/") || p.question.contains("`playbook"),
                "{} question names no file: {}",
                p.id,
                p.question
            );
        }
    }

    #[test]
    fn a_part_that_is_not_asked_has_no_question() {
        for p in PARTS.iter().filter(|p| !p.asked) {
            assert!(p.question.is_empty(), "{}", p.id);
        }
    }

    #[test]
    fn the_part_ids_are_unique() {
        let mut ids = ids();
        ids.sort_unstable();
        ids.dedup();
        assert_eq!(ids.len(), PARTS.len());
    }

    #[test]
    fn explain_names_every_part_when_none_is_given() {
        let text = explain(None).unwrap();
        for p in PARTS {
            assert!(
                text.contains(&format!("{}\n  What it does", p.id)),
                "{}",
                p.id
            );
        }
    }

    #[test]
    fn explain_of_an_unknown_part_lists_the_known_ones() {
        let err = explain(Some("nope")).unwrap_err();
        assert!(err.contains("hooks") && err.contains("path"), "{err}");
    }

    #[test]
    fn the_hooks_list_names_every_wired_and_plugin_hook() {
        let text = hooks_list();
        for h in wired_hooks().chain(PLUGIN_HOOKS) {
            assert!(
                text.contains(h.name) && text.contains(h.purpose),
                "{}",
                h.name
            );
        }
    }

    #[test]
    fn the_plan_names_every_asked_part_with_the_files_it_edits() {
        let text = plan();
        for p in PARTS.iter().filter(|p| p.asked) {
            assert!(text.contains(p.id) && text.contains(p.does), "{}", p.id);
            assert!(text.contains(&format!("Edits: {}", p.changes)), "{}", p.id);
        }
        assert!(text.contains("--dry-run") && text.contains("--explain"));
    }

    #[test]
    fn the_summary_says_how_to_undo_each_changed_part() {
        let text = summary(&["hooks", "shim"], &[]);
        assert!(
            text.contains("hooks: edited ~/.claude/settings.json"),
            "{text}"
        );
        assert!(
            text.contains("aliases: edited ~/.zshrc or ~/.bashrc"),
            "{text}"
        );
        assert!(text.contains("playbook uninstall"), "{text}");
        assert!(text.contains("settings.json.bak"), "{text}");
    }

    #[test]
    fn the_settings_step_is_reported_as_the_parts_that_use_it() {
        let text = summary(&["settings"], &["statusline", "path"]);
        assert!(text.contains("statusline: edited"), "{text}");
        assert!(!text.contains("security:"), "{text}");
    }

    #[test]
    fn there_is_no_part_for_the_retired_settings_flag() {
        assert!(part("settings").is_none());
    }

    #[test]
    fn the_summary_is_empty_when_nothing_changed() {
        assert_eq!(summary(&[], &[]), "");
        assert_eq!(summary(&["unknown-step"], &[]), "");
        assert_eq!(summary(&["settings"], &[]), "");
    }

    #[test]
    fn no_text_in_the_catalog_uses_a_dash() {
        let all = format!("{}{}{}", plan(), explain(None).unwrap(), hooks_list());
        assert!(!all.contains('\u{2014}') && !all.contains('\u{2013}'));
    }
}
