// SPDX-FileCopyrightText: 2026 Igor Santos
// SPDX-License-Identifier: Apache-2.0

//! The small generic helpers the tool needs: the clock, private file writes,
//! a bounded `git` call, and the home directory.

use std::path::PathBuf;

/// The user's home directory, empty when it cannot be found.
pub fn home_dir() -> PathBuf {
    std::env::home_dir().unwrap_or_default()
}

/// The wall clock as plain numbers.
pub mod time {
    use std::time::{SystemTime, UNIX_EPOCH};

    /// Whole seconds since the unix epoch. A clock set before 1970 reads as 0.
    pub fn now_secs() -> i64 {
        let secs = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs();
        i64::try_from(secs).unwrap_or(i64::MAX)
    }

    /// `secs` since the unix epoch as `YYYY-MM-DDTHH:MM:SSZ` (UTC). A negative
    /// value reads as the epoch.
    pub fn iso_utc(secs: i64) -> String {
        let secs = secs.max(0);
        let (days, rem) = (secs / 86_400, secs % 86_400);
        // Civil date from days since 1970-01-01 (Howard Hinnant's algorithm).
        let z = days + 719_468;
        let era = z / 146_097;
        let doe = z - era * 146_097;
        let yoe = (doe - doe / 1_460 + doe / 36_524 - doe / 146_096) / 365;
        let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
        let mp = (5 * doy + 2) / 153;
        let day = doy - (153 * mp + 2) / 5 + 1;
        let month = if mp < 10 { mp + 3 } else { mp - 9 };
        let year = yoe + era * 400 + i64::from(month <= 2);
        format!(
            "{year:04}-{month:02}-{day:02}T{:02}:{:02}:{:02}Z",
            rem / 3_600,
            rem % 3_600 / 60,
            rem % 60
        )
    }

    #[cfg(test)]
    mod tests {
        use super::*;

        #[test]
        fn the_clock_is_after_2020() {
            assert!(now_secs() > 1_577_836_800);
        }

        #[test]
        fn iso_utc_formats_known_instants() {
            assert_eq!(iso_utc(0), "1970-01-01T00:00:00Z");
            assert_eq!(iso_utc(-5), "1970-01-01T00:00:00Z");
            assert_eq!(iso_utc(1_700_000_000), "2023-11-14T22:13:20Z");
        }
    }
}

/// File writes that keep account and repo names owner-only.
pub mod atomic {
    use std::fs;
    use std::io::Write;
    use std::path::Path;
    use std::sync::atomic::{AtomicU64, Ordering};

    /// Creates `dir` (and any missing parents) and makes the leaf owner-only on
    /// unix.
    pub fn ensure_private_dir(dir: &Path) -> std::io::Result<()> {
        fs::create_dir_all(dir)?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            fs::set_permissions(dir, fs::Permissions::from_mode(0o700))?;
        }
        Ok(())
    }

    static TMP_SEQ: AtomicU64 = AtomicU64::new(0);

    /// Replaces the contents of `path` through a temp file in the same
    /// directory and a rename, so a reader never sees a partial file. A new
    /// file is owner-only on unix, and the content is flushed before the
    /// rename. The caller creates the directory.
    pub fn write_atomic_private(path: &Path, content: impl AsRef<[u8]>) -> std::io::Result<()> {
        let dir = path
            .parent()
            .filter(|p| !p.as_os_str().is_empty())
            .unwrap_or_else(|| Path::new("."));
        let name = path
            .file_name()
            .map_or_else(String::new, |n| n.to_string_lossy().into_owned());
        let seq = TMP_SEQ.fetch_add(1, Ordering::Relaxed);
        let tmp = dir.join(format!(".{name}.{}.{seq}.tmp", std::process::id()));
        let result = stage(&tmp, content.as_ref())
            .and_then(|()| match fs::metadata(path) {
                Ok(meta) => fs::set_permissions(&tmp, meta.permissions()),
                Err(_) => Ok(()),
            })
            .and_then(|()| fs::rename(&tmp, path));
        if result.is_err() {
            let _ = fs::remove_file(&tmp);
        }
        result
    }

    fn stage(tmp: &Path, content: &[u8]) -> std::io::Result<()> {
        let mut options = fs::OpenOptions::new();
        options.write(true).create_new(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            options.mode(0o600);
        }
        let mut file = options.open(tmp)?;
        file.write_all(content)?;
        file.sync_all()
    }

    /// Creates the database file owner-only before SQLite can make it with the
    /// default umask (the WAL and shm files copy this mode), and tightens an
    /// older, wider file.
    pub fn make_db_file_private(path: &Path) -> Result<(), String> {
        #[cfg(unix)]
        {
            use std::os::unix::fs::{OpenOptionsExt, PermissionsExt};
            let error = |e: std::io::Error| format!("failed to secure {}: {e}", path.display());
            fs::OpenOptions::new()
                .write(true)
                .create(true)
                .truncate(false)
                .mode(0o600)
                .open(path)
                .map_err(error)?;
            fs::set_permissions(path, fs::Permissions::from_mode(0o600)).map_err(error)?;
        }
        #[cfg(not(unix))]
        let _ = path;
        Ok(())
    }

    #[cfg(test)]
    mod tests {
        use super::*;

        #[test]
        fn a_private_write_replaces_the_file_and_is_owner_only() {
            let dir = crate::util::scratch_dir("atomic-private");
            ensure_private_dir(&dir).unwrap();
            let file = dir.join("a.json");
            write_atomic_private(&file, "one").unwrap();
            write_atomic_private(&file, "two").unwrap();
            assert_eq!(fs::read_to_string(&file).unwrap(), "two");
            #[cfg(unix)]
            {
                use std::os::unix::fs::PermissionsExt;
                let mode = fs::metadata(&file).unwrap().permissions().mode() & 0o777;
                assert_eq!(mode, 0o600);
            }
        }

        #[test]
        fn a_db_file_is_created_and_tightened() {
            let dir = crate::util::scratch_dir("atomic-db");
            ensure_private_dir(&dir).unwrap();
            let file = dir.join("x.db");
            fs::write(&file, b"").unwrap();
            #[cfg(unix)]
            {
                use std::os::unix::fs::PermissionsExt;
                fs::set_permissions(&file, fs::Permissions::from_mode(0o644)).unwrap();
            }
            make_db_file_private(&file).unwrap();
            #[cfg(unix)]
            {
                use std::os::unix::fs::PermissionsExt;
                assert_eq!(
                    fs::metadata(&file).unwrap().permissions().mode() & 0o777,
                    0o600
                );
            }
        }
    }
}

/// A bounded `git` call.
pub mod git {
    use std::path::Path;
    use std::process::{Command, Stdio};
    use std::time::{Duration, Instant};

    const TIMEOUT: Duration = Duration::from_secs(5);

    /// Trimmed stdout of `git -C dir <args>`. `None` when git cannot start,
    /// exits non-zero or takes longer than five seconds.
    pub fn output(dir: &Path, args: &[&str]) -> Option<String> {
        let mut child = Command::new("git")
            .arg("-C")
            .arg(dir)
            .args(args)
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()
            .ok()?;
        let started = Instant::now();
        loop {
            match child.try_wait().ok()? {
                Some(_) => break,
                None if started.elapsed() > TIMEOUT => {
                    let _ = child.kill();
                    let _ = child.wait();
                    return None;
                }
                None => std::thread::sleep(Duration::from_millis(5)),
            }
        }
        let out = child.wait_with_output().ok()?;
        out.status
            .success()
            .then(|| String::from_utf8_lossy(&out.stdout).trim().to_string())
    }

    #[cfg(test)]
    mod tests {
        use super::*;

        #[test]
        fn a_failing_command_reads_as_none() {
            let dir = std::env::temp_dir();
            assert_eq!(output(&dir, &["definitely-not-a-git-command"]), None);
        }

        #[test]
        fn a_missing_directory_reads_as_none() {
            let missing = std::env::temp_dir().join("playbook-telemetry-no-such-dir");
            assert_eq!(output(&missing, &["--version"]), None);
        }
    }
}

/// A unique scratch path for tests. Not created by this call.
#[cfg(test)]
pub(crate) fn scratch_dir(tag: &str) -> PathBuf {
    use std::sync::atomic::{AtomicU64, Ordering};
    static COUNTER: AtomicU64 = AtomicU64::new(0);
    let n = COUNTER.fetch_add(1, Ordering::Relaxed);
    std::env::temp_dir().join(format!(
        "playbook-telemetry-test-{}-{tag}-{n}",
        std::process::id()
    ))
}
