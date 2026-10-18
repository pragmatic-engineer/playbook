// SPDX-FileCopyrightText: 2026 Igor Santos
// SPDX-License-Identifier: Apache-2.0

//! `playbook disk clean` (ADR 0047 decision 5): removes only what playbook
//! can prove is disposable, and says what it freed.
//!
//! Every candidate is a direct child of one of a few known bases, and passes
//! the in-use checks for its kind and the protected-path check before it
//! goes. Destructive: it deletes folders, so the selection is kept apart
//! from the deletion and tested on its own.

use super::{ballast, size, targets};
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime};

/// A target folder touched more recently than this may be in a build.
pub const TARGET_IDLE: Duration = Duration::from_secs(30 * 60);
/// A scratch entry touched more recently than this may be in use.
pub const SCRATCH_IDLE: Duration = Duration::from_secs(60 * 60);
/// A plugin cache marked orphaned more recently than this may still serve a
/// running session.
pub const ORPHAN_IDLE: Duration = Duration::from_secs(24 * 60 * 60);

/// Why a folder is a candidate.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    /// A per-agent cargo target folder under a worktrees root.
    Target,
    /// A `pb-*` entry in `/tmp`.
    Scratch,
    /// An entry in playbook's own cache or tmp folder.
    Cache,
    /// A plugin cache version Claude Code marked orphaned.
    OrphanedPlugin,
}

impl Kind {
    pub fn label(self) -> &'static str {
        match self {
            Kind::Target => "target",
            Kind::Scratch => "scratch",
            Kind::Cache => "cache",
            Kind::OrphanedPlugin => "orphaned plugin",
        }
    }
}

/// One thing the cleanup removes.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Candidate {
    pub kind: Kind,
    pub path: PathBuf,
    pub bytes: u64,
}

/// One thing the cleanup looked at and kept, with the reason.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Kept {
    pub path: PathBuf,
    pub reason: String,
}

/// What the selection reads from the machine. Tests build it by hand.
pub struct Env {
    pub home: PathBuf,
    pub claude_home: PathBuf,
    /// The worktrees root of the current repo, where `cargo-*` folders live.
    pub worktrees_root: Option<PathBuf>,
    /// The system temp folders that hold `pb-*` scratch.
    pub tmp_dirs: Vec<PathBuf>,
    pub now: SystemTime,
    /// The command lines of every running process, one per line.
    pub process_args: String,
    /// This user's id, so only their own scratch is removed.
    pub uid: Option<u32>,
}

impl Env {
    /// The machine as it is now.
    pub fn from_system(home: &Path, repo_root: Option<&Path>) -> Env {
        Env {
            home: home.to_path_buf(),
            claude_home: home.join(".claude"),
            worktrees_root: repo_root.and_then(targets::worktrees_root),
            tmp_dirs: vec![PathBuf::from("/tmp"), PathBuf::from("/private/tmp")],
            now: SystemTime::now(),
            process_args: process_args(),
            uid: current_uid(),
        }
    }
}

/// Every running process's command line, from `ps`. Empty when `ps` fails,
/// which leaves the other in-use checks in place.
fn process_args() -> String {
    std::process::Command::new("ps")
        .args(["-axww", "-o", "args="])
        .output()
        .ok()
        .filter(|o| o.status.success())
        .map(|o| String::from_utf8_lossy(&o.stdout).into_owned())
        .unwrap_or_default()
}

#[cfg(unix)]
fn current_uid() -> Option<u32> {
    // SAFETY: getuid has no preconditions and cannot fail.
    Some(unsafe { libc::getuid() })
}

#[cfg(not(unix))]
fn current_uid() -> Option<u32> {
    None
}

/// The playbook data that is never removed, the same list the rm guard
/// protects (ADR 0045): the memory store, the databases, plans and
/// handoffs, legacy org config, session state, the effort ledger, the
/// system prompt and the config file.
pub fn protected(home: &Path) -> Vec<PathBuf> {
    let root = crate::common::paths::playbook_root_from(home);
    let legacy_db = playbook_data::config::legacy_db_in(&root);
    let mut paths: Vec<PathBuf> = ["memory", "repos", "orgs", "runtime", "effort", "prompts"]
        .iter()
        .map(|name| root.join(name))
        .collect();
    paths.push(playbook_data::data_dir_from(home));
    paths.push(crate::config::global_config_path(&root));
    paths.push(legacy_db);
    paths.push(ballast::dir(home));
    paths
}

/// Whether removing `path` could touch protected data: it is a protected
/// path, sits inside one, or holds one. Compared without letter case, since
/// macOS disks ignore it by default.
pub fn touches_protected(path: &Path, protected: &[PathBuf]) -> bool {
    let fold = |p: &Path| PathBuf::from(p.to_string_lossy().to_lowercase());
    let path = fold(path);
    protected
        .iter()
        .map(|p| fold(p))
        .any(|p| path.starts_with(&p) || p.starts_with(&path))
}

/// Picks what to remove. Nothing is deleted here.
pub fn select(env: &Env) -> (Vec<Candidate>, Vec<Kept>) {
    let protected = protected(&env.home);
    let mut picked = Vec::new();
    let mut kept = Vec::new();
    let mut consider =
        |kind: Kind, base: &Path, path: PathBuf, idle: Duration, extra: Option<String>| {
            let reason = extra
                .or_else(|| unsafe_target(base, &path, &protected))
                .or_else(|| named_by_a_process(&path, &env.process_args))
                .or_else(|| {
                    (kind == Kind::Target)
                        .then(|| build_lock_held(&path))
                        .flatten()
                });
            if let Some(reason) = reason {
                kept.push(Kept { path, reason });
                return;
            }
            let scan = size::scan(&path);
            let idle_for = env.now.duration_since(scan.newest).unwrap_or_default();
            if idle_for < idle {
                kept.push(Kept {
                    path,
                    reason: format!("changed {} minutes ago", idle_for.as_secs() / 60),
                });
                return;
            }
            picked.push(Candidate {
                kind,
                path,
                bytes: scan.bytes,
            });
        };

    if let Some(root) = &env.worktrees_root {
        for dir in targets::target_dirs(root) {
            consider(Kind::Target, root, dir, TARGET_IDLE, None);
        }
    }
    for tmp in unique_dirs(&env.tmp_dirs) {
        for path in children(&tmp).into_iter().filter(|p| name_starts(p, "pb-")) {
            let other_owner = env.uid.is_some_and(|uid| owner(&path) != Some(uid));
            let extra = other_owner.then(|| "owned by another user".to_string());
            consider(Kind::Scratch, &tmp, path, SCRATCH_IDLE, extra);
        }
    }
    let cache_bases = [
        env.home.join(".cache/playbook"),
        crate::common::paths::playbook_root_from(&env.home).join("tmp"),
    ];
    for base in cache_bases {
        for path in children(&base)
            .into_iter()
            .filter(|p| p != &ballast::dir(&env.home))
        {
            consider(Kind::Cache, &base, path, SCRATCH_IDLE, None);
        }
    }
    let plugin_base = crate::plugin_root::cache_dir(&env.claude_home);
    for path in crate::plugin_root::orphaned_cache_dirs(&env.claude_home) {
        let marked = std::fs::symlink_metadata(path.join(crate::plugin_root::ORPHAN_MARKER))
            .and_then(|m| m.modified())
            .ok();
        let recent =
            marked.is_none_or(|at| env.now.duration_since(at).unwrap_or_default() < ORPHAN_IDLE);
        let extra = recent.then(|| "marked orphaned less than a day ago".to_string());
        consider(
            Kind::OrphanedPlugin,
            &plugin_base,
            path,
            Duration::ZERO,
            extra,
        );
    }
    picked.sort_by_key(|c| std::cmp::Reverse(c.bytes));
    (picked, kept)
}

/// Why `path` must not be removed whatever its age: it is a symlink, not a
/// direct child of `base`, or touches protected data.
fn unsafe_target(base: &Path, path: &Path, protected: &[PathBuf]) -> Option<String> {
    let meta = std::fs::symlink_metadata(path).ok()?;
    if meta.file_type().is_symlink() {
        return Some("a symlink".to_string());
    }
    if path.parent() != Some(base) || path.file_name().is_none() {
        return Some("outside its base folder".to_string());
    }
    touches_protected(path, protected).then(|| "protected playbook data".to_string())
}

/// A running process names `path` on its command line, as rustc does with
/// `--out-dir`.
fn named_by_a_process(path: &Path, process_args: &str) -> Option<String> {
    let text = path.to_string_lossy();
    process_args
        .lines()
        .any(|line| line.contains(text.as_ref()))
        .then(|| "in use by a running process".to_string())
}

/// Cargo holds an exclusive lock on `<target>/<profile>/.cargo-lock` for the
/// whole build. A lock this process cannot take means a build is running.
fn build_lock_held(target: &Path) -> Option<String> {
    let mut locks = Vec::new();
    for profile in children(target) {
        locks.push(profile.join(".cargo-lock"));
        for nested in children(&profile) {
            locks.push(nested.join(".cargo-lock"));
        }
    }
    locks
        .iter()
        .filter(|lock| lock.is_file())
        .any(|lock| lock_is_held(lock))
        .then(|| "a cargo build holds its lock".to_string())
}

#[cfg(unix)]
pub(crate) fn lock_is_held(path: &Path) -> bool {
    use std::os::unix::io::AsRawFd;
    let Ok(file) = std::fs::File::open(path) else {
        return false;
    };
    // SAFETY: the descriptor comes from `file`, which stays open for both
    // calls. A lock taken here is released at once.
    unsafe {
        if libc::flock(file.as_raw_fd(), libc::LOCK_EX | libc::LOCK_NB) != 0 {
            return true;
        }
        libc::flock(file.as_raw_fd(), libc::LOCK_UN);
    }
    false
}

#[cfg(not(unix))]
pub(crate) fn lock_is_held(_path: &Path) -> bool {
    true
}

#[cfg(unix)]
fn owner(path: &Path) -> Option<u32> {
    use std::os::unix::fs::MetadataExt;
    std::fs::symlink_metadata(path).ok().map(|m| m.uid())
}

#[cfg(not(unix))]
fn owner(_path: &Path) -> Option<u32> {
    None
}

fn name_starts(path: &Path, prefix: &str) -> bool {
    path.file_name()
        .is_some_and(|n| n.to_string_lossy().starts_with(prefix))
}

fn children(dir: &Path) -> Vec<PathBuf> {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return Vec::new();
    };
    let mut found: Vec<PathBuf> = entries.filter_map(Result::ok).map(|e| e.path()).collect();
    found.sort();
    found
}

/// `dirs` that exist, with links resolved and each real folder once, so
/// `/tmp` and `/private/tmp` on macOS count once.
fn unique_dirs(dirs: &[PathBuf]) -> Vec<PathBuf> {
    let mut seen = Vec::new();
    for dir in dirs {
        if let Ok(real) = std::fs::canonicalize(dir) {
            if !seen.contains(&real) {
                seen.push(real);
            }
        }
    }
    seen
}

/// Removes each candidate. Returns what went and what failed.
pub fn remove(candidates: Vec<Candidate>) -> (Vec<Candidate>, Vec<Kept>) {
    let mut removed = Vec::new();
    let mut failed = Vec::new();
    for candidate in candidates {
        let result = match std::fs::symlink_metadata(&candidate.path) {
            Ok(meta) if meta.is_dir() => std::fs::remove_dir_all(&candidate.path),
            Ok(_) => std::fs::remove_file(&candidate.path),
            Err(e) => Err(e),
        };
        match result {
            Ok(()) => removed.push(candidate),
            Err(e) => failed.push(Kept {
                path: candidate.path,
                reason: format!("could not remove: {e}"),
            }),
        }
    }
    (removed, failed)
}

/// Holds the cleanup lock until dropped. On a full disk the lock file may
/// not be creatable, and the cleanup then runs without it.
pub struct Lock {
    _file: Option<std::fs::File>,
}

/// Takes the cleanup lock, or `None` when another cleanup holds it.
pub fn lock(home: &Path) -> Option<Lock> {
    let path = crate::common::paths::playbook_root_from(home).join("disk-clean.lock");
    let Ok(file) = std::fs::OpenOptions::new()
        .create(true)
        .truncate(false)
        .write(true)
        .open(&path)
    else {
        return Some(Lock { _file: None });
    };
    try_lock(&file).then_some(Lock { _file: Some(file) })
}

#[cfg(unix)]
fn try_lock(file: &std::fs::File) -> bool {
    use std::os::unix::io::AsRawFd;
    // SAFETY: the descriptor belongs to `file`, which outlives the call. The
    // lock is released when the file is closed.
    unsafe { libc::flock(file.as_raw_fd(), libc::LOCK_EX | libc::LOCK_NB) == 0 }
}

#[cfg(not(unix))]
fn try_lock(_file: &std::fs::File) -> bool {
    true
}

/// Whether an automatic cleanup should run: `disk.autoCleanup` is on and
/// the tightest volume is below `disk.warnFreeGb`.
pub fn auto_due(home: &Path, cwd: &Path) -> bool {
    use super::policy;
    let policy = policy::load(home);
    let volumes = policy::volumes(&policy::measured_paths(&policy, home, Some(cwd)));
    policy.auto_cleanup
        && policy::tightest(&volumes)
            .is_some_and(|v| policy.level(v.space.free) != super::Level::Ok)
}

/// Runs the whole cleanup for the repo at `repo_root`, if any: the selection
/// above, then the worktree sweep. Returns the lines to print. A dry run
/// takes no lock and writes nothing.
pub fn run(home: &Path, repo_root: Option<&Path>, dry_run: bool) -> Vec<String> {
    let _lock = if dry_run {
        None
    } else {
        let Some(held) = lock(home) else {
            return vec!["disk clean: another cleanup is running, nothing to do".to_string()];
        };
        Some(held)
    };
    let env = Env::from_system(home, repo_root);
    let (picked, kept) = select(&env);
    let mut lines = Vec::new();
    let verb = if dry_run { "would remove" } else { "removed" };
    let (done, failed) = if dry_run {
        (picked, Vec::new())
    } else {
        remove(picked)
    };
    for c in &done {
        lines.push(format!(
            "{verb} {:>9}  {:<15}  {}",
            size::gb(c.bytes),
            c.kind.label(),
            c.path.display()
        ));
    }
    for k in kept.iter().chain(&failed) {
        lines.push(format!("kept {}: {}", k.path.display(), k.reason));
    }
    if let Some(repo) = repo_root {
        let slug = playbook_common::repo::repo_slug_in(repo);
        let slug = (!slug.is_empty()).then_some(slug.as_str());
        let now = crate::common::time::now_secs();
        match crate::worktree::sweep(repo, home, slug, dry_run, now) {
            Ok(sweep) => lines.extend(sweep),
            Err(e) => lines.push(format!("worktree sweep: {e}")),
        }
    }
    let total: u64 = done.iter().map(|c| c.bytes).sum();
    lines.push(format!(
        "disk clean: {verb} {} in {} item(s)",
        size::gb(total),
        done.len()
    ));
    lines
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;
    use std::fs;

    struct Fixture {
        root: PathBuf,
        env: Env,
    }

    /// A scratch home, a worktrees root and a tmp folder, all inside one
    /// temp folder the test made. `now` is two hours ahead, so everything
    /// the test writes counts as idle unless a test says otherwise.
    fn fixture(tag: &str) -> Fixture {
        let root =
            std::env::temp_dir().join(format!("pb-disk-clean-{}-{tag}", crate::testing::run_id()));
        let _ = fs::remove_dir_all(&root);
        let home = root.join("home");
        for dir in ["home/.claude", "worktrees", "tmp"] {
            fs::create_dir_all(root.join(dir)).unwrap();
        }
        let root = fs::canonicalize(&root).unwrap();
        let env = Env {
            home: root.join("home"),
            claude_home: root.join("home/.claude"),
            worktrees_root: Some(root.join("worktrees")),
            tmp_dirs: vec![root.join("tmp")],
            now: SystemTime::now() + Duration::from_secs(2 * 60 * 60),
            process_args: String::new(),
            uid: current_uid(),
        };
        let _ = home;
        Fixture { root, env }
    }

    fn write(path: &Path) {
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(path, vec![1u8; 4096]).unwrap();
    }

    fn picked(env: &Env) -> Vec<PathBuf> {
        select(env).0.into_iter().map(|c| c.path).collect()
    }

    #[test]
    fn picks_idle_targets_scratch_and_cache() {
        let f = fixture("picks");
        write(&f.root.join("worktrees/cargo-a/debug/deps/x.rlib"));
        write(&f.root.join("tmp/pb-old/file"));
        write(&f.env.home.join(".cache/playbook/old/file"));
        write(&f.env.home.join(".config/playbook/tmp/doc/file"));
        let got = picked(&f.env);
        assert!(got.contains(&f.root.join("worktrees/cargo-a")), "{got:?}");
        assert!(got.contains(&f.root.join("tmp/pb-old")), "{got:?}");
        assert!(
            got.contains(&f.env.home.join(".cache/playbook/old")),
            "{got:?}"
        );
        assert!(
            got.contains(&f.env.home.join(".config/playbook/tmp/doc")),
            "{got:?}"
        );
        fs::remove_dir_all(&f.root).unwrap();
    }

    #[test]
    fn keeps_a_target_changed_in_the_last_half_hour() {
        let mut f = fixture("fresh");
        write(&f.root.join("worktrees/cargo-busy/debug/x"));
        f.env.now = SystemTime::now() + Duration::from_secs(10 * 60);
        let (got, kept) = select(&f.env);
        assert!(got.is_empty(), "{got:?}");
        assert!(kept[0].reason.starts_with("changed"), "{kept:?}");
        fs::remove_dir_all(&f.root).unwrap();
    }

    #[test]
    fn keeps_a_target_whose_build_lock_is_held() {
        use std::os::unix::io::AsRawFd;
        let f = fixture("locked");
        let lock = f.root.join("worktrees/cargo-building/debug/.cargo-lock");
        write(&lock);
        let held = fs::File::open(&lock).unwrap();
        // SAFETY: `held` stays open until the end of the test.
        assert_eq!(unsafe { libc::flock(held.as_raw_fd(), libc::LOCK_EX) }, 0);
        let (got, kept) = select(&f.env);
        assert!(got.is_empty(), "{got:?}");
        assert_eq!(kept[0].reason, "a cargo build holds its lock");
        drop(held);
        assert_eq!(
            picked(&f.env),
            vec![f.root.join("worktrees/cargo-building")]
        );
        fs::remove_dir_all(&f.root).unwrap();
    }

    #[test]
    fn keeps_a_folder_a_running_process_names() {
        let mut f = fixture("ps");
        let dir = f.root.join("worktrees/cargo-rustc");
        write(&dir.join("debug/x"));
        f.env.process_args = format!("rustc --out-dir {}/debug/deps src/lib.rs\n", dir.display());
        let (got, kept) = select(&f.env);
        assert!(got.is_empty());
        assert_eq!(kept[0].reason, "in use by a running process");
        fs::remove_dir_all(&f.root).unwrap();
    }

    #[test]
    fn never_picks_links_other_names_the_ballast_or_protected_data() {
        let f = fixture("never");
        let outside = f.root.join("outside");
        write(&outside.join("keep"));
        std::os::unix::fs::symlink(&outside, f.root.join("worktrees/cargo-link")).unwrap();
        std::os::unix::fs::symlink(&outside, f.root.join("tmp/pb-link")).unwrap();
        write(&f.root.join("worktrees/playbook/feature/src.rs"));
        write(&f.root.join("tmp/other-tool/file"));
        write(&ballast::file(&f.env.home));
        write(&f.env.home.join(".config/playbook/memory/fact.md"));
        write(&f.env.home.join(".config/playbook/data/config.db"));
        let got = picked(&f.env);
        assert!(got.is_empty(), "{got:?}");
        assert!(outside.join("keep").exists());
        fs::remove_dir_all(&f.root).unwrap();
    }

    #[test]
    fn protected_paths_and_their_parents_are_refused() {
        let home = Path::new("/Users/me");
        let list = protected(home);
        for path in [
            "/Users/me/.config/playbook/memory",
            "/Users/me/.config/playbook/memory/x",
            "/Users/me/.config/playbook",
            "/Users/me/.config/playbook/MEMORY",
            "/Users/me/.config/playbook/data/config.db",
            "/Users/me/.cache/playbook/ballast",
            "/Users/me",
        ] {
            assert!(touches_protected(Path::new(path), &list), "{path}");
        }
        for path in [
            "/Users/me/.config/playbook/tmp/x",
            "/Users/me/.cache/playbook/old",
            "/w/.worktrees/cargo-a",
        ] {
            assert!(!touches_protected(Path::new(path), &list), "{path}");
        }
    }

    #[test]
    fn keeps_a_fresh_orphan_and_picks_an_old_one() {
        let mut f = fixture("orphan");
        let cache = crate::plugin_root::cache_dir(&f.env.claude_home);
        write(&cache.join("0.30.0").join(crate::plugin_root::ORPHAN_MARKER));
        write(&cache.join("0.37.0/plugin.json"));
        f.env.now = SystemTime::now() + Duration::from_secs(60 * 60);
        assert!(picked(&f.env).is_empty());
        f.env.now = SystemTime::now() + Duration::from_secs(25 * 60 * 60);
        assert_eq!(picked(&f.env), vec![cache.join("0.30.0")]);
        fs::remove_dir_all(&f.root).unwrap();
    }

    #[test]
    fn a_second_cleanup_waits_for_the_lock() {
        let f = fixture("lock");
        fs::create_dir_all(crate::common::paths::playbook_root_from(&f.env.home)).unwrap();
        let first = lock(&f.env.home).expect("first lock");
        assert!(lock(&f.env.home).is_none());
        drop(first);
        assert!(lock(&f.env.home).is_some());
        fs::remove_dir_all(&f.root).unwrap();
    }

    #[test]
    fn remove_deletes_only_what_was_picked() {
        let f = fixture("remove");
        write(&f.root.join("worktrees/cargo-gone/debug/x"));
        write(&f.root.join("worktrees/playbook/keep/src.rs"));
        let (picked, _) = select(&f.env);
        let (removed, failed) = remove(picked);
        assert_eq!(removed.len(), 1);
        assert!(failed.is_empty());
        assert!(!f.root.join("worktrees/cargo-gone").exists());
        assert!(f.root.join("worktrees/playbook/keep/src.rs").exists());
        fs::remove_dir_all(&f.root).unwrap();
    }
}
