// SPDX-FileCopyrightText: 2026 Igor Santos
// SPDX-License-Identifier: Apache-2.0

//! The text of an rm guard denial: each denied target with the rule that
//! matched, the roots that are allowed, and the exact config command that
//! would allow a path outside them.

use super::{resolve_existing, Roots, Rule};
use std::path::{Path, PathBuf};

pub(super) fn denial(denied: &[(String, Rule)], roots: &Roots) -> String {
    let targets = denied
        .iter()
        .map(|(target, rule)| match rule {
            Rule::Outside(dir) if dir.as_os_str() != target.as_str() => format!(
                "`{target}` (rule: {}, resolved to {})",
                rule.name(),
                dir.display()
            ),
            _ => format!("`{target}` (rule: {})", rule.name()),
        })
        .collect::<Vec<_>>()
        .join("; ");
    let mut text = format!(
        "rm blocked: {targets}. Allowed with no config: the repo ({}), ~/.claude/**, playbook's own install files, and the contents of {} (symlinks resolved, never the root itself, and a glob directly under a root needs a named prefix such as /tmp/pb-*).",
        list(&roots.safe, ""),
        scratch_list(roots),
    );
    let home = resolve_existing(Path::new(&roots.home));
    let new_roots: Vec<String> = denied
        .iter()
        .filter_map(|(_, rule)| match rule {
            Rule::Outside(dir) => suggested_root(dir, &home, roots),
            _ => None,
        })
        .collect();
    if let Some(command) = fix_command(&roots.configured, &new_roots) {
        text.push_str(&format!(
            " To allow it, ask the user to run this in the prompt, since you cannot change it: `{command}`. It keeps the current roots and adds the new one, and a root covers everything under it."
        ));
    } else if denied.iter().any(|(_, r)| matches!(r, Rule::Outside(_))) {
        text.push_str(" No config root can cover this path, so ask the user to delete it.");
    }
    let has = |want: fn(&Rule) -> bool| denied.iter().any(|(_, r)| want(r));
    if has(|r| *r == Rule::Protected) {
        text.push_str(" Protected playbook data (the memory store, the databases and the other user data under ~/.config/playbook, or a folder that holds it) stays blocked whatever the roots say. Ask the user if it really has to go.");
    }
    if has(|r| matches!(r, Rule::RootItself | Rule::Glob)) {
        text.push_str(" Name the paths inside the root instead.");
    }
    if has(|r| {
        matches!(
            r,
            Rule::Unresolvable | Rule::StrayCharacter | Rule::Tilde | Rule::LinkLoop
        )
    }) {
        text.push_str(" A command substitution, `<xargs input>`, a variable other than `$HOME`, a relative path in a folder only the shell knows or a stray quote cannot be resolved: run the rm as its own call with a literal absolute path.");
    }
    text.push_str(" For scratch cleanup, create and delete a repo-local temp dir (for example `mktemp -d ./.tmp-XXXX`).");
    text.push_str(hint(denied, roots));
    text
}

/// The scratch roots, plus the two matched by shape: the macOS per-user temp
/// folder and any `tmp` folder under the playbook root.
fn scratch_list(roots: &Roots) -> String {
    let mut items: Vec<String> = roots
        .scratch
        .iter()
        .map(|p| format!("{}/*", p.display()))
        .collect();
    if cfg!(target_os = "macos") {
        items.push("/private/var/folders/*/*/T/*".to_string());
    }
    items.push(format!("{}/**/tmp/*", roots.playbook_root.display()));
    items.join(", ")
}

fn list(paths: &[PathBuf], suffix: &str) -> String {
    if paths.is_empty() {
        return "(none)".to_string();
    }
    paths
        .iter()
        .map(|p| format!("{}{suffix}", p.display()))
        .collect::<Vec<_>>()
        .join(", ")
}

/// The folder that, added as a root, would allow deleting `dir`: its parent,
/// written with `~/`. Only a folder inside home that is not home itself and
/// not under a dot folder is offered, so the message never invites a root
/// such as `/etc`, `~/.ssh` or the playbook data folder.
fn suggested_root(dir: &Path, home: &Path, roots: &Roots) -> Option<String> {
    if roots.home.is_empty() {
        return None;
    }
    let rest = dir.parent()?.strip_prefix(home).ok()?.to_str()?;
    // The config refuses these characters, and the guard drops a root that
    // holds one, so such a folder gets no suggestion.
    let refused = ['*', '?', '[', '{', '$', '`', '\'', '"', '\\'];
    if rest.is_empty() || rest.starts_with('.') || rest.contains(refused) {
        return None;
    }
    Some(format!("~/{rest}"))
}

/// The config command that keeps `current` and adds `new`, or `None` when
/// there is nothing new. Roots never hold a quote, so single quotes are safe
/// in zsh and bash.
fn fix_command(current: &[String], new: &[String]) -> Option<String> {
    let mut all: Vec<String> = current.to_vec();
    let before = all.len();
    for root in new {
        if !all.contains(root) {
            all.push(root.clone());
        }
    }
    if all.len() == before || all.iter().any(|r| r.contains('\'')) {
        return None;
    }
    let json = serde_json::to_string(&all).ok()?;
    Some(format!(
        "! playbook config set --global hooks.rmGuard.allowedRoots '{json}'"
    ))
}

fn hint(denied: &[(String, Rule)], roots: &Roots) -> &'static str {
    let any = |want: &dyn Fn(&str) -> bool| denied.iter().any(|(t, _)| want(t));
    let home_state = Path::new(&roots.home).join(".config/playbook");
    let in_state = |t: &str| {
        let path = Path::new(t);
        path.starts_with(&roots.playbook_root)
            || path.starts_with(&home_state)
            || path.starts_with("~/.config/playbook")
    };
    if denied.iter().any(|(_, r)| *r == Rule::Lock) || any(&|t| t.ends_with(".lock")) {
        " A leftover `.lock` directory clears itself on the next write once it is 10 seconds old, so do not rm it."
    } else if any(&in_state) {
        " This path is playbook's own state: to clear plan scratch files use `playbook plan cleanup <slug>` (or `playbook plan cleanup --stale` for old leftovers), do not rm them."
    } else if any(&|t| t.contains("/.worktrees/")) {
        " This path looks like a worktree leftover: use `git worktree remove --force <path>` for a registered one, `git worktree prune` for a deleted one, and an empty leftover folder is cleared by the next worktree create."
    } else {
        ""
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_fix_command_keeps_the_current_roots_and_adds_the_new_one() {
        let current = vec!["~/scratch/builds".to_string()];

        let command = fix_command(&current, &["/opt/data".to_string()]);

        assert_eq!(
            command.as_deref(),
            Some(
                r#"! playbook config set --global hooks.rmGuard.allowedRoots '["~/scratch/builds","/opt/data"]'"#
            )
        );
    }

    #[test]
    fn nothing_new_gives_no_fix_command() {
        let current = vec!["~/scratch/builds".to_string()];

        assert_eq!(fix_command(&current, &current), None);
    }

    #[test]
    fn a_root_with_a_quote_gives_no_fix_command() {
        let current = vec!["~/it's".to_string()];

        assert_eq!(fix_command(&current, &["~/x/y".to_string()]), None);
    }
}
