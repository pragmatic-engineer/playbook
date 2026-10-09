// SPDX-FileCopyrightText: 2026 Igor Santos
// SPDX-License-Identifier: Apache-2.0

//! Per-component effort ceilings (ADR-0017). A component is an agent, a
//! command or a skill, and its ceiling comes from `effort.<kind>.<name>`.
//!
//! The ceiling for a component is the lowest of Claude Code's own
//! `maxEffortLevel`, playbook's `maxEffortLevel` and the component's key. The
//! effective effort is the lower of the shipped effort and that ceiling, so a
//! ceiling never raises anything. For an agent, `resolve` also names the file
//! to dispatch, because an agent's effort is fixed by its file.

use super::{lower, rank};
use crate::agents::variants::{variant_name, MARKER, VARIANTS};
use crate::config;
use serde_json::{json, Value};
use std::fs;
use std::path::Path;

/// What a component is.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    Agent,
    Command,
    Skill,
}

impl Kind {
    /// Parse `agents`, `commands` or `skills` (singular forms work too).
    pub fn parse(text: &str) -> Option<Kind> {
        match text {
            "agent" | "agents" => Some(Kind::Agent),
            "command" | "commands" => Some(Kind::Command),
            "skill" | "skills" => Some(Kind::Skill),
            _ => None,
        }
    }

    /// The plural word used in config keys and plugin directories.
    pub fn plural(self) -> &'static str {
        match self {
            Kind::Agent => "agents",
            Kind::Command => "commands",
            Kind::Skill => "skills",
        }
    }

    /// The component's source file under the plugin root.
    fn file(self, root: &Path, name: &str) -> std::path::PathBuf {
        match self {
            Kind::Agent => root.join("agents").join(format!("{name}.md")),
            Kind::Command => root.join("commands").join(format!("{name}.md")),
            Kind::Skill => root.join("skills").join(name).join("SKILL.md"),
        }
    }
}

/// The config key that holds a component's ceiling.
pub fn config_key(kind: Kind, name: &str) -> String {
    format!("effort.{}.{name}", kind.plural())
}

/// The `effort:` value in a file's frontmatter, if any.
pub fn shipped_effort(path: &Path) -> Option<String> {
    let text = fs::read_to_string(path).ok()?;
    let mut lines = text.lines();
    if lines.next()? != "---" {
        return None;
    }
    for line in lines {
        if line == "---" {
            break;
        }
        if let Some(rest) = line.strip_prefix("effort:") {
            let value = rest.trim().trim_matches(['"', '\'']);
            return (!value.is_empty()).then(|| value.to_string());
        }
    }
    None
}

/// The variants this session can dispatch, named by the launcher in this
/// variable (comma separated). A session started without the launcher has none.
pub const VARIANTS_ENV: &str = "PLAYBOOK_AGENT_VARIANTS";

/// The variant names in `VARIANTS_ENV`.
pub fn available_variants() -> Vec<String> {
    std::env::var(VARIANTS_ENV)
        .unwrap_or_default()
        .split(',')
        .filter(|n| !n.is_empty())
        .map(str::to_string)
        .collect()
}

/// Everything known about one component's effort.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Resolution {
    pub kind: Kind,
    pub name: String,
    /// Whether the component's source file exists under the plugin root.
    pub known: bool,
    pub shipped: Option<String>,
    /// The component's own key, `auto` when unset.
    pub configured: String,
    /// playbook's `maxEffortLevel`.
    pub global: String,
    pub claude: Option<String>,
    /// The lowest ceiling in play, `None` when nothing limits.
    pub ceiling: Option<String>,
    /// Shipped effort lowered by the ceiling.
    pub effective: Option<String>,
    /// For an agent: the name to dispatch by default.
    pub file: Option<String>,
    /// For an agent: the `subagent_type` to pass the `Agent` tool for `file`.
    pub subagent_type: Option<String>,
    /// For an agent: every name at or below the ceiling, lowest first.
    pub allowed: Vec<String>,
    /// False when the ceiling is below every agent the session has, so the
    /// lowest one is returned and still exceeds it.
    pub satisfied: bool,
}

fn level_of(home: &Path, key: &str) -> String {
    match config::resolve_valid(key, home, None) {
        Ok((Value::String(level), _, _)) => level,
        _ => "auto".to_string(),
    }
}

/// `auto` and `max` set no ceiling.
fn as_ceiling(level: &str) -> Option<&str> {
    (level != "auto" && level != "max").then_some(level)
}

/// The agents with their efforts, lowest first: the base and each variant
/// the session has (`available`, from the launcher).
fn agent_files(name: &str, shipped: &str, available: &[String]) -> Vec<(String, String)> {
    let mut files = vec![(name.to_string(), shipped.to_string())];
    if VARIANTS.iter().any(|(base, _)| *base == name) {
        for tier in crate::agents::variants::TIERS {
            let variant = variant_name(name, tier);
            if tier != shipped && available.contains(&variant) {
                files.push((variant, tier.to_string()));
            }
        }
    }
    files.sort_by_key(|(_, effort)| rank(effort).unwrap_or(usize::MAX));
    files
}

/// Resolve one component. `root` is the plugin root holding `agents/`,
/// `commands/` and `skills/`; `None` leaves `known` false and `shipped` empty.
pub fn resolve(
    kind: Kind,
    name: &str,
    home: &Path,
    claude: Option<&str>,
    root: Option<&Path>,
) -> Resolution {
    resolve_with(kind, name, home, claude, root, &available_variants())
}

/// `resolve` with the session's variants given instead of read from the
/// environment.
pub fn resolve_with(
    kind: Kind,
    name: &str,
    home: &Path,
    claude: Option<&str>,
    root: Option<&Path>,
    available: &[String],
) -> Resolution {
    let path = root.map(|r| kind.file(r, name));
    let known = path.as_deref().is_some_and(Path::is_file);
    let shipped = path.as_deref().and_then(shipped_effort);
    let configured = level_of(home, &config_key(kind, name));
    let global = level_of(home, super::KEY);

    let ceiling =
        lower(lower(claude, as_ceiling(&global)), as_ceiling(&configured)).map(str::to_string);
    let effective = match (shipped.as_deref(), ceiling.as_deref()) {
        (Some(s), Some(c)) => lower(Some(s), Some(c)).map(str::to_string),
        (Some(s), None) => Some(s.to_string()),
        (None, _) => None,
    };

    let (mut file, mut allowed, mut satisfied) = (None, Vec::new(), true);
    let mut subagent_type = None;
    if kind == Kind::Agent {
        if let Some(base_effort) = shipped.as_deref() {
            let files = agent_files(name, base_effort, available);
            let limit = ceiling.as_deref().and_then(rank);
            allowed = files
                .iter()
                .filter(|(_, e)| limit.is_none_or(|l| rank(e).is_some_and(|r| r <= l)))
                .map(|(f, _)| f.clone())
                .collect();
            // Never above the shipped effort, so a ceiling above it picks the base.
            let want = rank(effective.as_deref().unwrap_or(base_effort));
            let pick = files
                .iter()
                .rev()
                .find(|(_, e)| rank(e) <= want)
                .or_else(|| files.first());
            if let Some((f, e)) = pick {
                satisfied = limit.is_none_or(|l| rank(e).is_some_and(|r| r <= l));
                subagent_type = Some(if f == name {
                    format!("playbook:{f}")
                } else {
                    f.clone()
                });
                file = Some(f.clone());
            }
            if allowed.is_empty() {
                allowed = file.iter().cloned().collect();
            }
        }
    }

    Resolution {
        kind,
        name: name.to_string(),
        known,
        shipped,
        configured,
        global,
        claude: claude.map(str::to_string),
        ceiling,
        effective,
        file,
        subagent_type,
        allowed,
        satisfied,
    }
}

impl Resolution {
    pub fn to_json(&self) -> Value {
        json!({
            "kind": self.kind.plural(),
            "name": self.name,
            "known": self.known,
            "shipped": self.shipped,
            "configured": self.configured,
            "global": self.global,
            "claudeCode": self.claude,
            "ceiling": self.ceiling,
            "effective": self.effective,
            "file": self.file,
            "subagentType": self.subagent_type,
            "allowed": self.allowed,
            "satisfied": self.satisfied,
        })
    }

    pub fn to_text(&self) -> String {
        let show = |v: &Option<String>| v.clone().unwrap_or_else(|| "none".to_string());
        let mut out = format!(
            "{} {}: shipped {}, ceiling {}, effective {}",
            self.kind.plural(),
            self.name,
            show(&self.shipped),
            show(&self.ceiling),
            show(&self.effective),
        );
        if let Some(file) = &self.file {
            let ty = self.subagent_type.as_deref().unwrap_or(file);
            out.push_str(&format!("\ndispatch: {ty}"));
            if !self.satisfied {
                out.push_str(
                    " (no variant is low enough for the ceiling, or the session has none)",
                );
            }
        }
        if !self.known {
            out.push_str("\nwarning: no such component in the plugin files");
        }
        out
    }
}

/// Every component under `root`, base agents only (generated variants are
/// left out), sorted by kind then name.
pub fn components(root: &Path) -> Vec<(Kind, String)> {
    let mut found = Vec::new();
    let md_stems = |dir: &Path, skip_generated: bool| -> Vec<String> {
        let mut names: Vec<String> = fs::read_dir(dir)
            .into_iter()
            .flatten()
            .flatten()
            .filter_map(|e| {
                let path = e.path();
                if path.extension()? != "md" {
                    return None;
                }
                if skip_generated && fs::read_to_string(&path).is_ok_and(|t| t.contains(MARKER)) {
                    return None;
                }
                Some(path.file_stem()?.to_string_lossy().into_owned())
            })
            .collect();
        names.sort();
        names
    };
    for name in md_stems(&root.join("agents"), true) {
        found.push((Kind::Agent, name));
    }
    for name in md_stems(&root.join("commands"), false) {
        found.push((Kind::Command, name));
    }
    let mut skills: Vec<String> = fs::read_dir(root.join("skills"))
        .into_iter()
        .flatten()
        .flatten()
        .filter(|e| e.path().join("SKILL.md").is_file())
        .map(|e| e.file_name().to_string_lossy().into_owned())
        .collect();
    skills.sort();
    for name in skills {
        found.push((Kind::Skill, name));
    }
    found
}

/// Component keys set in the global config that match no component under
/// `root`, as `effort.<kind>.<name>`.
pub fn stale_keys(home: &Path, root: &Path) -> Vec<String> {
    let store_root = crate::common::paths::playbook_root_from(home);
    let Ok(keys) = config::store::global_keys(&store_root) else {
        return Vec::new();
    };
    let mut stale = Vec::new();
    for key in keys {
        let mut parts = key.splitn(3, '.');
        let (Some("effort"), Some(kind_name), Some(name)) =
            (parts.next(), parts.next(), parts.next())
        else {
            continue;
        };
        let Some(kind) = Kind::parse(kind_name) else {
            continue;
        };
        if !kind.file(root, name).is_file() {
            stale.push(config_key(kind, name));
        }
    }
    stale.sort();
    stale
}

/// `playbook effort list`: every component with its shipped effort, ceiling
/// and effective level.
pub fn run_list(home: &Path, claude: Option<&str>, root: Option<&Path>, json: bool) -> String {
    let Some(root) = root else {
        return "no plugin root found: run from an installed plugin or set CLAUDE_PLUGIN_ROOT"
            .to_string();
    };
    let rows: Vec<Resolution> = components(root)
        .into_iter()
        .map(|(kind, name)| resolve(kind, &name, home, claude, Some(root)))
        .collect();
    let stale = stale_keys(home, root);
    if json {
        return json!({
            "components": rows.iter().map(Resolution::to_json).collect::<Vec<_>>(),
            "unknownKeys": stale,
        })
        .to_string();
    }
    let dash = |v: &Option<String>| v.clone().unwrap_or_else(|| "-".to_string());
    let mut out = String::from("component                         shipped  set     effective\n");
    for r in &rows {
        out.push_str(&format!(
            "{:<33} {:<8} {:<7} {}\n",
            format!("{}/{}", r.kind.plural(), r.name),
            dash(&r.shipped),
            r.configured,
            dash(&r.effective),
        ));
    }
    for key in &stale {
        out.push_str(&format!("warning: {key} matches no component\n"));
    }
    out.trim_end().to_string()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::common::test_support::scratch_dir;
    use crate::config::write::{self, Tier};

    fn plugin(tag: &str) -> (std::path::PathBuf, std::path::PathBuf) {
        let base = scratch_dir(tag);
        let root = base.join("plugin");
        let home = base.join("home");
        for d in ["agents", "commands", "skills/writing"] {
            fs::create_dir_all(root.join(d)).unwrap();
        }
        fs::create_dir_all(&home).unwrap();
        let agent =
            |effort: &str| format!("---\nname: x\ndescription: d\neffort: {effort}\n---\nBody\n");
        fs::write(root.join("agents/reviewer.md"), agent("high")).unwrap();
        fs::write(root.join("agents/git.md"), agent("low")).unwrap();
        fs::write(root.join("commands/deep-review.md"), agent("high")).unwrap();
        fs::write(
            root.join("skills/writing/SKILL.md"),
            "---\nname: w\n---\nBody\n",
        )
        .unwrap();
        (home, root)
    }

    fn names(list: &[&str]) -> Vec<String> {
        list.iter().map(|s| s.to_string()).collect()
    }

    fn set(home: &Path, key: &str, value: &str) {
        write::set(Tier::Global, key, Value::String(value.into()), home, None).unwrap();
    }

    #[test]
    fn a_component_ceiling_lowers_the_effective_effort() {
        let (home, root) = plugin("cmp-lower");
        set(&home, "effort.commands.deep-review", "medium");
        let r = resolve(Kind::Command, "deep-review", &home, None, Some(&root));
        assert_eq!(r.shipped.as_deref(), Some("high"));
        assert_eq!(r.ceiling.as_deref(), Some("medium"));
        assert_eq!(r.effective.as_deref(), Some("medium"));
    }

    #[test]
    fn a_ceiling_never_raises_the_shipped_effort() {
        let (home, root) = plugin("cmp-never-raise");
        set(&home, "effort.agents.git", "xhigh");
        let r = resolve_with(Kind::Agent, "git", &home, None, Some(&root), &[]);
        assert_eq!(r.effective.as_deref(), Some("low"));
        assert_eq!(r.file.as_deref(), Some("git"));
    }

    #[test]
    fn claude_code_is_the_top_ceiling_and_auto_adds_none() {
        let (home, root) = plugin("cmp-claude");
        set(&home, "effort.commands.deep-review", "xhigh");
        let r = resolve(
            Kind::Command,
            "deep-review",
            &home,
            Some("low"),
            Some(&root),
        );
        assert_eq!(r.ceiling.as_deref(), Some("low"));
        assert_eq!(r.effective.as_deref(), Some("low"));
        let auto = resolve(Kind::Command, "deep-review", &home, None, Some(&root));
        set(&home, "effort.commands.deep-review", "auto");
        let auto2 = resolve(Kind::Command, "deep-review", &home, None, Some(&root));
        assert_eq!(auto2.ceiling, None);
        assert_eq!(auto2.effective.as_deref(), Some("high"));
        assert_eq!(auto.ceiling.as_deref(), Some("xhigh"));
    }

    #[test]
    fn the_global_ceiling_applies_to_a_component_without_its_own() {
        let (home, root) = plugin("cmp-global");
        write::set(
            Tier::Global,
            "maxEffortLevel",
            Value::String("medium".into()),
            &home,
            None,
        )
        .unwrap();
        let r = resolve(Kind::Command, "deep-review", &home, None, Some(&root));
        assert_eq!(r.effective.as_deref(), Some("medium"));
    }

    #[test]
    fn an_agent_resolves_to_the_variant_at_or_below_the_ceiling() {
        let (home, root) = plugin("cmp-variant");
        set(&home, "effort.agents.reviewer", "low");
        let have = names(&["reviewer-low", "reviewer-medium", "reviewer-xhigh"]);
        let r = resolve_with(Kind::Agent, "reviewer", &home, None, Some(&root), &have);
        assert_eq!(r.file.as_deref(), Some("reviewer-low"));
        assert_eq!(r.subagent_type.as_deref(), Some("reviewer-low"));
        assert_eq!(r.allowed, vec!["reviewer-low"]);
        assert!(r.satisfied);
    }

    #[test]
    fn with_no_ceiling_an_agent_keeps_its_base_and_lists_every_variant() {
        let (home, root) = plugin("cmp-base");
        let have = names(&["reviewer-low", "reviewer-medium", "reviewer-xhigh"]);
        let r = resolve_with(Kind::Agent, "reviewer", &home, None, Some(&root), &have);
        assert_eq!(r.file.as_deref(), Some("reviewer"));
        assert_eq!(r.subagent_type.as_deref(), Some("playbook:reviewer"));
        assert!(r.allowed.contains(&"reviewer-low".to_string()));
        assert!(r.allowed.contains(&"reviewer-xhigh".to_string()));
    }

    #[test]
    fn a_session_without_the_launcher_has_only_the_base_agent() {
        let (home, root) = plugin("cmp-no-launcher");
        set(&home, "effort.agents.reviewer", "low");
        let r = resolve_with(Kind::Agent, "reviewer", &home, None, Some(&root), &[]);
        assert_eq!(r.file.as_deref(), Some("reviewer"));
        assert!(!r.satisfied);
    }

    #[test]
    fn an_agent_with_no_variant_low_enough_says_it_cannot_comply() {
        let (home, root) = plugin("cmp-unsatisfied");
        fs::write(
            root.join("agents/critic.md"),
            "---\nname: critic\ndescription: d\neffort: high\n---\n",
        )
        .unwrap();
        set(&home, "effort.agents.critic", "low");
        let r = resolve_with(Kind::Agent, "critic", &home, None, Some(&root), &[]);
        assert_eq!(r.file.as_deref(), Some("critic"));
        assert!(!r.satisfied);
        assert!(r.to_text().contains("no variant is low enough"));
    }

    #[test]
    fn a_skill_with_no_effort_has_no_effective_level() {
        let (home, root) = plugin("cmp-skill");
        let r = resolve(Kind::Skill, "writing", &home, None, Some(&root));
        assert!(r.known);
        assert_eq!(r.shipped, None);
        assert_eq!(r.effective, None);
    }

    #[test]
    fn an_unknown_component_is_reported_not_rejected() {
        let (home, root) = plugin("cmp-unknown");
        set(&home, "effort.agents.ghost", "low");
        let r = resolve_with(Kind::Agent, "ghost", &home, None, Some(&root), &[]);
        assert!(!r.known);
        assert!(r.to_text().contains("no such component"));
        assert_eq!(stale_keys(&home, &root), vec!["effort.agents.ghost"]);
    }

    #[test]
    fn list_shows_every_kind() {
        let (home, root) = plugin("cmp-list");
        let names: Vec<String> = components(&root)
            .into_iter()
            .map(|(k, n)| format!("{}/{n}", k.plural()))
            .collect();
        assert_eq!(
            names,
            vec![
                "agents/git",
                "agents/reviewer",
                "commands/deep-review",
                "skills/writing"
            ]
        );
        let text = run_list(&home, None, Some(&root), false);
        assert!(text.contains("agents/reviewer"), "{text}");
    }
}
