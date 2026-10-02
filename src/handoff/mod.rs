// SPDX-FileCopyrightText: 2026 Igor Santos
// SPDX-License-Identifier: MIT

//! Session handoffs: the markdown one session leaves for the next. The write
//! side (`playbook handoff save`) and the read side (the SessionStart hook)
//! both key a handoff through `is_handoff_for`, so they can never disagree.
//!
//! Files live in `runtime_root()/handoff/<slug>-<epoch>-<pid>.md`. A handoff
//! the hook has loaded moves to `handoff/used/` for 14 days, so `show` can
//! still print it after the session start consumed it.

use crate::common::atomic::ensure_private_dir;
use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

/// Age past which a handoff is stale: dropped without injecting, and pruned
/// from `used/`.
pub const MAX_AGE_DAYS: u64 = 14;

/// Most handoffs one SessionStart injects, so a busy directory cannot flood
/// the context.
pub const MAX_INJECTED: usize = 3;

const USED_DIR: &str = "used";
const SECS_PER_DAY: u64 = 86_400;
const OTHER_DIRS_SHOWN: usize = 5;

pub fn root() -> PathBuf {
    crate::common::paths::runtime_root().join("handoff")
}

/// Longest key kept. The file name adds an epoch, a pid, and `.md`, and a
/// file name tops out at 255 bytes, so a very deep path keeps its tail, the
/// most specific part. Both sides go through `key`, so they still agree.
const MAX_KEY_LEN: usize = 150;

fn key(path: &str) -> String {
    let slug = crate::cc::project_slug(path);
    if slug.len() <= MAX_KEY_LEN {
        return slug;
    }
    slug[slug.len() - MAX_KEY_LEN..].to_string()
}

/// The key for the directory this process runs in (`$PWD` first, like Claude Code).
pub fn current_slug() -> String {
    key(&crate::cc::logical_cwd())
}

/// The key for `--dir`: the path as given made absolute, never canonicalized,
/// so it matches the `$PWD` a session in that directory carries.
pub fn slug_for_dir(dir: &str) -> Result<String, String> {
    let given = Path::new(dir);
    let absolute = if given.is_absolute() {
        given.to_path_buf()
    } else {
        std::env::current_dir()
            .map_err(|e| format!("--dir {dir}: {e}"))?
            .join(given)
    };
    let tidy: PathBuf = absolute.components().collect();
    if !tidy.is_dir() {
        return Err(format!("--dir {dir} is not a directory"));
    }
    Ok(key(&tidy.to_string_lossy()))
}

/// `<slug>-<epoch>-<pid>.md`. The rest after the slug must be two digit runs,
/// so a sibling or child directory whose slug merely starts with ours never
/// matches.
pub fn is_handoff_for(name: &str, slug: &str) -> bool {
    let Some(rest) = name
        .strip_prefix(slug)
        .and_then(|r| r.strip_prefix('-'))
        .and_then(|r| r.strip_suffix(".md"))
    else {
        return false;
    };
    let mut parts = rest.split('-');
    let digits =
        |p: Option<&str>| p.is_some_and(|s| !s.is_empty() && s.bytes().all(|b| b.is_ascii_digit()));
    digits(parts.next()) && digits(parts.next()) && parts.next().is_none()
}

#[derive(Debug, Clone)]
pub struct Entry {
    pub path: PathBuf,
    pub modified: SystemTime,
}

/// Regular `.md` files in `dir` whose name passes `keep`, freshest first.
fn list(dir: &Path, keep: impl Fn(&str) -> bool) -> Vec<Entry> {
    let Ok(entries) = fs::read_dir(dir) else {
        return Vec::new();
    };
    let mut out: Vec<Entry> = entries
        .flatten()
        .filter_map(|entry| {
            let name = entry.file_name().into_string().ok()?;
            if !name.ends_with(".md") || !keep(&name) {
                return None;
            }
            let meta = entry.metadata().ok()?;
            if !meta.is_file() {
                return None;
            }
            Some(Entry {
                path: entry.path(),
                modified: meta.modified().ok()?,
            })
        })
        .collect();
    out.sort_by_key(|e| std::cmp::Reverse(e.modified));
    out
}

/// Handoffs for `slug` not yet loaded by a session start, freshest first.
pub fn unread(dir: &Path, slug: &str) -> Vec<Entry> {
    list(dir, |name| is_handoff_for(name, slug))
}

pub fn freshest_mtime(dir: &Path, slug: &str) -> Option<SystemTime> {
    unread(dir, slug).first().map(|e| e.modified)
}

fn loaded(dir: &Path, slug: &str) -> Vec<Entry> {
    list(&dir.join(USED_DIR), |name| is_handoff_for(name, slug))
}

fn cutoff(now: SystemTime) -> SystemTime {
    now.checked_sub(Duration::from_secs(MAX_AGE_DAYS * SECS_PER_DAY))
        .unwrap_or(UNIX_EPOCH)
}

#[cfg(unix)]
fn write_private(path: &Path, text: &str) -> std::io::Result<()> {
    use std::os::unix::fs::OpenOptionsExt;
    let mut file = fs::OpenOptions::new()
        .write(true)
        .create(true)
        .truncate(true)
        .mode(0o600)
        .open(path)?;
    file.write_all(text.as_bytes())?;
    file.sync_all()
}

#[cfg(not(unix))]
fn write_private(path: &Path, text: &str) -> std::io::Result<()> {
    let mut file = fs::File::create(path)?;
    file.write_all(text.as_bytes())?;
    file.sync_all()
}

/// Writes `text` for `slug`. A temp file in the same directory plus a rename
/// keeps a reader from ever seeing half a handoff. Never overwrites: a name
/// already taken moves the epoch forward by one second.
pub fn save_in(
    dir: &Path,
    slug: &str,
    text: &str,
    epoch: u64,
    pid: u32,
) -> Result<PathBuf, String> {
    if text.trim().is_empty() {
        return Err(
            "empty handoff: pipe the handoff markdown to this command on stdin".to_string(),
        );
    }
    if slug.is_empty() {
        return Err("could not work out which directory this handoff belongs to".to_string());
    }
    ensure_private_dir(dir).map_err(|e| format!("could not create {}: {e}", dir.display()))?;
    let mut stamp = epoch;
    let target = loop {
        let candidate = dir.join(format!("{slug}-{stamp}-{pid}.md"));
        if !candidate.exists() {
            break candidate;
        }
        stamp += 1;
    };
    let tmp = dir.join(format!(".{pid}.tmp"));
    write_private(&tmp, text).map_err(|e| format!("could not write {}: {e}", tmp.display()))?;
    fs::rename(&tmp, &target).map_err(|e| {
        let _ = fs::remove_file(&tmp);
        format!("could not save {}: {e}", target.display())
    })?;
    Ok(target)
}

pub struct Taken {
    pub contents: Vec<String>,
    pub total: usize,
}

fn prune_loaded(dir: &Path, now: SystemTime) {
    let limit = cutoff(now);
    for entry in list(&dir.join(USED_DIR), |_| true) {
        if entry.modified < limit {
            let _ = fs::remove_file(&entry.path);
        }
    }
}

/// Moves a loaded handoff to `used/`. If the move fails the file is removed
/// instead, so a stuck file can never re-inject on every session start.
fn archive(dir: &Path, path: &Path) {
    let used = dir.join(USED_DIR);
    let moved = ensure_private_dir(&used).is_ok()
        && path
            .file_name()
            .is_some_and(|name| fs::rename(path, used.join(name)).is_ok());
    if !moved {
        let _ = fs::remove_file(path);
    }
}

/// The read side of a session start: up to `MAX_INJECTED` fresh handoffs for
/// `slug`. Loaded ones move to `used/`; stale and over-cap ones are removed.
/// Never panics, and any failure just yields fewer handoffs.
pub fn take_in(dir: &Path, slug: &str, now: SystemTime) -> Taken {
    prune_loaded(dir, now);
    let found = unread(dir, slug);
    let total = found.len();
    let limit = cutoff(now);
    let mut contents = Vec::new();
    for entry in &found {
        let mut keep = false;
        if contents.len() < MAX_INJECTED && entry.modified >= limit {
            if let Ok(text) = fs::read_to_string(&entry.path) {
                if !text.is_empty() {
                    contents.push(text);
                    keep = true;
                }
            }
        }
        if keep {
            archive(dir, &entry.path);
        } else {
            let _ = fs::remove_file(&entry.path);
        }
    }
    Taken { contents, total }
}

fn age(now: SystemTime, then: SystemTime) -> String {
    let secs = now.duration_since(then).unwrap_or_default().as_secs();
    match secs {
        0..=59 => "just now".to_string(),
        60..=3599 => format!("{}m ago", secs / 60),
        3600..=86_399 => format!("{}h ago", secs / 3600),
        _ => format!("{}d ago", secs / SECS_PER_DAY),
    }
}

/// The topic line: the first markdown heading, else the first non-empty line.
fn heading(path: &Path) -> String {
    let text = fs::read_to_string(path).unwrap_or_default();
    let line = text
        .lines()
        .find(|l| l.trim_start().starts_with('#'))
        .or_else(|| text.lines().find(|l| !l.trim().is_empty()))
        .unwrap_or("(empty)");
    line.trim().trim_start_matches('#').trim().to_string()
}

fn render(entry: &Entry, label: &str, now: SystemTime) -> String {
    let text = fs::read_to_string(&entry.path).unwrap_or_default();
    format!(
        "{label}, saved {} ({}):\n\n{}",
        age(now, entry.modified),
        entry.path.display(),
        text.trim_end()
    )
}

/// What `playbook handoff show` prints. Never deletes or moves anything.
pub fn show_in(dir: &Path, slug: &str, all: bool, now: SystemTime) -> String {
    let waiting = unread(dir, slug);
    if !waiting.is_empty() {
        let shown = if all { waiting.len() } else { 1 };
        let mut parts: Vec<String> = waiting
            .iter()
            .take(shown)
            .map(|e| render(e, "Handoff for this directory", now))
            .collect();
        if waiting.len() > shown {
            parts.push(format!(
                "({} more waiting for this directory; run `playbook handoff show --all` to print them.)",
                waiting.len() - shown
            ));
        }
        return parts.join("\n\n---\n\n");
    }
    if let Some(entry) = loaded(dir, slug).first() {
        return render(
            entry,
            "Handoff for this directory, already loaded by the last session start",
            now,
        );
    }
    let mut out = String::from("No handoff saved for this directory.");
    let others = list(dir, |name| !is_handoff_for(name, slug));
    if !others.is_empty() {
        out.push_str(
            "\n\nHandoffs saved for other directories (a handoff belongs to the directory its session ran in):",
        );
        for entry in others.iter().take(OTHER_DIRS_SHOWN) {
            out.push_str(&format!(
                "\n- {}: {} ({})",
                age(now, entry.modified),
                heading(&entry.path),
                entry.path.display()
            ));
        }
    }
    out
}

fn now_epoch_secs() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

/// CLI entry for `handoff save`.
pub fn run_save(dir: Option<&str>, text: &str) -> Result<String, String> {
    let slug = match dir {
        Some(d) => slug_for_dir(d)?,
        None => current_slug(),
    };
    let path = save_in(&root(), &slug, text, now_epoch_secs(), std::process::id())?;
    Ok(format!("handoff saved: {}", path.display()))
}

/// CLI entry for `handoff show`.
pub fn run_show(dir: Option<&str>, all: bool) -> Result<String, String> {
    let slug = match dir {
        Some(d) => slug_for_dir(d)?,
        None => current_slug(),
    };
    Ok(show_in(&root(), &slug, all, SystemTime::now()))
}

const LOG_FILE: &str = "session-start.log";
const LOG_TRIM_AT: usize = 300;
const LOG_KEEP: usize = 200;
const STATUS_ROWS: usize = 5;

pub fn log_path() -> PathBuf {
    crate::common::paths::runtime_root().join(LOG_FILE)
}

/// `2026-10-02T10:40:25Z` from epoch seconds (UTC, no date crate).
fn iso_utc(secs: u64) -> String {
    let days = (secs / SECS_PER_DAY) as i64;
    let rem = secs % SECS_PER_DAY;
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1_460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = doy - (153 * mp + 2) / 5 + 1;
    let month = if mp < 10 { mp + 3 } else { mp - 9 };
    let year = yoe + era * 400 + i64::from(month <= 2);
    format!(
        "{year:04}-{month:02}-{day:02}T{:02}:{:02}:{:02}Z",
        rem / 3600,
        rem % 3600 / 60,
        rem % 60
    )
}

fn field(value: &str) -> String {
    let clean: String = value
        .chars()
        .map(|c| if c.is_control() { ' ' } else { c })
        .collect();
    let clean = clean.trim();
    if clean.is_empty() {
        "-".to_string()
    } else {
        clean.to_string()
    }
}

/// Appends one line per SessionStart: time, source, session id, directory
/// key, handoffs injected. Best-effort and owner-only; the file is cut back
/// to the last 200 lines once it passes 300. Never fails the hook.
pub fn log_start_in(
    path: &Path,
    now_secs: u64,
    source: &str,
    session_id: &str,
    slug: &str,
    injected: usize,
) {
    let line = format!(
        "{}\t{}\t{}\t{}\t{injected}\n",
        iso_utc(now_secs),
        field(source),
        field(session_id),
        field(slug)
    );
    let Some(parent) = path.parent() else { return };
    if fs::create_dir_all(parent).is_err() {
        return;
    }
    let appended = open_log_for_append(path).and_then(|mut f| f.write_all(line.as_bytes()));
    if appended.is_err() {
        return;
    }
    let Ok(text) = fs::read_to_string(path) else {
        return;
    };
    let lines: Vec<&str> = text.lines().collect();
    if lines.len() > LOG_TRIM_AT {
        let kept = lines[lines.len() - LOG_KEEP..].join("\n") + "\n";
        let tmp = path.with_extension("log.tmp");
        if write_private(&tmp, &kept).is_ok() && fs::rename(&tmp, path).is_err() {
            let _ = fs::remove_file(&tmp);
        }
    }
}

#[cfg(unix)]
fn open_log_for_append(path: &Path) -> std::io::Result<fs::File> {
    use std::os::unix::fs::OpenOptionsExt;
    fs::OpenOptions::new()
        .create(true)
        .append(true)
        .mode(0o600)
        .open(path)
}

#[cfg(not(unix))]
fn open_log_for_append(path: &Path) -> std::io::Result<fs::File> {
    fs::OpenOptions::new().create(true).append(true).open(path)
}

/// Called by the SessionStart hook; see `log_start_in`.
pub fn log_start(source: &str, session_id: &str, slug: &str, injected: usize) {
    log_start_in(
        &log_path(),
        now_epoch_secs(),
        source,
        session_id,
        slug,
        injected,
    );
}

/// What `playbook handoff status` prints: the last few SessionStart events
/// and how many handoffs wait or were loaded for this directory.
pub fn status_in(dir: &Path, log: &Path, slug: &str) -> String {
    let text = fs::read_to_string(log).unwrap_or_default();
    let rows: Vec<Vec<&str>> = text
        .lines()
        .map(|l| l.split('\t').collect::<Vec<_>>())
        .filter(|f| f.len() >= 5)
        .collect();
    let mut out = String::new();
    if rows.is_empty() {
        out.push_str("No SessionStart has been logged yet.\n");
    } else {
        out.push_str("Last SessionStart events (newest last):\n");
        out.push_str(&format!(
            "  {:<21} {:<8} {}\n",
            "time (UTC)", "source", "handoffs injected"
        ));
        for f in rows.iter().skip(rows.len().saturating_sub(STATUS_ROWS)) {
            out.push_str(&format!("  {:<21} {:<8} {}\n", f[0], f[1], f[4]));
        }
    }
    out.push_str(&format!(
        "\nThis directory: {} unread, {} already loaded.\n",
        unread(dir, slug).len(),
        loaded(dir, slug).len()
    ));
    out.push_str(
        "If you ran /clear and no `clear` row appears above, Claude Code did not run the hook: run /playbook:session-start to load the handoff on demand.",
    );
    out
}

/// CLI entry for `handoff status`.
pub fn run_status() -> String {
    status_in(&root(), &log_path(), &current_slug())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::common::test_support::scratch_dir;

    const SLUG: &str = "-Users-me-repo";

    fn day(n: u64) -> Duration {
        Duration::from_secs(n * SECS_PER_DAY)
    }

    fn backdate(path: &Path, age_days: u64) {
        let when = SystemTime::now() - day(age_days);
        let file = fs::File::options().write(true).open(path).unwrap();
        file.set_modified(when).unwrap();
    }

    #[test]
    fn a_very_deep_path_still_gives_a_key_that_fits_a_file_name() {
        let deep = format!("/{}", "segment/".repeat(60));

        let k = key(&deep);

        assert_eq!(k.len(), MAX_KEY_LEN);
        assert!(format!("{k}-1790000000-99999.md").len() < 255);
        assert_eq!(key(&deep), k, "the key is stable");
        assert!(is_handoff_for(&format!("{k}-1790000000-99999.md"), &k));
    }

    #[test]
    fn only_the_exact_directory_key_matches() {
        assert!(is_handoff_for("-Users-me-repo-1790000000-42.md", SLUG));
        assert!(!is_handoff_for("-Users-me-repo-sub-1790000000-42.md", SLUG));
        assert!(!is_handoff_for("-Users-me-repo2-1790000000-42.md", SLUG));
        assert!(!is_handoff_for("-Users-me-repo-1790000000-test.md", SLUG));
        assert!(!is_handoff_for("-Users-me-repo-1790000000-42.txt", SLUG));
    }

    #[test]
    fn two_saves_in_one_second_do_not_overwrite_each_other() {
        let dir = scratch_dir("handoff-two");
        let a = save_in(&dir, SLUG, "first", 100, 7).unwrap();
        let b = save_in(&dir, SLUG, "second", 100, 7).unwrap();

        assert_ne!(a, b);
        assert_eq!(fs::read_to_string(a).unwrap(), "first");
        assert_eq!(fs::read_to_string(b).unwrap(), "second");
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn an_empty_handoff_is_refused_and_writes_nothing() {
        let dir = scratch_dir("handoff-empty");
        let err = save_in(&dir, SLUG, "  \n", 100, 7).unwrap_err();

        assert!(err.contains("empty handoff"), "{err}");
        assert_eq!(unread(&dir, SLUG).len(), 0);
        let _ = fs::remove_dir_all(dir);
    }

    #[cfg(unix)]
    #[test]
    fn the_directory_is_private_and_so_is_the_file() {
        use std::os::unix::fs::PermissionsExt;
        let root = scratch_dir("handoff-mode");
        let dir = root.join("handoff");
        let path = save_in(&dir, SLUG, "# x\n", 100, 7).unwrap();

        let mode = |p: &Path| fs::metadata(p).unwrap().permissions().mode() & 0o777;
        assert_eq!(mode(&dir), 0o700);
        assert_eq!(mode(&path), 0o600);
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn a_loaded_handoff_moves_to_used_and_show_still_prints_it() {
        let dir = scratch_dir("handoff-take");
        save_in(&dir, SLUG, "# topic\nNEXT-STEP\n", 100, 7).unwrap();

        let taken = take_in(&dir, SLUG, SystemTime::now());

        assert_eq!(taken.contents.len(), 1);
        assert!(taken.contents[0].contains("NEXT-STEP"));
        assert_eq!(unread(&dir, SLUG).len(), 0);
        let shown = show_in(&dir, SLUG, false, SystemTime::now());
        assert!(
            shown.contains("already loaded by the last session start"),
            "{shown}"
        );
        assert!(shown.contains("NEXT-STEP"), "{shown}");
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn a_stale_handoff_is_removed_not_injected_or_kept() {
        let dir = scratch_dir("handoff-stale");
        let path = save_in(&dir, SLUG, "# old\n", 100, 7).unwrap();
        backdate(&path, MAX_AGE_DAYS + 1);

        let taken = take_in(&dir, SLUG, SystemTime::now());

        assert!(taken.contents.is_empty());
        assert!(!path.exists());
        assert!(loaded(&dir, SLUG).is_empty());
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn loaded_handoffs_are_pruned_after_the_age_limit() {
        let dir = scratch_dir("handoff-prune");
        save_in(&dir, SLUG, "# a\n", 100, 7).unwrap();
        take_in(&dir, SLUG, SystemTime::now());
        let kept = loaded(&dir, SLUG).remove(0).path;
        backdate(&kept, MAX_AGE_DAYS + 1);

        take_in(&dir, SLUG, SystemTime::now());

        assert!(!kept.exists());
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn the_cap_injects_the_freshest_and_removes_the_rest() {
        let dir = scratch_dir("handoff-cap");
        for (i, pid) in [1u32, 2, 3, 4].iter().enumerate() {
            let p = save_in(&dir, SLUG, &format!("# h{i}\n"), 100 + i as u64, *pid).unwrap();
            backdate(&p, 4 - i as u64);
        }

        let taken = take_in(&dir, SLUG, SystemTime::now());

        assert_eq!((taken.contents.len(), taken.total), (MAX_INJECTED, 4));
        assert!(taken.contents[0].contains("h3"));
        assert!(unread(&dir, SLUG).is_empty());
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn show_lists_handoffs_saved_for_other_directories() {
        let dir = scratch_dir("handoff-others");
        save_in(&dir, "-Users-me-other", "# Other topic\n", 100, 7).unwrap();

        let shown = show_in(&dir, SLUG, false, SystemTime::now());

        assert!(
            shown.starts_with("No handoff saved for this directory."),
            "{shown}"
        );
        assert!(shown.contains("Other topic"), "{shown}");
        assert_eq!(
            unread(&dir, "-Users-me-other").len(),
            1,
            "show never consumes"
        );
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn show_all_prints_every_waiting_handoff_and_default_notes_the_rest() {
        let dir = scratch_dir("handoff-all");
        save_in(&dir, SLUG, "# one\n", 100, 1).unwrap();
        save_in(&dir, SLUG, "# two\n", 100, 2).unwrap();

        let one = show_in(&dir, SLUG, false, SystemTime::now());
        let both = show_in(&dir, SLUG, true, SystemTime::now());

        assert!(one.contains("1 more waiting"), "{one}");
        assert!(both.contains("# one") && both.contains("# two"), "{both}");
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn iso_utc_matches_known_instants() {
        assert_eq!(iso_utc(0), "1970-01-01T00:00:00Z");
        assert_eq!(iso_utc(1_788_252_682), "2026-09-01T08:51:22Z");
        assert_eq!(iso_utc(951_782_400), "2000-02-29T00:00:00Z");
    }

    #[test]
    fn a_log_line_has_five_tab_separated_fields() {
        let dir = scratch_dir("handoff-log-line");
        let log = dir.join("session-start.log");

        log_start_in(&log, 0, "clear", "sess\t1", SLUG, 2);

        assert_eq!(
            fs::read_to_string(&log).unwrap(),
            "1970-01-01T00:00:00Z\tclear\tsess 1\t-Users-me-repo\t2\n"
        );
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn the_log_is_cut_back_to_the_last_200_lines_past_300() {
        let dir = scratch_dir("handoff-log-bound");
        let log = dir.join("session-start.log");

        for n in 0..=LOG_TRIM_AT {
            log_start_in(&log, n as u64, "startup", "s", SLUG, 0);
        }

        let lines = fs::read_to_string(&log).unwrap().lines().count();
        assert_eq!(lines, LOG_KEEP);
        let _ = fs::remove_dir_all(dir);
    }

    #[cfg(unix)]
    #[test]
    fn a_read_only_runtime_dir_never_breaks_the_log_call() {
        use std::os::unix::fs::PermissionsExt;
        let dir = scratch_dir("handoff-log-ro");
        fs::create_dir_all(&dir).unwrap();
        fs::set_permissions(&dir, fs::Permissions::from_mode(0o500)).unwrap();

        log_start_in(&dir.join("session-start.log"), 1, "clear", "s", SLUG, 1);

        assert!(!dir.join("session-start.log").exists() || dir.join("session-start.log").is_file());
        fs::set_permissions(&dir, fs::Permissions::from_mode(0o700)).unwrap();
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn status_shows_recent_events_and_this_directorys_counts() {
        let dir = scratch_dir("handoff-status");
        let log = dir.join("session-start.log");
        log_start_in(&log, 1_788_252_682, "clear", "s1", SLUG, 1);
        save_in(&dir, SLUG, "# a\n", 100, 1).unwrap();

        let shown = status_in(&dir, &log, SLUG);

        assert!(shown.contains("2026-09-01T08:51:22Z"), "{shown}");
        assert!(shown.contains("clear"), "{shown}");
        assert!(shown.contains("1 unread, 0 already loaded"), "{shown}");
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn status_with_no_log_says_so() {
        let dir = scratch_dir("handoff-status-empty");

        let shown = status_in(&dir, &dir.join("none.log"), SLUG);

        assert!(
            shown.starts_with("No SessionStart has been logged yet."),
            "{shown}"
        );
        let _ = fs::remove_dir_all(dir);
    }
}
