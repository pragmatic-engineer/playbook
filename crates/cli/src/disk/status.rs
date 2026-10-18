// SPDX-FileCopyrightText: 2026 Igor Santos
// SPDX-License-Identifier: Apache-2.0

//! `playbook disk status`: free space per volume, the thresholds, and the
//! biggest folders playbook and its agents made.

use super::policy::{self, Level, Policy, Volume};
use super::{ballast, size, targets};
use serde_json::{json, Value};
use std::path::{Path, PathBuf};

/// What a consumer is.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    /// A per-agent cargo target folder.
    Target,
    /// A linked git worktree of this repo.
    Worktree,
    /// A cache or scratch folder playbook owns.
    Cache,
    /// The ballast file.
    Ballast,
}

impl Kind {
    pub fn label(self) -> &'static str {
        match self {
            Kind::Target => "target",
            Kind::Worktree => "worktree",
            Kind::Cache => "cache",
            Kind::Ballast => "ballast",
        }
    }
}

/// One folder or file and the space it takes.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Consumer {
    pub kind: Kind,
    pub path: PathBuf,
    pub bytes: u64,
}

/// Everything `disk status` prints.
pub struct Report {
    pub policy: Policy,
    pub volumes: Vec<Volume>,
    pub consumers: Vec<Consumer>,
    pub target_count: usize,
}

impl Report {
    /// The level of the volume with the least free space, `None` when no
    /// volume could be read.
    pub fn level(&self) -> Option<Level> {
        policy::tightest(&self.volumes).map(|v| self.policy.level(v.space.free))
    }
}

/// The folders to measure, before their sizes are known.
pub fn candidates(
    home: &Path,
    claude_home: &Path,
    repo_root: Option<&Path>,
) -> Vec<(Kind, PathBuf)> {
    let mut found = Vec::new();
    if let Some(repo) = repo_root {
        if let Some(root) = targets::worktrees_root(repo) {
            found.extend(
                targets::target_dirs(&root)
                    .into_iter()
                    .map(|p| (Kind::Target, p)),
            );
        }
        if let Ok(list) = crate::worktree::list_worktrees(repo) {
            found.extend(list.into_iter().skip(1).map(|w| (Kind::Worktree, w.path)));
        }
    }
    found.extend(
        cache_dirs(home, claude_home)
            .into_iter()
            .map(|p| (Kind::Cache, p)),
    );
    if ballast::size(home).is_some() {
        found.push((Kind::Ballast, ballast::file(home)));
    }
    found
}

/// Playbook's own caches and scratch folders. The ballast folder sits under
/// `~/.cache/playbook` and is reported on its own.
fn cache_dirs(home: &Path, claude_home: &Path) -> Vec<PathBuf> {
    let mut dirs = Vec::new();
    let cache = home.join(".cache/playbook");
    if let Ok(entries) = std::fs::read_dir(&cache) {
        dirs.extend(
            entries
                .filter_map(Result::ok)
                .map(|e| e.path())
                .filter(|p| p != &ballast::dir(home)),
        );
    }
    dirs.push(crate::common::paths::playbook_root_from(home).join("tmp"));
    dirs.push(claude_home.join("plugins/cache/pragmatic-engineer/playbook"));
    dirs.retain(|p| p.exists());
    dirs
}

/// Measures every candidate, biggest first. Sizing walks every file, so it
/// runs four folders at a time.
pub fn measure(candidates: Vec<(Kind, PathBuf)>) -> Vec<Consumer> {
    let mut consumers = crate::common::par::map(
        &candidates,
        crate::common::par::MAX_CONCURRENT,
        |(kind, path)| Consumer {
            kind: *kind,
            path: path.clone(),
            bytes: size::allocated(path),
        },
    );
    consumers.sort_by(|a, b| b.bytes.cmp(&a.bytes).then_with(|| a.path.cmp(&b.path)));
    consumers
}

/// Builds the full report.
pub fn report(home: &Path, claude_home: &Path, cwd: &Path) -> Report {
    let policy = policy::load(home);
    let repo_root = playbook_common::gitfacts::toplevel(cwd);
    let volumes = policy::volumes(&policy::measured_paths(&policy, home, Some(cwd)));
    let consumers = measure(candidates(home, claude_home, repo_root.as_deref()));
    let target_count = consumers.iter().filter(|c| c.kind == Kind::Target).count();
    Report {
        policy,
        volumes,
        consumers,
        target_count,
    }
}

/// How many consumers the text output lists.
const SHOWN: usize = 15;

/// The report as lines of text.
pub fn render(report: &Report) -> String {
    let p = &report.policy;
    let mut out = String::new();
    out.push_str(&format!(
        "Thresholds: warn below {} GB free, block below {} GB free (auto cleanup {})\n",
        p.warn_free_gb,
        p.block_free_gb,
        if p.auto_cleanup { "on" } else { "off" }
    ));
    out.push_str("\nVolumes\n");
    if report.volumes.is_empty() {
        out.push_str("  could not read free space\n");
    }
    for v in &report.volumes {
        out.push_str(&format!(
            "  {:<9} {} free of {}  {}\n",
            p.level(v.space.free).label(),
            size::gb(v.space.free),
            size::gb(v.space.total),
            v.path.display()
        ));
    }
    out.push_str("\nBiggest playbook folders\n");
    if report.consumers.is_empty() {
        out.push_str("  none found\n");
    }
    for c in report.consumers.iter().take(SHOWN) {
        out.push_str(&format!(
            "  {:>9}  {:<8}  {}\n",
            size::gb(c.bytes),
            c.kind.label(),
            c.path.display()
        ));
    }
    if report.consumers.len() > SHOWN {
        out.push_str(&format!(
            "  ... and {} more (see --json)\n",
            report.consumers.len() - SHOWN
        ));
    }
    if report.target_count as u64 > p.max_target_dirs {
        out.push_str(&format!(
            "\n{} cargo target folders exist, more than disk.maxTargetDirs ({}). Remove the ones no agent uses with `cargo clean` or `playbook disk clean`.\n",
            report.target_count, p.max_target_dirs
        ));
    }
    out
}

/// The report as JSON for tools.
pub fn to_json(report: &Report) -> Value {
    let p = &report.policy;
    json!({
        "level": report.level().map(Level::label),
        "thresholds": {
            "warnFreeGb": p.warn_free_gb,
            "blockFreeGb": p.block_free_gb,
            "autoCleanup": p.auto_cleanup,
            "maxTargetDirs": p.max_target_dirs,
        },
        "volumes": report.volumes.iter().map(|v| json!({
            "path": v.path,
            "freeBytes": v.space.free,
            "totalBytes": v.space.total,
            "level": p.level(v.space.free).label(),
        })).collect::<Vec<_>>(),
        "consumers": report.consumers.iter().map(|c| json!({
            "kind": c.kind.label(),
            "path": c.path,
            "bytes": c.bytes,
        })).collect::<Vec<_>>(),
        "targetDirs": report.target_count,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::disk::probe::{Space, GIB};

    fn sample(target_count: usize) -> Report {
        Report {
            policy: Policy::default(),
            volumes: vec![Volume {
                path: PathBuf::from("/Users/me"),
                space: Space {
                    free: 5 * GIB,
                    total: 460 * GIB,
                    device: 1,
                },
            }],
            consumers: vec![
                Consumer {
                    kind: Kind::Target,
                    path: PathBuf::from("/w/.worktrees/cargo-a"),
                    bytes: 2 * GIB,
                },
                Consumer {
                    kind: Kind::Ballast,
                    path: PathBuf::from("/Users/me/.cache/playbook/ballast/ballast.bin"),
                    bytes: GIB,
                },
            ],
            target_count,
        }
    }

    #[test]
    fn text_names_the_level_sizes_and_paths() {
        let text = render(&sample(1));
        assert!(
            text.contains("warn below 15 GB free, block below 6 GB free"),
            "{text}"
        );
        assert!(text.contains("critical"), "{text}");
        assert!(text.contains("5.0 GB free of 460.0 GB"), "{text}");
        assert!(
            text.contains("2.0 GB  target    /w/.worktrees/cargo-a"),
            "{text}"
        );
        assert!(text.contains("ballast"), "{text}");
        assert!(!text.contains("disk.maxTargetDirs"), "{text}");
    }

    #[test]
    fn text_warns_about_too_many_target_folders() {
        let text = render(&sample(5));
        assert!(
            text.contains("5 cargo target folders exist, more than disk.maxTargetDirs (4)"),
            "{text}"
        );
    }

    #[test]
    fn json_has_every_field() {
        let value = to_json(&sample(1));
        assert_eq!(value["level"], "critical");
        assert_eq!(value["thresholds"]["blockFreeGb"], 6);
        assert_eq!(value["volumes"][0]["freeBytes"], 5 * GIB);
        assert_eq!(value["consumers"][0]["kind"], "target");
        assert_eq!(value["consumers"][1]["kind"], "ballast");
        assert_eq!(value["targetDirs"], 1);
    }

    #[test]
    fn measure_sorts_biggest_first() {
        let dir = std::env::temp_dir().join(format!("pb-disk-status-{}", crate::testing::run_id()));
        std::fs::create_dir_all(dir.join("small")).unwrap();
        std::fs::create_dir_all(dir.join("big")).unwrap();
        std::fs::write(dir.join("small/f"), vec![1u8; 8 * 1024]).unwrap();
        std::fs::write(dir.join("big/f"), vec![1u8; 512 * 1024]).unwrap();
        let found = measure(vec![
            (Kind::Cache, dir.join("small")),
            (Kind::Target, dir.join("big")),
        ]);
        assert_eq!(found[0].path, dir.join("big"));
        assert!(found[0].bytes > found[1].bytes);
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn caches_leave_out_the_ballast_folder() {
        let home =
            std::env::temp_dir().join(format!("pb-disk-status-home-{}", crate::testing::run_id()));
        std::fs::create_dir_all(home.join(".cache/playbook/ballast")).unwrap();
        std::fs::create_dir_all(home.join(".cache/playbook/other")).unwrap();
        let dirs = cache_dirs(&home, &home.join(".claude"));
        assert_eq!(dirs, vec![home.join(".cache/playbook/other")]);
        std::fs::remove_dir_all(&home).unwrap();
    }
}
