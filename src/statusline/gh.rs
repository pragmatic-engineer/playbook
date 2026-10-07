// SPDX-FileCopyrightText: 2026 Igor Santos
// SPDX-License-Identifier: MIT

//! GitHub-facing helpers: remote parsing (`shell/gh-remote.sh`), ISO-8601
//! parsing, `PATH` lookup and the detached cache refreshes.

use std::os::unix::process::CommandExt;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::SystemTime;

/// Seconds a background `gh` call may run before its watchdog kills it.
const REFRESH_TIMEOUT_SECS: u32 = 20;
/// Age after which a refresh lock is treated as abandoned.
const LOCK_STALE_SECS: i64 = 30;

const CI_QUERY: &str = "query($owner: String!, $repo: String!, $branch: String!) { repository(owner: $owner, name: $repo) { ref(qualifiedName: $branch) { target { ... on Commit { statusCheckRollup { contexts(first: 100) { nodes { __typename ... on CheckRun { status conclusion } ... on StatusContext { state } } } } } } } } }";

const REFRESH_SCRIPT: &str = r#"trap 'rm -f "$1"' EXIT
lock="$1"; cache="$2"; tmp="$2.tmp.$$"; limit="$3"; shift 3
NO_COLOR=1 GIT_TERMINAL_PROMPT=0 "$@" >"$tmp" 2>/dev/null &
pid=$!
( sleep "$limit"; kill "$pid" 2>/dev/null ) >/dev/null 2>&1 &
dog=$!
if wait "$pid"; then mv "$tmp" "$cache"; else rm -f "$tmp"; fi
kill "$dog" 2>/dev/null
"#;

/// Splits a scp-style, ssh or http(s) remote into `(host, path)`.
fn split_remote(url: &str) -> Option<(String, String)> {
    if let Some(rest) = url.strip_prefix("git@") {
        let (h, p) = rest.split_once(':')?;
        return Some((h.to_string(), p.to_string()));
    }
    if let Some(rest) = url.strip_prefix("ssh://git@") {
        let (h, p) = rest.split_once('/')?;
        return Some((h.split(':').next()?.to_string(), p.to_string()));
    }
    let rest = url
        .strip_prefix("https://")
        .or_else(|| url.strip_prefix("http://"))?;
    let (h, p) = rest.split_once('/')?;
    let h = h.rsplit('@').next()?;
    Some((h.split(':').next()?.to_string(), p.to_string()))
}

/// Parses a git remote URL into `(host, owner/repo)` for github.com and
/// `*.ghe.com`; `None` for anything else.
pub fn remote_parse(url: &str) -> Option<(String, String)> {
    let (host, path) = split_remote(url)?;
    if host.is_empty() || path.is_empty() {
        return None;
    }
    let path = path.strip_suffix(".git").unwrap_or(&path).to_string();
    if !(host == "github.com" || host.ends_with(".ghe.com")) || !path.contains('/') {
        return None;
    }
    Some((host, path))
}

/// Epoch seconds for a strict `YYYY-MM-DDTHH:MM:SSZ` UTC timestamp.
pub fn iso_to_epoch(s: &str) -> Option<i64> {
    let b = s.as_bytes();
    if b.len() != 20 || b[4] != b'-' || b[7] != b'-' || b[10] != b'T' || b[19] != b'Z' {
        return None;
    }
    if b[13] != b':' || b[16] != b':' {
        return None;
    }
    let num = |a: usize, z: usize| -> Option<i64> {
        let part = s.get(a..z)?;
        part.bytes()
            .all(|c| c.is_ascii_digit())
            .then(|| part.parse().ok())?
    };
    let (y, mo, d) = (num(0, 4)?, num(5, 7)?, num(8, 10)?);
    let (h, mi, sec) = (num(11, 13)?, num(14, 16)?, num(17, 19)?);
    if !(1..=12).contains(&mo) || !(1..=31).contains(&d) || h > 23 || mi > 59 || sec > 60 {
        return None;
    }
    let y = if mo <= 2 { y - 1 } else { y };
    let era = y.div_euclid(400);
    let yoe = y - era * 400;
    let mp = (mo + 9) % 12;
    let doy = (153 * mp + 2) / 5 + d - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    let days = era * 146097 + doe - 719468;
    Some(days * 86400 + h * 3600 + mi * 60 + sec)
}

/// First executable named `name` on `path`.
pub fn find_in_path(name: &str, path: &str) -> Option<PathBuf> {
    use std::os::unix::fs::PermissionsExt;
    path.split(':').filter(|d| !d.is_empty()).find_map(|dir| {
        let cand = Path::new(dir).join(name);
        let meta = std::fs::metadata(&cand).ok()?;
        (meta.is_file() && meta.permissions().mode() & 0o111 != 0).then_some(cand)
    })
}

/// File mtime in epoch seconds, 0 when it cannot be read.
pub fn file_mtime(p: &Path) -> i64 {
    std::fs::metadata(p)
        .and_then(|m| m.modified())
        .ok()
        .and_then(|t| t.duration_since(SystemTime::UNIX_EPOCH).ok())
        .map_or(0, |d| d.as_secs() as i64)
}

/// Age of `p` in seconds, or `missing` when the file does not exist.
pub fn age_of(p: &Path, now: i64, missing: i64) -> i64 {
    if p.is_file() {
        now - file_mtime(p)
    } else {
        missing
    }
}

/// Runs `gh` in the background to refresh `cache`, guarded by `cache.lock` so
/// at most one refresh is in flight. Never waits for the child.
fn fire(gh: &Path, cache: &Path, now: i64, args: &[String]) {
    let mut lock = cache.as_os_str().to_owned();
    lock.push(".lock");
    let lock = PathBuf::from(lock);
    if age_of(&lock, now, 0) > LOCK_STALE_SECS {
        let _ = std::fs::remove_file(&lock);
    }
    if std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&lock)
        .is_err()
    {
        return;
    }
    let spawned = Command::new("sh")
        .arg("-c")
        .arg(REFRESH_SCRIPT)
        .arg("_")
        .arg(&lock)
        .arg(cache)
        .arg(REFRESH_TIMEOUT_SECS.to_string())
        .arg(gh)
        .args(args)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .process_group(0)
        .spawn();
    if spawned.is_err() {
        let _ = std::fs::remove_file(&lock);
    }
}

/// Detached refresh of the cached `gh pr view` JSON.
pub fn refresh_pr(gh: &Path, cache: &Path, now: i64) {
    let args = [
        "pr",
        "view",
        "--json",
        "number,author,reviewDecision,state,mergedAt,closedAt,body,latestReviews,reviewRequests,statusCheckRollup",
    ];
    fire(gh, cache, now, &args.map(String::from));
}

/// Detached refresh of the cached branch-head CI rollup.
pub fn refresh_ci(gh: &Path, cache: &Path, now: i64, owner: &str, repo: &str, branch: &str) {
    let args = [
        "api".to_string(),
        "graphql".to_string(),
        "-F".to_string(),
        format!("owner={owner}"),
        "-F".to_string(),
        format!("repo={repo}"),
        "-F".to_string(),
        format!("branch={branch}"),
        "-f".to_string(),
        format!("query={CI_QUERY}"),
    ];
    fire(gh, cache, now, &args);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn remote_parse_accepts_github_family_hosts_only() {
        let gh = |h: &str, p: &str| Some((h.to_string(), p.to_string()));
        assert_eq!(
            remote_parse("git@github.com:o/r.git"),
            gh("github.com", "o/r")
        );
        assert_eq!(
            remote_parse("https://u@github.com/o/r"),
            gh("github.com", "o/r")
        );
        assert_eq!(
            remote_parse("ssh://git@acme.ghe.com:22/o/r.git"),
            gh("acme.ghe.com", "o/r")
        );
        assert_eq!(remote_parse("git@gitlab.com:o/r.git"), None);
        assert_eq!(remote_parse("https://github.com/owner"), None);
        assert_eq!(remote_parse("/local/path"), None);
        assert_eq!(remote_parse(""), None);
    }

    #[test]
    fn iso_to_epoch_parses_utc_and_rejects_other_shapes() {
        assert_eq!(iso_to_epoch("2026-07-08T00:00:00Z"), Some(1783468800));
        assert_eq!(iso_to_epoch("1970-01-01T00:00:01Z"), Some(1));
        assert_eq!(iso_to_epoch("2026-07-08 00:00:00"), None);
        assert_eq!(iso_to_epoch("2026-13-08T00:00:00Z"), None);
        assert_eq!(iso_to_epoch(""), None);
    }
}
