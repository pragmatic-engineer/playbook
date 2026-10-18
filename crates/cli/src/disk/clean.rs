// SPDX-FileCopyrightText: 2026 Igor Santos
// SPDX-License-Identifier: Apache-2.0

//! `playbook disk clean` (ADR 0047 decision 5): removes only what playbook
//! can prove is disposable, and says what it freed.
//!
//! Every candidate is a direct child of one of a few known bases, and passes
//! the in-use checks for its kind and the protected-path check before it
//! goes. Destructive: it deletes folders, so the selection is kept apart
//! from the deletion and tested on its own, and a target folder's build
//! locks are taken again and held while it is deleted.

use super::{ballast, size, targets};
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime};

/// A target folder touched more recently than this may be in a build.
pub const TARGET_IDLE: Duration = Duration::from_secs(30 * 60);
/// A scratch entry touched more recently than this may be in use.
pub const SCRATCH_IDLE: Duration = Duration::from_secs(60 * 60);
/// A plugin cache orphaned more recently may still serve a running session.
pub const ORPHAN_IDLE: Duration = Duration::from_secs(24 * 60 * 60);

/// Debug builds scan this folder in place of `/tmp`, for tests.
pub const TEST_TMP_VAR: &str = "PLAYBOOK_TEST_DISK_TMP";

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

    /// How long a folder of this kind must sit unchanged before it goes.
    fn idle(self) -> Duration {
        match self {
            Kind::Target => TARGET_IDLE,
            Kind::Scratch | Kind::Cache => SCRATCH_IDLE,
            Kind::OrphanedPlugin => Duration::ZERO,
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
            claude_home: super::status::claude_home(home),
            worktrees_root: repo_root.and_then(targets::worktrees_root),
            tmp_dirs: system_tmp_dirs(),
            now: SystemTime::now(),
            process_args: process_args(),
            uid: current_uid(),
        }
    }
}

fn system_tmp_dirs() -> Vec<PathBuf> {
    if cfg!(debug_assertions) {
        if let Some(dir) = std::env::var_os(TEST_TMP_VAR).filter(|v| !v.is_empty()) {
            return vec![PathBuf::from(dir)];
        }
    }
    vec![PathBuf::from("/tmp"), PathBuf::from("/private/tmp")]
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

fn current_uid() -> Option<u32> {
    cfg!(unix).then(crate::service_manager::uid)
}

/// The playbook data that is never removed. It is the list the rm guard
/// protects (ADR 0045, `rm_workspace_guard::roots::protected`), kept here
/// as a copy because the rm guard keeps its list private: the memory store,
/// the databases, plans and handoffs, legacy org config, session state, the
/// effort ledger, the system prompt and the config file. The ballast is
/// added, and each path is listed as given and with links resolved.
pub fn protected(home: &Path) -> Vec<PathBuf> {
    let mut homes = vec![home.to_path_buf()];
    if let Ok(real) = std::fs::canonicalize(home) {
        if real != home {
            homes.push(real);
        }
    }
    let mut paths = Vec::new();
    for home in &homes {
        let root = crate::common::paths::playbook_root_from(home);
        let legacy_db = playbook_data::config::legacy_db_in(&root);
        paths.extend(
            ["memory", "repos", "orgs", "runtime", "effort", "prompts"]
                .iter()
                .map(|name| root.join(name)),
        );
        paths.push(playbook_data::data_dir_from(home));
        paths.push(crate::config::global_config_path(&root));
        for suffix in ["-wal", "-shm"] {
            let mut name = legacy_db.clone().into_os_string();
            name.push(suffix);
            paths.push(PathBuf::from(name));
        }
        paths.push(legacy_db);
        paths.push(ballast::dir(home));
    }
    paths
}

/// Whether removing `path` could touch protected data: it is a protected
/// path, sits inside one, or holds one. Compared without letter case, since
/// macOS disks ignore it by default, and with links resolved where the path
/// exists.
pub fn touches_protected(path: &Path, protected: &[PathBuf]) -> bool {
    let fold = |p: &Path| PathBuf::from(p.to_string_lossy().to_lowercase());
    let mut forms = vec![fold(path)];
    if let Ok(real) = std::fs::canonicalize(path) {
        forms.push(fold(&real));
    }
    protected.iter().map(|p| fold(p)).any(|p| {
        forms
            .iter()
            .any(|form| form.starts_with(&p) || p.starts_with(form))
    })
}

/// One folder to judge, with the base it must sit directly in and a reason
/// to keep it that its source already knows.
struct Source {
    kind: Kind,
    base: PathBuf,
    path: PathBuf,
    keep: Option<String>,
}

/// Picks what to remove. Nothing is deleted here.
pub fn select(env: &Env) -> (Vec<Candidate>, Vec<Kept>) {
    let protected = protected(&env.home);
    let mut kept = Vec::new();
    let mut sized = Vec::new();
    for source in sources(env) {
        match pre_check(env, &protected, &source) {
            Some(reason) => kept.push(Kept {
                path: source.path,
                reason,
            }),
            None => sized.push(source),
        }
    }
    let scans = crate::common::par::map(&sized, crate::common::par::MAX_CONCURRENT, |s| {
        size::scan(&s.path)
    });
    let mut picked = Vec::new();
    for (source, scan) in sized.into_iter().zip(scans) {
        match idle_check(env, source.kind, &scan) {
            Some(reason) => kept.push(Kept {
                path: source.path,
                reason,
            }),
            None => picked.push(Candidate {
                kind: source.kind,
                path: source.path,
                bytes: scan.bytes,
            }),
        }
    }
    picked.sort_by_key(|c| std::cmp::Reverse(c.bytes));
    (picked, kept)
}

/// Every folder the cleanup looks at, from each base.
fn sources(env: &Env) -> Vec<Source> {
    let mut found = Vec::new();
    if let Some(root) = &env.worktrees_root {
        found.extend(targets::target_dirs(root).into_iter().map(|path| {
            let keep = (!is_cargo_target(&path)).then(|| "not a cargo target folder".to_string());
            Source {
                kind: Kind::Target,
                base: root.clone(),
                path,
                keep,
            }
        }));
    }
    for tmp in unique_dirs(&env.tmp_dirs) {
        for path in children(&tmp).into_iter().filter(|p| name_starts(p, "pb-")) {
            let other_owner = env.uid.is_some_and(|uid| owner(&path) != Some(uid));
            found.push(Source {
                kind: Kind::Scratch,
                base: tmp.clone(),
                path,
                keep: other_owner.then(|| "owned by another user".to_string()),
            });
        }
    }
    let cache_bases = [
        env.home.join(".cache/playbook"),
        crate::common::paths::playbook_root_from(&env.home).join("tmp"),
    ];
    for base in cache_bases {
        for path in children(&base) {
            if path != ballast::dir(&env.home) {
                found.push(Source {
                    kind: Kind::Cache,
                    base: base.clone(),
                    path,
                    keep: None,
                });
            }
        }
    }
    let plugin_base = crate::plugin_root::cache_dir(&env.claude_home);
    for path in crate::plugin_root::orphaned_cache_dirs(&env.claude_home) {
        let marked = std::fs::symlink_metadata(path.join(crate::plugin_root::ORPHAN_MARKER))
            .and_then(|m| m.modified())
            .ok();
        let recent =
            marked.is_none_or(|at| env.now.duration_since(at).unwrap_or_default() < ORPHAN_IDLE);
        found.push(Source {
            kind: Kind::OrphanedPlugin,
            base: plugin_base.clone(),
            path,
            keep: recent.then(|| "marked orphaned less than a day ago".to_string()),
        });
    }
    found
}

/// The checks that need no walk of the folder: the reason its source gave,
/// then the path rules, then whether something uses it.
fn pre_check(env: &Env, protected: &[PathBuf], source: &Source) -> Option<String> {
    source
        .keep
        .clone()
        .or_else(|| unsafe_target(&source.base, &source.path, protected))
        .or_else(|| named_by_a_process(&source.path, &env.process_args))
        .or_else(|| {
            (source.kind == Kind::Target)
                .then(|| build_lock_held(&source.path))
                .flatten()
        })
}

/// Keeps a folder that changed within its kind's idle time, or one whose
/// change time could not be read.
fn idle_check(env: &Env, kind: Kind, scan: &size::Scan) -> Option<String> {
    if scan.newest == SystemTime::UNIX_EPOCH {
        return Some("unreadable".to_string());
    }
    let idle_for = env.now.duration_since(scan.newest).unwrap_or_default();
    (idle_for < kind.idle()).then(|| format!("changed {} minutes ago", idle_for.as_secs() / 60))
}

/// A cargo target folder has the `CACHEDIR.TAG` and `.rustc_info.json` cargo
/// writes at its top, and no git checkout inside. A sibling repo named
/// `cargo-something` in the same root fails this test and is kept.
fn is_cargo_target(path: &Path) -> bool {
    let markers = path.join("CACHEDIR.TAG").is_file() && path.join(".rustc_info.json").is_file();
    let holds_git = path.join(".git").exists()
        || children(path)
            .iter()
            .any(|child| child.join(".git").exists());
    markers && !holds_git
}

/// Why `path` must not be removed whatever its age: it cannot be read, is a
/// symlink, is not a direct child of `base`, or touches protected data.
fn unsafe_target(base: &Path, path: &Path, protected: &[PathBuf]) -> Option<String> {
    let Ok(meta) = std::fs::symlink_metadata(path) else {
        return Some("unreadable".to_string());
    };
    if meta.file_type().is_symlink() {
        return Some("a symlink".to_string());
    }
    if path.parent() != Some(base) || path.file_name().is_none() {
        return Some("outside its base folder".to_string());
    }
    touches_protected(path, protected).then(|| "protected playbook data".to_string())
}

/// A running process names `path` on its command line, as rustc does with
/// `--out-dir`. A path under `/private/tmp` also matches when written as
/// `/tmp`, since macOS links one to the other.
fn named_by_a_process(path: &Path, process_args: &str) -> Option<String> {
    let text = path.to_string_lossy();
    let mut forms = vec![text.to_string()];
    if let Some(rest) = text.strip_prefix("/private/") {
        forms.push(format!("/{rest}"));
    }
    process_args
        .lines()
        .any(|line| forms.iter().any(|form| line.contains(form.as_str())))
        .then(|| "in use by a running process".to_string())
}

/// The `.cargo-lock` files of a target folder, per profile and per triple.
fn cargo_locks(target: &Path) -> Vec<PathBuf> {
    let mut locks = Vec::new();
    for profile in children(target) {
        locks.push(profile.join(".cargo-lock"));
        for nested in children(&profile) {
            locks.push(nested.join(".cargo-lock"));
        }
    }
    locks.retain(|lock| lock.is_file());
    locks
}

/// Cargo holds an exclusive lock on `<target>/<profile>/.cargo-lock` for the
/// whole build. A lock this process cannot take means a build is running.
fn build_lock_held(target: &Path) -> Option<String> {
    take_locks(target)
        .is_none()
        .then(|| "a cargo build holds its lock".to_string())
}

/// Takes every build lock of `target`, or `None` when a build holds one.
fn take_locks(target: &Path) -> Option<Vec<std::fs::File>> {
    let mut held = Vec::new();
    for lock in cargo_locks(target) {
        let Ok(file) = std::fs::File::open(&lock) else {
            continue;
        };
        if !try_lock(&file) {
            return None;
        }
        held.push(file);
    }
    Some(held)
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

/// Removes each candidate. A target folder's build locks are taken again
/// and held while it is deleted, so a build that started after the
/// selection keeps its folder and one that starts during the delete waits.
/// Returns what went and what was kept.
pub fn remove(candidates: Vec<Candidate>) -> (Vec<Candidate>, Vec<Kept>) {
    let mut removed = Vec::new();
    let mut kept = Vec::new();
    let process_args = process_args();
    for candidate in candidates {
        let busy = named_by_a_process(&candidate.path, &process_args);
        let locks = if candidate.kind == Kind::Target {
            take_locks(&candidate.path)
        } else {
            Some(Vec::new())
        };
        let (Some(_locks), None) = (locks, busy.clone()) else {
            kept.push(Kept {
                path: candidate.path,
                reason: busy.unwrap_or_else(|| "a cargo build started".to_string()),
            });
            continue;
        };
        let result = match std::fs::symlink_metadata(&candidate.path) {
            Ok(meta) if meta.is_dir() => std::fs::remove_dir_all(&candidate.path),
            Ok(_) => std::fs::remove_file(&candidate.path),
            Err(e) => Err(e),
        };
        match result {
            Ok(()) => removed.push(candidate),
            Err(e) => kept.push(Kept {
                path: candidate.path,
                reason: format!("could not remove: {e}"),
            }),
        }
    }
    (removed, kept)
}

/// The cleanup lock, as far as this run could take it.
pub enum Lock {
    /// Held until dropped.
    Held(std::fs::File),
    /// Another cleanup holds it.
    Busy,
    /// The lock file could not be opened, for example on a full disk.
    Unavailable(std::io::Error),
}

/// Takes the cleanup lock.
pub fn lock(home: &Path) -> Lock {
    let path = crate::common::paths::playbook_root_from(home).join("disk-clean.lock");
    match std::fs::OpenOptions::new()
        .create(true)
        .truncate(false)
        .write(true)
        .open(&path)
    {
        Ok(file) if try_lock(&file) => Lock::Held(file),
        Ok(_) => Lock::Busy,
        Err(e) => Lock::Unavailable(e),
    }
}

#[cfg(unix)]
pub(super) fn try_lock(file: &std::fs::File) -> bool {
    use std::os::unix::io::AsRawFd;
    // SAFETY: the descriptor belongs to `file`, which outlives the call. The
    // lock is released when the file is closed.
    unsafe { libc::flock(file.as_raw_fd(), libc::LOCK_EX | libc::LOCK_NB) == 0 }
}

#[cfg(not(unix))]
pub(super) fn try_lock(_file: &std::fs::File) -> bool {
    false
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
    let mut lines = Vec::new();
    let _lock = if dry_run {
        None
    } else {
        match lock(home) {
            Lock::Busy => {
                return vec!["disk clean: another cleanup is running, nothing to do".to_string()]
            }
            Lock::Unavailable(e) => {
                lines.push(format!(
                    "disk clean: running without the cleanup lock ({e})"
                ));
                None
            }
            Lock::Held(file) => Some(file),
        }
    };
    let env = Env::from_system(home, repo_root);
    let (picked, mut kept) = select(&env);
    let verb = if dry_run { "would remove" } else { "removed" };
    let done = if dry_run {
        picked
    } else {
        let (removed, late) = remove(picked);
        kept.extend(late);
        removed
    };
    for c in &done {
        lines.push(format!(
            "{verb} {:>9}  {:<15}  {}",
            size::gb(c.bytes),
            c.kind.label(),
            c.path.display()
        ));
    }
    for k in &kept {
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
        Fixture { root, env }
    }

    fn write(path: &Path) {
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(path, vec![1u8; 4096]).unwrap();
    }

    /// A cargo target folder with the markers cargo writes.
    fn target(path: &Path) {
        write(&path.join("debug/deps/x.rlib"));
        fs::write(
            path.join("CACHEDIR.TAG"),
            "Signature: 8a477f597d28d172789f06886806bc55",
        )
        .unwrap();
        fs::write(path.join(".rustc_info.json"), "{}").unwrap();
    }

    fn picked(env: &Env) -> Vec<PathBuf> {
        select(env).0.into_iter().map(|c| c.path).collect()
    }

    fn reason(env: &Env, path: &Path) -> Option<String> {
        select(env)
            .1
            .into_iter()
            .find(|k| k.path == path)
            .map(|k| k.reason)
    }

    #[test]
    fn picks_idle_targets_scratch_and_cache() {
        let f = fixture("picks");
        target(&f.root.join("worktrees/cargo-a"));
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
    fn keeps_a_sibling_repo_named_like_a_target() {
        let f = fixture("sibling");
        let repo = f.root.join("worktrees/cargo-nextest");
        write(&repo.join("feature/.git"));
        write(&repo.join("feature/src/main.rs"));
        fs::write(repo.join("CACHEDIR.TAG"), "x").unwrap();
        fs::write(repo.join(".rustc_info.json"), "{}").unwrap();
        let plain = f.root.join("worktrees/cargo-plain");
        write(&plain.join("notes.txt"));
        assert!(picked(&f.env).is_empty());
        assert_eq!(
            reason(&f.env, &repo).as_deref(),
            Some("not a cargo target folder")
        );
        assert_eq!(
            reason(&f.env, &plain).as_deref(),
            Some("not a cargo target folder")
        );
        fs::remove_dir_all(&f.root).unwrap();
    }

    #[test]
    fn keeps_a_target_changed_in_the_last_half_hour() {
        let mut f = fixture("fresh");
        let dir = f.root.join("worktrees/cargo-busy");
        target(&dir);
        f.env.now = SystemTime::now() + Duration::from_secs(10 * 60);
        assert!(picked(&f.env).is_empty());
        assert!(reason(&f.env, &dir).unwrap().starts_with("changed"));
        fs::remove_dir_all(&f.root).unwrap();
    }

    #[test]
    fn keeps_a_target_whose_build_lock_is_held() {
        use std::os::unix::io::AsRawFd;
        let f = fixture("locked");
        let dir = f.root.join("worktrees/cargo-building");
        target(&dir);
        let lock = dir.join("debug/.cargo-lock");
        write(&lock);
        let held = fs::File::open(&lock).unwrap();
        // SAFETY: `held` stays open until it is dropped below.
        assert_eq!(unsafe { libc::flock(held.as_raw_fd(), libc::LOCK_EX) }, 0);
        assert!(picked(&f.env).is_empty());
        assert_eq!(
            reason(&f.env, &dir).as_deref(),
            Some("a cargo build holds its lock")
        );
        drop(held);
        assert_eq!(picked(&f.env), vec![dir]);
        fs::remove_dir_all(&f.root).unwrap();
    }

    #[test]
    fn remove_keeps_a_target_whose_build_started_after_selection() {
        use std::os::unix::io::AsRawFd;
        let f = fixture("late-lock");
        let dir = f.root.join("worktrees/cargo-late");
        target(&dir);
        let (chosen, _) = select(&f.env);
        assert_eq!(chosen.len(), 1);
        let lock = dir.join("debug/.cargo-lock");
        write(&lock);
        let held = fs::File::open(&lock).unwrap();
        // SAFETY: `held` stays open until the end of the test.
        assert_eq!(unsafe { libc::flock(held.as_raw_fd(), libc::LOCK_EX) }, 0);
        let (removed, kept) = remove(chosen);
        assert!(removed.is_empty());
        assert_eq!(kept[0].reason, "a cargo build started");
        assert!(dir.exists());
        drop(held);
        fs::remove_dir_all(&f.root).unwrap();
    }

    #[test]
    fn keeps_a_folder_a_running_process_names() {
        let mut f = fixture("ps");
        let dir = f.root.join("worktrees/cargo-rustc");
        target(&dir);
        f.env.process_args = format!("rustc --out-dir {}/debug/deps src/lib.rs\n", dir.display());
        assert!(picked(&f.env).is_empty());
        assert_eq!(
            reason(&f.env, &dir).as_deref(),
            Some("in use by a running process")
        );
        fs::remove_dir_all(&f.root).unwrap();
    }

    #[test]
    fn a_tmp_path_matches_its_private_form() {
        let args = "cargo build --target-dir /tmp/pb-x/target\n";
        assert!(named_by_a_process(Path::new("/private/tmp/pb-x"), args).is_some());
        assert!(named_by_a_process(Path::new("/private/tmp/pb-y"), args).is_none());
    }

    #[test]
    fn keeps_scratch_owned_by_another_user() {
        let mut f = fixture("owner");
        let dir = f.root.join("tmp/pb-theirs");
        write(&dir.join("file"));
        f.env.uid = current_uid().map(|uid| uid + 1);
        assert!(picked(&f.env).is_empty());
        assert_eq!(
            reason(&f.env, &dir).as_deref(),
            Some("owned by another user")
        );
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
        assert!(picked(&f.env).is_empty());
        assert!(outside.join("keep").exists());
        fs::remove_dir_all(&f.root).unwrap();
    }

    #[test]
    fn a_home_inside_the_tmp_folder_is_protected_data() {
        let mut f = fixture("home-in-tmp");
        let home = f.root.join("tmp/pb-home");
        write(&home.join(".config/playbook/memory/fact.md"));
        f.env.home = home.clone();
        assert!(picked(&f.env).is_empty());
        assert_eq!(
            reason(&f.env, &home).as_deref(),
            Some("protected playbook data")
        );
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
            "/Users/me/.config/playbook/playbook.db-wal",
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
        let first = lock(&f.env.home);
        assert!(matches!(first, Lock::Held(_)));
        assert!(matches!(lock(&f.env.home), Lock::Busy));
        drop(first);
        assert!(matches!(lock(&f.env.home), Lock::Held(_)));
        let missing = f.root.join("no-home");
        assert!(matches!(lock(&missing), Lock::Unavailable(_)));
        fs::remove_dir_all(&f.root).unwrap();
    }

    #[test]
    fn remove_deletes_only_what_was_picked_and_never_follows_links() {
        let f = fixture("remove");
        let gone = f.root.join("worktrees/cargo-gone");
        target(&gone);
        let outside = f.root.join("outside/keep");
        write(&outside);
        std::os::unix::fs::symlink(f.root.join("outside"), gone.join("link")).unwrap();
        write(&f.root.join("worktrees/playbook/keep/src.rs"));
        let (chosen, _) = select(&f.env);
        let (removed, kept) = remove(chosen);
        assert_eq!(removed.len(), 1);
        assert!(kept.is_empty(), "{kept:?}");
        assert!(!gone.exists());
        assert!(outside.exists());
        assert!(f.root.join("worktrees/playbook/keep/src.rs").exists());
        fs::remove_dir_all(&f.root).unwrap();
    }
}
