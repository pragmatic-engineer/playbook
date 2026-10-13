// SPDX-FileCopyrightText: 2026 Igor Santos
// SPDX-License-Identifier: Apache-2.0

//! `playbook atlassian check`: is `acli` installed and new enough for the
//! Confluence commands, and is `twg` installed. The `playbook:atlassian`
//! skill runs it once before choosing a route.

use std::ffi::OsStr;
use std::path::PathBuf;
use std::process::Command;
use std::time::Duration;

/// The oldest acli this plugin relies on. 1.3.39-stable is the release the
/// Confluence commands (`page`, `space`, `blog`) were verified against.
pub const MIN_ACLI: Version = Version(1, 3, 39);

/// The hint printed when acli is too old.
pub const UPGRADE_HINT: &str = "brew upgrade atlassian/acli/acli";

const PROBE_TIMEOUT: Duration = Duration::from_secs(10);

/// `major.minor.patch`, ordered field by field.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub struct Version(pub u32, pub u32, pub u32);

impl std::fmt::Display for Version {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}.{}.{}", self.0, self.1, self.2)
    }
}

/// The first `X.Y.Z` in `text`, ignoring any suffix such as `-stable` or
/// `-beta.1`. `acli version 1.3.39-stable` gives `1.3.39`.
pub fn parse_version(text: &str) -> Option<Version> {
    text.split(|c: char| !(c.is_ascii_digit() || c == '.'))
        .find_map(|token| {
            let mut parts = token.trim_matches('.').split('.');
            let major = parts.next()?.parse().ok()?;
            let minor = parts.next()?.parse().ok()?;
            let patch = parts.next()?.parse().ok()?;
            if parts.next().is_some() {
                return None;
            }
            Some(Version(major, minor, patch))
        })
}

/// What one tool looks like on this machine.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Status {
    Missing,
    /// Present but its version output held no `X.Y.Z`.
    Unreadable,
    TooOld(Version),
    Ok(Version),
}

/// Classify `--version` output against `minimum`. `None` means the binary
/// is not there.
pub fn classify(output: Option<&str>, minimum: Version) -> Status {
    match output {
        None => Status::Missing,
        Some(text) => match parse_version(text) {
            None => Status::Unreadable,
            Some(v) if v < minimum => Status::TooOld(v),
            Some(v) => Status::Ok(v),
        },
    }
}

fn find_on_path(path_var: &OsStr, name: &str) -> Option<PathBuf> {
    std::env::split_paths(path_var)
        .filter(|d| d.is_absolute())
        .map(|d| d.join(name))
        .find(|p| p.is_file())
}

/// Run `name --version` using only the directories in `path_var`. `None`
/// when the binary is not on that path. A binary that is there but silent
/// or hung yields `Some("")`.
fn version_output(path_var: &OsStr, name: &str) -> Option<String> {
    let bin = find_on_path(path_var, name)?;
    let mut cmd = Command::new(bin);
    cmd.arg("--version");
    let out = crate::common::proc::run_with_timeout(&mut cmd, PROBE_TIMEOUT);
    Some(
        out.map(|o| {
            format!(
                "{}{}",
                String::from_utf8_lossy(&o.stdout),
                String::from_utf8_lossy(&o.stderr)
            )
        })
        .unwrap_or_default(),
    )
}

/// acli's status on `path_var`.
pub fn acli_status(path_var: &OsStr) -> Status {
    classify(version_output(path_var, "acli").as_deref(), MIN_ACLI)
}

/// twg's status on `path_var`. There is no minimum, so a readable version
/// is `Ok` and anything else present is `Unreadable`.
pub fn twg_status(path_var: &OsStr) -> Status {
    classify(version_output(path_var, "twg").as_deref(), Version(0, 0, 0))
}

/// One human line for acli.
pub fn acli_line(status: &Status) -> String {
    match status {
        Status::Missing => "acli: missing (next route: twg)".to_string(),
        Status::Unreadable => {
            format!("acli: version unreadable, need >= {MIN_ACLI}; run: {UPGRADE_HINT}")
        }
        Status::TooOld(v) => {
            format!("acli: too old ({v}, need >= {MIN_ACLI}); run: {UPGRADE_HINT}")
        }
        Status::Ok(v) => format!("acli: ok ({v}, need >= {MIN_ACLI})"),
    }
}

/// One human line for twg. Its version flag is not documented, so a present
/// binary with unreadable output is reported as present.
pub fn twg_line(status: &Status) -> String {
    match status {
        Status::Missing => "twg: missing".to_string(),
        Status::Unreadable | Status::TooOld(_) => "twg: present (version unknown)".to_string(),
        Status::Ok(v) => format!("twg: present ({v})"),
    }
}

/// The report and whether acli is usable (exit 0) or not (exit 1).
pub fn check(path_var: &OsStr) -> (String, bool) {
    let acli = acli_status(path_var);
    let twg = twg_status(path_var);
    let usable = matches!(acli, Status::Ok(_));
    (format!("{}\n{}", acli_line(&acli), twg_line(&twg)), usable)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use std::os::unix::fs::PermissionsExt;

    #[test]
    fn a_version_ignores_the_stable_suffix_and_surrounding_words() {
        assert_eq!(
            parse_version("acli version 1.3.39-stable\n"),
            Some(Version(1, 3, 39))
        );
        assert_eq!(parse_version("1.3.39"), Some(Version(1, 3, 39)));
        assert_eq!(parse_version("v2.0.1-beta.2"), Some(Version(2, 0, 1)));
    }

    #[test]
    fn garbage_has_no_version() {
        assert_eq!(parse_version(""), None);
        assert_eq!(parse_version("command not found"), None);
        assert_eq!(parse_version("1.3"), None);
        assert_eq!(parse_version("1.2.3.4"), None);
        assert_eq!(parse_version("a.b.c"), None);
    }

    #[test]
    fn versions_compare_by_field_not_by_text() {
        assert!(Version(1, 3, 9) < Version(1, 3, 39));
        assert!(Version(1, 10, 0) > Version(1, 9, 99));
        assert!(Version(2, 0, 0) > Version(1, 99, 99));
    }

    #[test]
    fn classify_sorts_missing_old_ok_and_unreadable() {
        assert_eq!(classify(None, MIN_ACLI), Status::Missing);
        assert_eq!(classify(Some("nonsense"), MIN_ACLI), Status::Unreadable);
        assert_eq!(
            classify(Some("acli version 1.3.22-stable"), MIN_ACLI),
            Status::TooOld(Version(1, 3, 22))
        );
        assert_eq!(
            classify(Some("acli version 1.3.39-stable"), MIN_ACLI),
            Status::Ok(MIN_ACLI)
        );
        assert_eq!(
            classify(Some("acli version 1.4.0-stable"), MIN_ACLI),
            Status::Ok(Version(1, 4, 0))
        );
    }

    #[test]
    fn the_too_old_line_carries_the_upgrade_hint() {
        let line = acli_line(&Status::TooOld(Version(1, 3, 22)));
        assert!(line.contains("brew upgrade atlassian/acli/acli"), "{line}");
        assert!(line.contains("1.3.22") && line.contains("1.3.39"), "{line}");
        assert!(!acli_line(&Status::Ok(MIN_ACLI)).contains("brew"));
    }

    fn fake_bin(dir: &std::path::Path, name: &str, script: &str) {
        let p = dir.join(name);
        fs::write(&p, format!("#!/bin/sh\n{script}\n")).unwrap();
        fs::set_permissions(&p, fs::Permissions::from_mode(0o755)).unwrap();
    }

    fn scratch(tag: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("pb-atl-{tag}-{}", crate::testing::run_id()));
        let _ = fs::remove_dir_all(&d);
        fs::create_dir_all(&d).unwrap();
        d
    }

    #[test]
    fn a_path_without_the_binaries_reports_both_missing() {
        let d = scratch("empty");
        let (text, usable) = check(d.as_os_str());
        assert!(!usable);
        assert!(
            text.contains("acli: missing") && text.contains("twg: missing"),
            "{text}"
        );
        let _ = fs::remove_dir_all(&d);
    }

    #[test]
    fn a_path_override_finds_a_current_acli_and_a_twg() {
        let d = scratch("ok");
        fake_bin(&d, "acli", "echo 'acli version 1.3.39-stable'");
        fake_bin(&d, "twg", "echo 'twg 0.9.1'");
        let (text, usable) = check(d.as_os_str());
        assert!(usable, "{text}");
        assert!(text.contains("acli: ok (1.3.39"), "{text}");
        assert!(text.contains("twg: present (0.9.1)"), "{text}");
        let _ = fs::remove_dir_all(&d);
    }

    #[test]
    fn an_old_acli_is_not_usable() {
        let d = scratch("old");
        fake_bin(&d, "acli", "echo 'acli version 1.3.22-stable'");
        let (text, usable) = check(d.as_os_str());
        assert!(!usable);
        assert!(text.contains("too old (1.3.22"), "{text}");
        let _ = fs::remove_dir_all(&d);
    }

    #[test]
    fn a_binary_with_garbage_output_is_unreadable_not_missing() {
        let d = scratch("junk");
        fake_bin(&d, "acli", "echo 'boom'");
        fake_bin(&d, "twg", "exit 1");
        let (text, usable) = check(d.as_os_str());
        assert!(!usable);
        assert!(text.contains("version unreadable"), "{text}");
        assert!(text.contains("twg: present (version unknown)"), "{text}");
        let _ = fs::remove_dir_all(&d);
    }
}
