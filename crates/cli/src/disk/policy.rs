// SPDX-FileCopyrightText: 2026 Igor Santos
// SPDX-License-Identifier: Apache-2.0

//! The `disk.*` config keys as one policy, and the volumes it measures.
//!
//! The keys are per-user settings in the global tier, since free space
//! belongs to the machine. All of them are read with one read-only query, so a
//! hook pays for one SQLite open. A missing, unreadable or wrong-typed value
//! falls back to its default, so a broken config store never turns the guard
//! off.

use super::probe::{self, Space, GIB};
use serde_json::Value;
use std::path::{Path, PathBuf};

/// Default `disk.warnFreeGb`: below this many GB free, the hooks warn.
pub const DEFAULT_WARN_FREE_GB: u64 = 15;
/// Default `disk.blockFreeGb`: below this many GB free, build commands and
/// new agents are denied.
pub const DEFAULT_BLOCK_FREE_GB: u64 = 6;
/// Default `disk.maxTargetDirs`: more cargo target folders than this under
/// a worktrees root earns a warning.
pub const DEFAULT_MAX_TARGET_DIRS: u64 = 4;
/// The largest value `disk.maxTargetDirs` accepts.
pub const MAX_TARGET_DIRS: u64 = 1000;
/// The largest value either threshold accepts.
pub const MAX_THRESHOLD_GB: i64 = 10_000;
/// The most entries `disk.paths` accepts.
pub const MAX_PATHS: usize = 16;

/// The resolved `disk.*` keys.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Policy {
    /// Warn below this many GB free. Never below `block_free_gb`.
    pub warn_free_gb: u64,
    /// Deny build commands below this many GB free. 0 turns blocking off.
    pub block_free_gb: u64,
    /// Run the safe cleanup on its own when space is low.
    pub auto_cleanup: bool,
    /// Extra paths to measure, with `~/` expanded.
    pub paths: Vec<PathBuf>,
    /// Warn when more cargo target folders than this exist.
    pub max_target_dirs: u64,
}

impl Default for Policy {
    fn default() -> Self {
        Policy {
            warn_free_gb: DEFAULT_WARN_FREE_GB,
            block_free_gb: DEFAULT_BLOCK_FREE_GB,
            auto_cleanup: true,
            paths: Vec::new(),
            max_target_dirs: DEFAULT_MAX_TARGET_DIRS,
        }
    }
}

/// How a free-space reading compares with the thresholds.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Level {
    /// At or above `disk.warnFreeGb`.
    Ok,
    /// Below `disk.warnFreeGb`.
    Low,
    /// Below `disk.blockFreeGb`.
    Critical,
}

impl Level {
    /// The word `disk status` and the doctor print.
    pub fn label(self) -> &'static str {
        match self {
            Level::Ok => "ok",
            Level::Low => "low",
            Level::Critical => "critical",
        }
    }
}

impl Policy {
    /// Builds a policy from the stored `(key, value)` pairs. A value of the
    /// wrong type or out of range is skipped, so its default applies.
    pub fn from_values(values: &[(String, Value)], home: &Path) -> Policy {
        let mut policy = Policy::default();
        for (key, value) in values {
            match key.as_str() {
                "disk.warnFreeGb" => {
                    if let Some(gb) = threshold(value) {
                        policy.warn_free_gb = gb;
                    }
                }
                "disk.blockFreeGb" => {
                    if let Some(gb) = threshold(value) {
                        policy.block_free_gb = gb;
                    }
                }
                "disk.autoCleanup" => {
                    if let Some(on) = value.as_bool() {
                        policy.auto_cleanup = on;
                    }
                }
                "disk.paths" => {
                    if path_list_error(value).is_none() {
                        policy.paths = value
                            .as_array()
                            .into_iter()
                            .flatten()
                            .filter_map(Value::as_str)
                            .map(|p| expand_home(p, home))
                            .collect();
                    }
                }
                "disk.maxTargetDirs" => {
                    if let Some(n) = value.as_u64().filter(|n| (1..=MAX_TARGET_DIRS).contains(n)) {
                        policy.max_target_dirs = n;
                    }
                }
                _ => {}
            }
        }
        policy.warn_free_gb = policy.warn_free_gb.max(policy.block_free_gb);
        policy
    }

    /// Where `free` bytes stand against the thresholds.
    pub fn level(&self, free: u64) -> Level {
        if free < self.block_free_gb.saturating_mul(GIB) {
            Level::Critical
        } else if free < self.warn_free_gb.saturating_mul(GIB) {
            Level::Low
        } else {
            Level::Ok
        }
    }
}

/// A threshold value: a whole number from 0 to [`MAX_THRESHOLD_GB`].
fn threshold(value: &Value) -> Option<u64> {
    value
        .as_i64()
        .filter(|gb| (0..=MAX_THRESHOLD_GB).contains(gb))
        .and_then(|gb| u64::try_from(gb).ok())
}

/// Why `value` is not a valid `disk.paths` list, or `None` when it is: at
/// most [`MAX_PATHS`] strings, each an absolute path or one starting with
/// `~/`, with no `..` part.
pub fn path_list_error(value: &Value) -> Option<&'static str> {
    const CONSTRAINT: &str = "a list of at most 16 absolute or ~/ paths with no .. part. Example: playbook config set disk.paths '[\"/Volumes/Work\",\"~/code\"]'";
    let Some(items) = value.as_array() else {
        return Some(CONSTRAINT);
    };
    let fine = items.len() <= MAX_PATHS
        && items.iter().all(|item| {
            item.as_str().is_some_and(|text| {
                let absolute = text.starts_with('/') || text == "~" || text.starts_with("~/");
                absolute && !text.contains('\0') && !text.split('/').any(|part| part == "..")
            })
        });
    (!fine).then_some(CONSTRAINT)
}

fn expand_home(text: &str, home: &Path) -> PathBuf {
    match text.strip_prefix('~') {
        Some(rest) => home.join(rest.trim_start_matches('/')),
        None => PathBuf::from(text),
    }
}

/// The policy as stored for the user whose home is `home`. Defaults when
/// the store is missing or cannot be read.
pub fn load(home: &Path) -> Policy {
    Policy::from_values(&stored_values(home), home)
}

/// Every global `disk.*` row, read with one query.
fn stored_values(home: &Path) -> Vec<(String, Value)> {
    let root = crate::common::paths::playbook_root_from(home);
    crate::config::store::global_read_only_prefix(&root, "disk.")
}

/// The paths to measure: the home folder, the repo that holds `cwd` (or
/// `cwd` itself outside a repo), the temp folder where Claude Code writes
/// its scratch files, an absolute `WORKTREE_BASE_DIR` where worktrees and
/// target folders grow, then `disk.paths`.
pub fn measured_paths(policy: &Policy, home: &Path, cwd: Option<&Path>) -> Vec<PathBuf> {
    let mut paths = vec![home.to_path_buf()];
    if let Some(cwd) = cwd {
        paths.push(playbook_common::gitfacts::toplevel(cwd).unwrap_or_else(|| cwd.to_path_buf()));
    }
    paths.push(std::env::temp_dir());
    if let Some(base) = std::env::var_os("WORKTREE_BASE_DIR").map(PathBuf::from) {
        if base.is_absolute() {
            paths.push(base);
        }
    }
    paths.extend(policy.paths.iter().cloned());
    paths
}

/// One measured volume: the first path that named it and its space.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Volume {
    pub path: PathBuf,
    pub space: Space,
}

/// Each distinct volume behind `paths`, in order. A path that cannot be read,
/// or that sits on an in-memory file system, is left out.
pub fn volumes(paths: &[PathBuf]) -> Vec<Volume> {
    let mut found: Vec<Volume> = Vec::new();
    for path in paths {
        let Ok(space) = probe::space(path) else {
            continue;
        };
        if probe::ram_backed(path) {
            continue;
        }
        if found.iter().all(|v| v.space.device != space.device) {
            found.push(Volume {
                path: path.clone(),
                space,
            });
        }
    }
    found
}

/// The volume with the least free space, which sets the level.
pub fn tightest(volumes: &[Volume]) -> Option<&Volume> {
    volumes.iter().min_by_key(|v| v.space.free)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn bytes(gb: f64) -> u64 {
        (gb * GIB as f64) as u64
    }

    #[test]
    fn defaults_block_under_six_and_warn_under_fifteen() {
        let policy = Policy::default();
        let cases = [
            (0.0, Level::Critical),
            (1.0, Level::Critical),
            (5.9, Level::Critical),
            (6.0, Level::Low),
            (14.9, Level::Low),
            (15.0, Level::Ok),
            (400.0, Level::Ok),
        ];
        for (gb, want) in cases {
            assert_eq!(policy.level(bytes(gb)), want, "{gb} GB free");
        }
    }

    #[test]
    fn one_byte_under_the_floor_is_critical() {
        let policy = Policy::default();
        assert_eq!(policy.level(6 * GIB - 1), Level::Critical);
        assert_eq!(policy.level(15 * GIB - 1), Level::Low);
    }

    #[test]
    fn a_zero_floor_never_blocks() {
        let policy = Policy {
            block_free_gb: 0,
            ..Policy::default()
        };
        assert_eq!(policy.level(0), Level::Low);
    }

    #[test]
    fn stored_values_override_defaults() {
        let home = Path::new("/home/u");
        let values = vec![
            ("disk.warnFreeGb".to_string(), json!(30)),
            ("disk.blockFreeGb".to_string(), json!(10)),
            ("disk.autoCleanup".to_string(), json!(false)),
            ("disk.paths".to_string(), json!(["~/code", "/Volumes/Work"])),
            ("disk.maxTargetDirs".to_string(), json!(8)),
        ];
        let policy = Policy::from_values(&values, home);
        assert_eq!(policy.warn_free_gb, 30);
        assert_eq!(policy.block_free_gb, 10);
        assert!(!policy.auto_cleanup);
        assert_eq!(policy.max_target_dirs, 8);
        assert_eq!(
            policy.paths,
            vec![
                PathBuf::from("/home/u/code"),
                PathBuf::from("/Volumes/Work")
            ]
        );
    }

    #[test]
    fn bad_stored_values_keep_the_defaults() {
        let home = Path::new("/home/u");
        let values = vec![
            ("disk.warnFreeGb".to_string(), json!("lots")),
            ("disk.blockFreeGb".to_string(), json!(-1)),
            ("disk.autoCleanup".to_string(), json!("yes")),
            ("disk.paths".to_string(), json!(["relative/path"])),
            ("disk.maxTargetDirs".to_string(), json!(0)),
        ];
        assert_eq!(Policy::from_values(&values, home), Policy::default());
    }

    #[test]
    fn a_warn_value_below_the_floor_is_raised_to_it() {
        let values = vec![
            ("disk.warnFreeGb".to_string(), json!(2)),
            ("disk.blockFreeGb".to_string(), json!(8)),
        ];
        let policy = Policy::from_values(&values, Path::new("/h"));
        assert_eq!(policy.warn_free_gb, 8);
    }

    #[test]
    fn path_list_rules() {
        assert!(path_list_error(&json!([])).is_none());
        assert!(path_list_error(&json!(["/", "~", "~/x", "/Volumes/A"])).is_none());
        assert!(path_list_error(&json!(["x"])).is_some());
        assert!(path_list_error(&json!(["/a/../b"])).is_some());
        assert!(path_list_error(&json!([1])).is_some());
        assert!(path_list_error(&json!("/a")).is_some());
        let many: Vec<String> = (0..17).map(|i| format!("/v{i}")).collect();
        assert!(path_list_error(&json!(many)).is_some());
    }

    fn scratch_home(tag: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "playbook-disk-policy-{}-{tag}",
            crate::testing::run_id()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn config_set_values_are_what_load_reads() {
        use crate::config::write::{self, Tier};
        let home = scratch_home("round-trip");
        for (key, value) in [
            ("disk.warnFreeGb", json!(25)),
            ("disk.blockFreeGb", json!(9)),
            ("disk.autoCleanup", json!(false)),
            ("disk.paths", json!(["~/work"])),
            ("disk.maxTargetDirs", json!(8)),
        ] {
            write::set(Tier::Global, key, value, &home, None).expect(key);
        }
        let policy = load(&home);
        assert_eq!(policy.warn_free_gb, 25);
        assert_eq!(policy.block_free_gb, 9);
        assert!(!policy.auto_cleanup);
        assert_eq!(policy.paths, vec![home.join("work")]);
        assert_eq!(policy.max_target_dirs, 8);
        let _ = std::fs::remove_dir_all(&home);
    }

    #[test]
    fn config_set_refuses_bad_values() {
        use crate::config::write::{self, Tier};
        let home = scratch_home("rejects");
        let cases = [
            ("disk.warnFreeGb", json!(-1)),
            ("disk.warnFreeGb", json!(10_001)),
            ("disk.blockFreeGb", json!(5.5)),
            ("disk.blockFreeGb", json!("6")),
            ("disk.autoCleanup", json!("yes")),
            ("disk.paths", json!(["relative"])),
            ("disk.paths", json!(["/a/../b"])),
            ("disk.paths", json!("/a")),
            ("disk.maxTargetDirs", json!(0)),
            ("disk.maxTargetDirs", json!(1001)),
            (
                "disk.paths",
                json!((0..17).map(|i| format!("/v{i}")).collect::<Vec<_>>()),
            ),
        ];
        for (key, value) in cases {
            let shown = value.to_string();
            let error = write::set(Tier::Global, key, value, &home, None)
                .expect_err(&format!("{key} = {shown} should be refused"));
            assert!(error.to_string().contains(key), "{error}");
        }
        for (key, value) in [
            ("disk.warnFreeGb", json!(0)),
            ("disk.blockFreeGb", json!(10_000)),
        ] {
            write::set(Tier::Global, key, value, &home, None).expect(key);
        }
        let _ = std::fs::remove_dir_all(&home);
    }

    #[test]
    fn missing_store_gives_defaults() {
        let home = std::env::temp_dir().join("pb-disk-policy-no-store");
        assert_eq!(load(&home), Policy::default());
    }

    #[test]
    fn measured_paths_lists_home_cwd_then_extras() {
        let policy = Policy {
            paths: vec![PathBuf::from("/extra")],
            ..Policy::default()
        };
        let tmp = std::env::temp_dir();
        let paths = measured_paths(&policy, Path::new("/h"), Some(&tmp));
        assert_eq!(paths.first(), Some(&PathBuf::from("/h")));
        assert!(paths.contains(&std::env::temp_dir()));
        assert_eq!(paths.last(), Some(&PathBuf::from("/extra")));
    }

    #[test]
    fn volumes_on_one_device_count_once() {
        let tmp = std::env::temp_dir();
        let found = volumes(&[tmp.clone(), tmp.join("pb-disk-missing-child")]);
        assert_eq!(found.len(), 1);
        assert_eq!(tightest(&found).map(|v| v.path.clone()), Some(tmp));
    }

    #[test]
    fn the_volume_with_least_free_space_sets_the_level() {
        let volume = |name: &str, free: u64, device: u64| Volume {
            path: PathBuf::from(name),
            space: Space {
                free,
                total: 100 * GIB,
                device,
            },
        };
        let found = vec![
            volume("/home", 40 * GIB, 1),
            volume("/repo", 3 * GIB, 2),
            volume("/extra", 20 * GIB, 3),
        ];
        assert_eq!(
            tightest(&found).map(|v| v.path.clone()),
            Some(PathBuf::from("/repo"))
        );
        assert!(tightest(&[]).is_none());
    }
}
