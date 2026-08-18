// SPDX-FileCopyrightText: 2026 Igor Santos
// SPDX-License-Identifier: MIT

//! Behavioural tests for the launcher internals ported in WU-16.
//!
//! These modules delete files, so every scenario asserts what SURVIVES as well
//! as what goes. A retention test that only counted deletions would pass while
//! deleting everything.

use playbook::cc::{bust_cache, project_slug, retention};
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;

static COUNTER: AtomicU64 = AtomicU64::new(0);

/// An isolated HOME, so a test can never reach the real `~/.claude`.
struct Sandbox {
    home: PathBuf,
}

impl Sandbox {
    fn new(tag: &str) -> Self {
        let n = COUNTER.fetch_add(1, Ordering::Relaxed);
        let home =
            std::env::temp_dir().join(format!("playbook-cc-{tag}-{}-{n}", std::process::id()));
        fs::create_dir_all(&home).expect("sandbox home");
        Self { home }
    }

    fn claude(&self) -> PathBuf {
        self.home.join(".claude")
    }

    fn touch(&self, rel: &str) -> PathBuf {
        let path = self.claude().join(rel);
        fs::create_dir_all(path.parent().expect("parent")).expect("mkdir");
        fs::write(&path, "x").expect("write");
        path
    }

    fn mkdir(&self, rel: &str) -> PathBuf {
        let path = self.claude().join(rel);
        fs::create_dir_all(&path).expect("mkdir");
        path
    }
}

fn exists(p: &Path) -> bool {
    p.exists()
}

/// Sets mtime by writing then backdating through a filetime-free trick: files
/// are created oldest first with a real sleep, since the port ranks on mtime
/// and a fabricated ordering would not exercise that.
fn age_apart(paths: &[PathBuf]) {
    for p in paths {
        fs::write(p, "x").expect("write");
        std::thread::sleep(Duration::from_millis(10));
    }
}

mod retention_tests {
    use super::*;

    /// Serialises the env mutations, since CCD_KEEP and HOME are process-wide.
    fn with_env<T>(home: &Path, keep: Option<&str>, f: impl FnOnce() -> T) -> T {
        static LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());
        let _guard = LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let prev_home = std::env::var_os("HOME");
        let prev_keep = std::env::var_os("CCD_KEEP");
        std::env::set_var("HOME", home);
        match keep {
            Some(k) => std::env::set_var("CCD_KEEP", k),
            None => std::env::remove_var("CCD_KEEP"),
        }
        let out = f();
        match prev_home {
            Some(v) => std::env::set_var("HOME", v),
            None => std::env::remove_var("HOME"),
        }
        match prev_keep {
            Some(v) => std::env::set_var("CCD_KEEP", v),
            None => std::env::remove_var("CCD_KEEP"),
        }
        out
    }

    /// Three transcripts with distinct mtimes, plus the sidecar and runtime
    /// state retention is supposed to remove alongside each one.
    fn seed(sb: &Sandbox, cwd: &str, count: usize) -> Vec<PathBuf> {
        let dir = format!("projects/{}", project_slug(cwd));
        let mut files = Vec::new();
        for i in 0..count {
            let sid = format!("sess{i}");
            files.push(sb.touch(&format!("{dir}/{sid}.jsonl")));
            sb.mkdir(&format!("{dir}/{sid}"));
            sb.mkdir(&format!("runtime/{sid}"));
        }
        age_apart(&files);
        files
    }

    #[test]
    fn keeps_the_newest_and_deletes_the_rest() {
        let sb = Sandbox::new("keep2");
        let cwd = "/tmp/proj";
        let files = seed(&sb, cwd, 3);
        with_env(&sb.home, Some("2"), || retention::prune(cwd));

        // seed() ages oldest first, so the last written is newest.
        assert!(exists(&files[2]), "newest must survive");
        assert!(exists(&files[1]), "second newest must survive");
        assert!(!exists(&files[0]), "oldest must go");

        let dir = sb.claude().join("projects").join(project_slug(cwd));
        assert!(
            !exists(&dir.join("sess0")),
            "the sidecar directory goes with its transcript"
        );
        assert!(
            !exists(&sb.claude().join("runtime").join("sess0")),
            "runtime state goes with its transcript"
        );
        assert!(
            exists(&dir.join("sess2")),
            "a surviving transcript keeps its sidecar"
        );
    }

    /// The floor exists because fork and clean resume both read the
    /// second-newest transcript as their parent.
    #[test]
    fn a_keep_of_one_is_raised_to_the_floor_of_two() {
        let sb = Sandbox::new("floor");
        let cwd = "/tmp/proj";
        let files = seed(&sb, cwd, 3);
        with_env(&sb.home, Some("1"), || retention::prune(cwd));

        assert!(exists(&files[2]), "newest survives");
        assert!(exists(&files[1]), "the floor keeps the second newest");
        assert!(!exists(&files[0]), "the oldest still goes");
    }

    #[test]
    fn zero_disables_retention_entirely() {
        let sb = Sandbox::new("zero");
        let cwd = "/tmp/proj";
        let files = seed(&sb, cwd, 3);
        with_env(&sb.home, Some("0"), || retention::prune(cwd));
        for f in &files {
            assert!(exists(f), "nothing may be deleted when disabled");
        }
    }

    #[test]
    fn fewer_transcripts_than_keep_deletes_nothing() {
        let sb = Sandbox::new("under");
        let cwd = "/tmp/proj";
        let files = seed(&sb, cwd, 2);
        with_env(&sb.home, Some("5"), || retention::prune(cwd));
        for f in &files {
            assert!(exists(f), "under the limit nothing is pruned");
        }
    }

    #[test]
    fn a_missing_project_directory_is_a_no_op() {
        let sb = Sandbox::new("missing");
        with_env(&sb.home, Some("2"), || retention::prune("/tmp/never-used"));
    }

    /// The slug has to match the shell's `${PWD//[^a-zA-Z0-9]/-}` exactly, or
    /// the binary would prune a different directory than the launcher wrote.
    #[test]
    fn the_project_slug_matches_the_shell_expansion() {
        assert_eq!(project_slug("/Users/x/Work_dir.1"), "-Users-x-Work-dir-1");
        assert_eq!(project_slug("abc123"), "abc123");
        assert_eq!(project_slug("/a b/c"), "-a-b-c");
    }

    /// `$PWD` wins over `current_dir()`, which resolves symlinks. macOS ships
    /// `/tmp` and `/var` as symlinks into `/private`, so resolving would build
    /// a slug the launcher never wrote to and prune nothing. Found by the
    /// differential: the shell deleted a transcript and the port did not.
    #[test]
    fn the_logical_path_is_preferred_over_the_resolved_one() {
        static LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());
        let _guard = LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let prev = std::env::var_os("PWD");
        std::env::set_var("PWD", "/tmp/logical-path");
        assert_eq!(playbook::cc::logical_cwd(), "/tmp/logical-path");
        match prev {
            Some(v) => std::env::set_var("PWD", v),
            None => std::env::remove_var("PWD"),
        }
    }
}

mod bust_cache_tests {
    use super::*;

    fn with_home<T>(home: &Path, f: impl FnOnce() -> T) -> T {
        static LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());
        let _guard = LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let prev = std::env::var_os("HOME");
        std::env::set_var("HOME", home);
        let out = f();
        match prev {
            Some(v) => std::env::set_var("HOME", v),
            None => std::env::remove_var("HOME"),
        }
        out
    }

    #[test]
    fn clears_the_four_caches_and_leaves_everything_else() {
        let sb = Sandbox::new("bust");
        let snapshot = sb.touch("shell-snapshots/snapshot-abc.sh");
        let other_snapshot = sb.touch("shell-snapshots/keep-me.txt");
        let hash = sb.touch("runtime/sess1/config-hash");
        let other_runtime = sb.touch("runtime/sess1/telemetry.jsonl");
        let catalog = sb.touch("plugins/plugin-catalog-cache.json");
        let other_plugin = sb.touch("plugins/keep-me.json");
        let backup = sb.touch("backups/install-1/file.txt");
        let settings = sb.touch("settings.json");

        with_home(&sb.home, bust_cache::bust);

        assert!(!exists(&snapshot), "stale snapshot must go");
        assert!(!exists(&hash), "config-hash must go, nested one level down");
        assert!(!exists(&catalog), "plugin catalog cache must go");

        // A POPULATED backup directory survives. The shell used `find -delete`,
        // which cannot remove a non-empty directory, so using remove_dir_all
        // here would destroy backups the shell preserved. Caught by the
        // differential; both spellings clear the cache, only one keeps the data.
        assert!(exists(&backup), "a non-empty backup directory must survive");

        // The other half of the contract: nothing beyond those caches.
        assert!(exists(&other_snapshot), "a non-snapshot file stays");
        assert!(exists(&other_runtime), "other runtime state stays");
        assert!(exists(&other_plugin), "other plugin files stay");
        assert!(exists(&settings), "settings.json is never touched");
    }

    #[test]
    fn an_empty_backup_directory_is_removed() {
        let sb = Sandbox::new("bust-empty-dir");
        let empty = sb.mkdir("backups/empty-one");
        with_home(&sb.home, bust_cache::bust);
        assert!(
            !exists(&empty),
            "an empty directory is what find -delete can remove"
        );
    }

    #[test]
    fn missing_directories_are_a_no_op() {
        let sb = Sandbox::new("bust-empty");
        fs::create_dir_all(sb.claude()).expect("mkdir");
        with_home(&sb.home, bust_cache::bust);
    }
}
