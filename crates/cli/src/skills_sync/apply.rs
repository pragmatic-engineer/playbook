// SPDX-FileCopyrightText: 2026 Igor Santos
// SPDX-License-Identifier: Apache-2.0

//! Writes rendered skill folders into an agent's skill folder without ever
//! overwriting work the user did. A `state` record per folder remembers what
//! playbook wrote, as a hash per file, so a folder is replaced only while it
//! still matches that record. See `docs/internals/15-codex-skills.md`.

use super::Agent;
use crate::common::atomic::{with_dir_lock_aged, LONG_LOCK_STALE_AGE};
use crate::init::migrate::content_hash;
use std::collections::BTreeMap;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;

const LOCK_NAME: &str = ".playbook-sync.lock";
const TMP_PREFIX: &str = ".playbook-tmp-";
const OLD_PREFIX: &str = ".playbook-old-";
/// A run takes seconds, so a leftover this old belongs to a dead run.
const LEFTOVER_MAX_AGE: Duration = Duration::from_secs(600);
/// The refresh claim lives beside the folder records and is not one.
const REFRESH_KEY: &str = "refresh";
const RECORD_PREFIX: &str = "skills-sync/";

const SKIP_NOT_OURS: &str = "not written by playbook";
const SKIP_EDITED: &str = "edited since the last sync";
const SKIP_BUSY: &str = "another sync is running";

static NONCE: AtomicU64 = AtomicU64::new(0);

/// One folder ready to write: its name and its files, paths relative to it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RenderedFolder {
    pub name: String,
    pub files: Vec<(PathBuf, Vec<u8>)>,
}

/// A step of the write sequence, named so a test can make it fail the way a
/// crash there would stop the run.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Step {
    AfterTempWrite,
    AfterPendingRecord,
    AfterOldMoved,
    AfterNewMoved,
    OnDoneRecord,
    AfterDropParked,
    AfterDropRecord,
}

/// Makes one step fail. The default fails nothing.
#[derive(Debug, Clone, Copy, Default)]
pub struct Faults {
    pub fail_at: Option<Step>,
}

/// What a run did: one line per folder, then a summary line.
#[derive(Debug, Default, PartialEq, Eq)]
pub struct Report {
    pub lines: Vec<String>,
    /// Folders that could not be written because of an error.
    pub failed: usize,
    /// Another sync held the lock, so nothing was written.
    pub busy: bool,
    added: usize,
    updated: usize,
    removed: usize,
    skipped: usize,
}

enum Change {
    Added,
    Updated,
    Removed,
    Skipped(&'static str),
}

/// The word for a change, in the future tense on a dry run.
fn verb(change: &Change, dry_run: bool) -> &'static str {
    match (change, dry_run) {
        (Change::Added, false) => "added",
        (Change::Added, true) => "would add",
        (Change::Updated, false) => "updated",
        (Change::Updated, true) => "would update",
        (Change::Removed, false) => "removed",
        (Change::Removed, true) => "would remove",
        (Change::Skipped(_), _) => "skipped",
    }
}

impl Report {
    /// True when every folder was handled: no error and no busy lock.
    pub fn is_clean(&self) -> bool {
        self.failed == 0 && !self.busy
    }

    fn record(&mut self, name: &str, change: &Change, dry_run: bool) {
        let counter = match change {
            Change::Added => &mut self.added,
            Change::Updated => &mut self.updated,
            Change::Removed => &mut self.removed,
            Change::Skipped(reason) => {
                self.skipped += 1;
                self.lines.push(format!("skipped {name}, {reason}"));
                return;
            }
        };
        *counter += 1;
        self.lines.push(format!("{} {name}", verb(change, dry_run)));
    }

    fn fail(&mut self, name: &str, why: &str) {
        self.failed += 1;
        self.lines.push(format!("failed {name}: {why}"));
    }

    fn finish(mut self, dry_run: bool) -> Self {
        let parts: Vec<String> = [
            (self.added, verb(&Change::Added, dry_run)),
            (self.updated, verb(&Change::Updated, dry_run)),
            (self.removed, verb(&Change::Removed, dry_run)),
            (self.skipped, verb(&Change::Skipped(""), dry_run)),
            (self.failed, "failed"),
        ]
        .iter()
        .filter(|(n, _)| *n > 0)
        .map(|(n, label)| format!("{n} {label}"))
        .collect();
        let summary = if parts.is_empty() {
            "nothing to change".to_string()
        } else {
            parts.join(", ")
        };
        self.lines.push(summary);
        self
    }
}

/// File path (with `/` separators) to content hash.
type Hashes = BTreeMap<String, String>;

/// What playbook last wrote to a folder. While `pending`, the folder may
/// still hold the `old` files or already hold the `new` ones.
#[derive(Debug, PartialEq, Eq)]
struct Record {
    version: String,
    pending: bool,
    old: Hashes,
    new: Hashes,
}

impl Record {
    fn owns(&self, found: &Hashes) -> bool {
        *found == self.new || (self.pending && *found == self.old)
    }

    fn encode(&self) -> String {
        let mut out = format!(
            "version {}\nstate {}\n",
            self.version,
            if self.pending { "pending" } else { "done" }
        );
        for (tag, set) in [("old", &self.old), ("new", &self.new)] {
            for (path, hash) in set {
                out.push_str(&format!("{tag} {hash} {path}\n"));
            }
        }
        out
    }

    fn decode(text: &str) -> Option<Self> {
        let mut record = Record {
            version: String::new(),
            pending: false,
            old: Hashes::new(),
            new: Hashes::new(),
        };
        let mut has_state = false;
        for line in text.lines() {
            let mut parts = line.splitn(3, ' ');
            match (parts.next()?, parts.next()?, parts.next()) {
                ("version", v, None) => record.version = v.to_string(),
                ("state", s @ ("pending" | "done"), None) => {
                    record.pending = s == "pending";
                    has_state = true;
                }
                ("old", hash, Some(path)) => {
                    record.old.insert(path.to_string(), hash.to_string());
                }
                ("new", hash, Some(path)) => {
                    record.new.insert(path.to_string(), hash.to_string());
                }
                _ => return None,
            }
        }
        has_state.then_some(record)
    }
}

fn hash_files(files: &[(PathBuf, Vec<u8>)]) -> Hashes {
    files
        .iter()
        .map(|(path, bytes)| (slash_path(path), content_hash(bytes)))
        .collect()
}

fn slash_path(path: &Path) -> String {
    path.components()
        .map(|c| c.as_os_str().to_string_lossy())
        .collect::<Vec<_>>()
        .join("/")
}

/// Hashes every file under `dir`. A link or other odd entry gets a marker
/// hash no record holds, so the folder reads as edited.
fn hash_folder(dir: &Path) -> io::Result<Hashes> {
    let mut out = Hashes::new();
    collect_hashes(dir, Path::new(""), &mut out)?;
    Ok(out)
}

fn collect_hashes(base: &Path, rel: &Path, out: &mut Hashes) -> io::Result<()> {
    for entry in fs::read_dir(base.join(rel))? {
        let entry = entry?;
        let child = rel.join(entry.file_name());
        let kind = entry.file_type()?;
        if kind.is_dir() {
            collect_hashes(base, &child, out)?;
        } else if kind.is_file() {
            out.insert(slash_path(&child), content_hash(&fs::read(entry.path())?));
        } else {
            out.insert(slash_path(&child), "not-a-plain-file".to_string());
        }
    }
    Ok(())
}

fn write_files(dir: &Path, files: &[(PathBuf, Vec<u8>)]) -> io::Result<()> {
    fs::create_dir_all(dir)?;
    for (rel, bytes) in files {
        let path = dir.join(rel);
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)?;
        }
        fs::write(path, bytes)?;
    }
    Ok(())
}

struct Run<'a> {
    root: &'a Path,
    agent: Agent,
    target_root: &'a Path,
    version: &'a str,
    dry_run: bool,
    faults: &'a Faults,
}

/// Syncs `folders` into `target_root`, keeping ownership records under
/// `root` (the data root that holds the `state` table). Temp folders and the
/// lock live in `target_root` so renames stay on one file system.
pub fn apply(
    root: &Path,
    agent: Agent,
    target_root: &Path,
    plugin_version: &str,
    folders: &[RenderedFolder],
    dry_run: bool,
    faults: &Faults,
) -> Report {
    let run = Run {
        root,
        agent,
        target_root,
        version: plugin_version,
        dry_run,
        faults,
    };
    let mut report = Report::default();
    if dry_run {
        run.sync_all(folders, &mut report);
        return report.finish(true);
    }
    // Created before the lock, which is a folder inside it.
    if let Err(e) = fs::create_dir_all(target_root) {
        report.fail(
            &target_root.display().to_string(),
            &format!("cannot create the folder: {e}"),
        );
        return report.finish(false);
    }
    let lock = target_root.join(LOCK_NAME);
    // Fail-open helper: the closure only runs either way, the flag decides.
    let (acquired, ()) = with_dir_lock_aged(&lock, 0, Duration::ZERO, LONG_LOCK_STALE_AGE, || ());
    if !acquired {
        report.busy = true;
        report.skipped += 1;
        report.lines.push(format!("skipped, {SKIP_BUSY}"));
        return report.finish(false);
    }
    remove_leftovers(target_root, LEFTOVER_MAX_AGE);
    run.sync_all(folders, &mut report);
    let _ = fs::remove_dir(&lock);
    report.finish(false)
}

impl Run<'_> {
    fn prefix(&self) -> String {
        format!("{RECORD_PREFIX}{}/", self.agent.key())
    }

    fn key(&self, name: &str) -> String {
        format!("{}{name}", self.prefix())
    }

    fn sync_all(&self, folders: &[RenderedFolder], report: &mut Report) {
        let rows = match crate::state::list_read_only(self.root, &self.prefix()) {
            Ok(rows) => rows,
            Err(e) => {
                report.fail("ownership records", &format!("cannot read them: {e}"));
                return;
            }
        };
        let records: BTreeMap<String, Record> = rows
            .into_iter()
            .filter_map(|(key, value)| {
                let name = key.strip_prefix(&self.prefix())?.to_string();
                Some((name, Record::decode(&value)?))
            })
            .collect();
        for folder in folders {
            match self.sync_folder(folder, records.get(&folder.name)) {
                Ok(Some(change)) => report.record(&folder.name, &change, self.dry_run),
                Ok(None) => {}
                Err(why) => report.fail(&folder.name, &why),
            }
        }
        for (name, record) in &records {
            if name == REFRESH_KEY || folders.iter().any(|f| f.name == *name) {
                continue;
            }
            match self.remove_dropped(name, record) {
                Ok(Some(change)) => report.record(name, &change, self.dry_run),
                Ok(None) => {}
                Err(why) => report.fail(name, &why),
            }
        }
    }

    fn sync_folder(
        &self,
        folder: &RenderedFolder,
        record: Option<&Record>,
    ) -> Result<Option<Change>, String> {
        let dir = self.target_root.join(&folder.name);
        let new = hash_files(&folder.files);
        let found = match fs::symlink_metadata(&dir) {
            Err(e) if e.kind() == io::ErrorKind::NotFound => None,
            Err(e) => return Err(format!("cannot read the folder: {e}")),
            Ok(meta) if meta.is_dir() => Some(hash_folder(&dir).map_err(io_msg)?),
            Ok(_) => return Ok(Some(Change::Skipped(SKIP_NOT_OURS))),
        };
        let Some(found) = found else {
            self.replace(folder, &Hashes::new(), &new, false)?;
            return Ok(Some(Change::Added));
        };
        let Some(record) = record else {
            return Ok(Some(Change::Skipped(SKIP_NOT_OURS)));
        };
        if !record.owns(&found) {
            return Ok(Some(Change::Skipped(SKIP_EDITED)));
        }
        if !record.pending && found == new {
            if record.version != self.version && !self.dry_run {
                self.write_record(&folder.name, false, &Hashes::new(), &new)?;
            }
            return Ok(None);
        }
        self.replace(folder, &found, &new, true)?;
        Ok(Some(Change::Updated))
    }

    fn remove_dropped(&self, name: &str, record: &Record) -> Result<Option<Change>, String> {
        let dir = self.target_root.join(name);
        match fs::symlink_metadata(&dir) {
            Err(e) if e.kind() == io::ErrorKind::NotFound => {
                if !self.dry_run {
                    self.delete_record(name)?;
                }
                return Ok(None);
            }
            Err(e) => return Err(format!("cannot read the folder: {e}")),
            // A link or file is the user's, even when its target looks like ours.
            Ok(meta) if !meta.is_dir() => return Ok(Some(Change::Skipped(SKIP_NOT_OURS))),
            Ok(_) => {}
        }
        if !record.owns(&hash_folder(&dir).map_err(io_msg)?) {
            return Ok(Some(Change::Skipped(SKIP_EDITED)));
        }
        if !self.dry_run {
            // Parked first so a half-finished delete never leaves a partial
            // folder under the real name; leftover cleanup finishes it.
            let parked = self
                .target_root
                .join(format!("{OLD_PREFIX}{}", unique_suffix(name)));
            fs::rename(&dir, &parked).map_err(io_msg)?;
            self.fault(Step::AfterDropParked)?;
            self.delete_record(name)?;
            self.fault(Step::AfterDropRecord)?;
            fs::remove_dir_all(&parked).map_err(io_msg)?;
        }
        Ok(Some(Change::Removed))
    }

    /// The crash-safe write. A stop at any step leaves a folder the next run
    /// still recognises as owned, because the pending record holds both sets.
    fn replace(
        &self,
        folder: &RenderedFolder,
        old: &Hashes,
        new: &Hashes,
        had_old: bool,
    ) -> Result<(), String> {
        if self.dry_run {
            return Ok(());
        }
        let suffix = unique_suffix(&folder.name);
        let tmp = self.target_root.join(format!("{TMP_PREFIX}{suffix}"));
        let parked = self.target_root.join(format!("{OLD_PREFIX}{suffix}"));
        let dir = self.target_root.join(&folder.name);
        write_files(&tmp, &folder.files).map_err(io_msg)?;
        self.fault(Step::AfterTempWrite)?;
        self.write_record(&folder.name, true, old, new)?;
        self.fault(Step::AfterPendingRecord)?;
        if had_old {
            fs::rename(&dir, &parked).map_err(io_msg)?;
        }
        self.fault(Step::AfterOldMoved)?;
        fs::rename(&tmp, &dir).map_err(io_msg)?;
        self.fault(Step::AfterNewMoved)?;
        if had_old {
            fs::remove_dir_all(&parked).map_err(io_msg)?;
        }
        self.fault(Step::OnDoneRecord)?;
        self.write_record(&folder.name, false, &Hashes::new(), new)
    }

    fn fault(&self, step: Step) -> Result<(), String> {
        if self.faults.fail_at == Some(step) {
            return Err(format!("stopped at {step:?}"));
        }
        Ok(())
    }

    fn write_record(
        &self,
        name: &str,
        pending: bool,
        old: &Hashes,
        new: &Hashes,
    ) -> Result<(), String> {
        let record = Record {
            version: self.version.to_string(),
            pending,
            old: old.clone(),
            new: new.clone(),
        };
        crate::state::set(self.root, &self.key(name), &record.encode())
            .map_err(|e| format!("cannot write the ownership record: {e}"))
    }

    fn delete_record(&self, name: &str) -> Result<(), String> {
        crate::state::delete(self.root, &self.key(name))
            .map(|_| ())
            .map_err(|e| format!("cannot delete the ownership record: {e}"))
    }
}

/// A name no other run or step of this process shares.
fn unique_suffix(name: &str) -> String {
    format!(
        "{name}-{}-{}",
        std::process::id(),
        NONCE.fetch_add(1, Ordering::Relaxed)
    )
}

fn io_msg(e: io::Error) -> String {
    e.to_string()
}

/// Removes temp and parked folders a dead run left behind.
fn remove_leftovers(target_root: &Path, max_age: Duration) {
    let Ok(entries) = fs::read_dir(target_root) else {
        return;
    };
    for entry in entries.flatten() {
        let name = entry.file_name();
        let name = name.to_string_lossy();
        if !(name.starts_with(TMP_PREFIX) || name.starts_with(OLD_PREFIX)) {
            continue;
        }
        let old_enough = entry
            .metadata()
            .and_then(|m| m.modified())
            .ok()
            .and_then(|t| t.elapsed().ok())
            .is_some_and(|age| age > max_age);
        if old_enough {
            let _ = fs::remove_dir_all(entry.path());
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::common::test_support::scratch_dir;

    const V1: &str = "1.0.0";
    const V2: &str = "2.0.0";
    const NAME: &str = "playbook-fix";
    const BACKDATE: Duration = Duration::from_secs(660);

    struct Setup {
        data: PathBuf,
        target: PathBuf,
    }

    fn setup(tag: &str) -> Setup {
        let base = scratch_dir(tag);
        fs::create_dir_all(&base).unwrap();
        Setup {
            data: base.join("data"),
            target: base.join("agents").join("skills"),
        }
    }

    fn folder(name: &str, body: &str) -> RenderedFolder {
        RenderedFolder {
            name: name.to_string(),
            files: vec![
                (PathBuf::from("SKILL.md"), body.as_bytes().to_vec()),
                (PathBuf::from("references/a.md"), b"ref".to_vec()),
            ],
        }
    }

    fn run(s: &Setup, v: &str, folders: &[RenderedFolder], dry: bool, faults: Faults) -> Report {
        apply(&s.data, Agent::Codex, &s.target, v, folders, dry, &faults)
    }

    fn sync(s: &Setup, v: &str, folders: &[RenderedFolder]) -> Report {
        run(s, v, folders, false, Faults::default())
    }

    fn record(s: &Setup, name: &str) -> Option<Record> {
        let key = format!("skills-sync/codex/{name}");
        let rows = crate::state::list_read_only(&s.data, &key).unwrap();
        rows.into_iter()
            .find(|(k, _)| *k == key)
            .and_then(|(_, v)| Record::decode(&v))
    }

    fn skill_md(s: &Setup, name: &str) -> String {
        fs::read_to_string(s.target.join(name).join("SKILL.md")).unwrap()
    }

    fn leftovers(s: &Setup) -> Vec<String> {
        fs::read_dir(&s.target)
            .unwrap()
            .flatten()
            .map(|e| e.file_name().to_string_lossy().into_owned())
            .filter(|n| n.starts_with(".playbook-"))
            .collect()
    }

    #[test]
    fn a_new_folder_is_added() {
        // Arrange
        let s = setup("apply-add");

        // Act
        let report = sync(&s, V1, &[folder(NAME, "one")]);

        // Assert
        assert_eq!(report.lines, vec!["added playbook-fix", "1 added"]);
        assert_eq!(skill_md(&s, NAME), "one");
        assert!(!record(&s, NAME).unwrap().pending);
    }

    #[test]
    fn an_owned_unchanged_folder_is_updated() {
        // Arrange
        let s = setup("apply-update");
        sync(&s, V1, &[folder(NAME, "one")]);

        // Act
        let report = sync(&s, V2, &[folder(NAME, "two")]);

        // Assert
        assert_eq!(report.lines, vec!["updated playbook-fix", "1 updated"]);
        assert_eq!(skill_md(&s, NAME), "two");
        assert_eq!(record(&s, NAME).unwrap().version, V2);
    }

    #[test]
    fn a_second_identical_run_changes_nothing() {
        // Arrange
        let s = setup("apply-idempotent");
        sync(&s, V1, &[folder(NAME, "one")]);

        // Act
        let report = sync(&s, V1, &[folder(NAME, "one")]);

        // Assert
        assert_eq!(report.lines, vec!["nothing to change"]);
    }

    #[test]
    fn an_edited_folder_is_skipped_and_reported() {
        // Arrange
        let s = setup("apply-edited");
        sync(&s, V1, &[folder(NAME, "one")]);
        fs::write(s.target.join(NAME).join("SKILL.md"), "mine").unwrap();

        // Act
        let report = sync(&s, V2, &[folder(NAME, "two")]);

        // Assert
        assert_eq!(
            report.lines[0],
            "skipped playbook-fix, edited since the last sync"
        );
        assert_eq!(skill_md(&s, NAME), "mine");
    }

    #[test]
    fn a_folder_with_an_extra_user_file_is_skipped_and_reported() {
        // Arrange
        let s = setup("apply-extra");
        sync(&s, V1, &[folder(NAME, "one")]);
        fs::write(s.target.join(NAME).join("notes.md"), "mine").unwrap();

        // Act
        let report = sync(&s, V2, &[folder(NAME, "two")]);

        // Assert
        assert_eq!(
            report.lines[0],
            "skipped playbook-fix, edited since the last sync"
        );
        assert_eq!(skill_md(&s, NAME), "one");
    }

    #[test]
    fn a_folder_with_no_record_is_skipped_and_reported() {
        // Arrange
        let s = setup("apply-no-record");
        fs::create_dir_all(s.target.join(NAME)).unwrap();
        fs::write(s.target.join(NAME).join("SKILL.md"), "mine").unwrap();

        // Act
        let report = sync(&s, V1, &[folder(NAME, "one")]);

        // Assert
        assert_eq!(
            report.lines[0],
            "skipped playbook-fix, not written by playbook"
        );
        assert_eq!(skill_md(&s, NAME), "mine");
        assert!(record(&s, NAME).is_none());
    }

    #[test]
    fn a_name_the_plugin_dropped_is_removed() {
        // Arrange
        let s = setup("apply-dropped");
        sync(&s, V1, &[folder(NAME, "one"), folder("playbook-gone", "x")]);

        // Act
        let report = sync(&s, V2, &[folder(NAME, "one")]);

        // Assert
        assert_eq!(report.lines[0], "removed playbook-gone");
        assert!(!s.target.join("playbook-gone").exists());
        assert!(record(&s, "playbook-gone").is_none());
        assert!(record(&s, NAME).is_some());
    }

    #[test]
    fn a_dry_run_writes_nothing_and_says_would() {
        // Arrange
        let s = setup("apply-dry");

        // Act
        let report = run(&s, V1, &[folder(NAME, "one")], true, Faults::default());

        // Assert
        assert_eq!(report.lines, vec!["would add playbook-fix", "1 would add"]);
        assert!(!s.target.exists());
        assert!(!s.data.exists());
    }

    fn backdate_leftovers(s: &Setup) {
        for name in leftovers(s) {
            let handle = fs::File::open(s.target.join(name)).unwrap();
            handle
                .set_modified(std::time::SystemTime::now() - BACKDATE)
                .unwrap();
        }
    }

    fn crash_then_recover(step: Step) {
        // Arrange
        let s = setup(&format!("apply-crash-{step:?}"));
        sync(&s, V1, &[folder(NAME, "one")]);
        let crashed = run(
            &s,
            V2,
            &[folder(NAME, "two")],
            false,
            Faults {
                fail_at: Some(step),
            },
        );
        assert_eq!(crashed.failed, 1);
        backdate_leftovers(&s);
        // A real crash never releases the lock folder.
        let lock = s.target.join(LOCK_NAME);
        fs::create_dir_all(&lock).unwrap();
        fs::File::open(&lock)
            .unwrap()
            .set_modified(std::time::SystemTime::now() - LONG_LOCK_STALE_AGE * 2)
            .unwrap();

        // Act
        let report = sync(&s, V2, &[folder(NAME, "two")]);

        // Assert
        assert_eq!(report.failed, 0, "{:?}", report.lines);
        assert_eq!(skill_md(&s, NAME), "two");
        let rec = record(&s, NAME).unwrap();
        assert!(!rec.pending);
        assert_eq!(rec.version, V2);
        assert_eq!(leftovers(&s), Vec::<String>::new());
    }

    #[test]
    fn a_stop_after_the_temp_write_recovers() {
        crash_then_recover(Step::AfterTempWrite);
    }

    #[test]
    fn a_stop_after_the_pending_record_recovers() {
        crash_then_recover(Step::AfterPendingRecord);
    }

    #[test]
    fn a_stop_after_the_old_folder_moved_recovers() {
        crash_then_recover(Step::AfterOldMoved);
    }

    #[test]
    fn a_stop_after_the_new_folder_moved_recovers() {
        crash_then_recover(Step::AfterNewMoved);
    }

    #[test]
    fn a_stop_on_the_done_record_recovers() {
        crash_then_recover(Step::OnDoneRecord);
    }

    #[test]
    fn a_stop_while_dropping_a_folder_recovers_on_the_next_run() {
        for step in [Step::AfterDropParked, Step::AfterDropRecord] {
            // Arrange
            let s = setup(&format!("apply-drop-crash-{step:?}"));
            sync(&s, V1, &[folder(NAME, "one"), folder("playbook-gone", "x")]);
            let crashed = run(
                &s,
                V1,
                &[folder(NAME, "one")],
                false,
                Faults {
                    fail_at: Some(step),
                },
            );
            assert_eq!(crashed.failed, 1, "{:?}", crashed.lines);
            assert!(!s.target.join("playbook-gone").exists());
            backdate_leftovers(&s);

            // Act
            let report = sync(&s, V1, &[folder(NAME, "one")]);

            // Assert
            assert_eq!(report.failed, 0, "{:?}", report.lines);
            assert!(record(&s, "playbook-gone").is_none());
            assert_eq!(leftovers(&s), Vec::<String>::new());
        }
    }

    #[cfg(unix)]
    #[test]
    fn a_dropped_name_that_is_a_symlink_is_left_in_place_and_reported() {
        // Arrange
        let s = setup("apply-drop-symlink");
        sync(&s, V1, &[folder(NAME, "one"), folder("playbook-gone", "x")]);
        let real = s.target.join("real-copy");
        fs::rename(s.target.join("playbook-gone"), &real).unwrap();
        std::os::unix::fs::symlink(&real, s.target.join("playbook-gone")).unwrap();

        // Act
        let report = sync(&s, V1, &[folder(NAME, "one")]);

        // Assert
        assert_eq!(
            report.lines[0],
            "skipped playbook-gone, not written by playbook"
        );
        assert!(s.target.join("playbook-gone").is_symlink());
        assert!(real.join("SKILL.md").is_file());
    }

    #[test]
    fn a_held_lock_marks_the_report_busy() {
        // Arrange
        let s = setup("apply-busy-flag");
        fs::create_dir_all(s.target.join(LOCK_NAME)).unwrap();

        // Act
        let busy = sync(&s, V1, &[folder(NAME, "one")]);
        fs::remove_dir(s.target.join(LOCK_NAME)).unwrap();
        let free = sync(&s, V1, &[folder(NAME, "one")]);

        // Assert
        assert!(busy.busy && !busy.is_clean());
        assert!(!free.busy && free.is_clean());
    }

    #[test]
    fn a_missing_target_folder_is_created() {
        // Arrange
        let s = setup("apply-create-target");
        assert!(!s.target.exists());

        // Act
        let report = sync(&s, V1, &[folder(NAME, "one")]);

        // Assert
        assert_eq!(report.failed, 0);
        assert_eq!(skill_md(&s, NAME), "one");
    }

    #[cfg(unix)]
    #[test]
    fn a_read_only_parent_reports_an_error_and_writes_no_record() {
        use std::os::unix::fs::PermissionsExt;
        // Arrange
        let s = setup("apply-read-only");
        let parent = s.target.parent().unwrap().to_path_buf();
        fs::create_dir_all(&parent).unwrap();
        fs::set_permissions(&parent, fs::Permissions::from_mode(0o555)).unwrap();
        let writable = fs::create_dir(parent.join("probe")).is_ok();

        // Act
        let report = sync(&s, V1, &[folder(NAME, "one")]);

        // Assert
        fs::set_permissions(&parent, fs::Permissions::from_mode(0o755)).unwrap();
        if writable {
            return; // running as a user the mode does not bind
        }
        assert_eq!(report.failed, 1);
        assert!(record(&s, NAME).is_none());
    }

    #[test]
    fn a_held_lock_skips_the_run_and_stays_in_place() {
        // Arrange
        let s = setup("apply-locked");
        fs::create_dir_all(s.target.join(LOCK_NAME)).unwrap();

        // Act
        let report = sync(&s, V1, &[folder(NAME, "one")]);

        // Assert
        assert_eq!(report.lines[0], "skipped, another sync is running");
        assert!(!s.target.join(NAME).exists());
        assert!(record(&s, NAME).is_none());
        assert!(s.target.join(LOCK_NAME).is_dir());
    }
}
