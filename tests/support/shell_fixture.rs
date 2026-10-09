// SPDX-FileCopyrightText: 2026 Igor Santos
// SPDX-License-Identifier: Apache-2.0

//! Helpers for tests that run the repo's real shell scripts (`install.sh`,
//! `bin/playbook`) against stubbed `curl`, `gh`, `uname` and `claude`.

#![allow(dead_code)]

use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Output, Stdio};
use std::sync::atomic::{AtomicU64, Ordering};

static COUNTER: AtomicU64 = AtomicU64::new(0);

/// The repository root.
pub fn repo_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
}

/// A scratch directory removed on drop.
pub struct Work(pub PathBuf);

impl Work {
    pub fn new(tag: &str) -> Work {
        let n = COUNTER.fetch_add(1, Ordering::Relaxed);
        let dir = std::env::temp_dir().join(format!("pb-shell-{tag}-{}-{n}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).expect("scratch dir");
        Work(dir)
    }

    pub fn path(&self, rel: &str) -> PathBuf {
        self.0.join(rel)
    }

    /// A new empty directory under the scratch dir.
    pub fn dir(&self, rel: &str) -> PathBuf {
        let p = self.0.join(rel);
        fs::create_dir_all(&p).expect("subdir");
        p
    }
}

impl Drop for Work {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

/// Write `content` to `path` and make it executable.
pub fn write_exec(path: &Path, content: &str) {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).expect("parent dir");
    }
    fs::write(path, content).expect("write script");
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(path, fs::Permissions::from_mode(0o755)).expect("chmod");
    }
}

/// The tracked files of HEAD extracted into `dest`, so untracked build output
/// cannot leak into a run. `None` when this is not a git checkout.
pub fn archive_head(dest: &Path) -> Option<()> {
    let root = repo_root();
    let inside = Command::new("git")
        .args(["-C"])
        .arg(&root)
        .args(["rev-parse", "--git-dir"])
        .output()
        .ok()?;
    if !inside.status.success() {
        return None;
    }
    fs::create_dir_all(dest).ok()?;
    let archive = Command::new("git")
        .arg("-C")
        .arg(&root)
        .args(["archive", "HEAD"])
        .stdout(Stdio::piped())
        .spawn()
        .ok()?;
    let status = Command::new("tar")
        .arg("-x")
        .arg("-C")
        .arg(dest)
        .stdin(archive.stdout?)
        .status()
        .ok()?;
    status.success().then_some(())
}

/// Run `bash <script> <args>` with `envs`, capturing both streams.
pub fn bash(script: &Path, args: &[&str], envs: &[(&str, String)]) -> Output {
    let mut cmd = Command::new("bash");
    cmd.arg(script).args(args);
    for (k, v) in envs {
        cmd.env(k, v);
    }
    cmd.output().expect("bash should spawn")
}

pub fn stdout(out: &Output) -> String {
    String::from_utf8_lossy(&out.stdout).into_owned()
}

pub fn stderr(out: &Output) -> String {
    String::from_utf8_lossy(&out.stderr).into_owned()
}

/// A directory holding a copy of the built `playbook` binary, for runs of
/// `install.sh` that need a real one at `PLAYBOOK_BIN_DIR`.
pub fn bin_dir_with_playbook(work: &Work) -> PathBuf {
    let dir = work.dir("bin");
    let dest = dir.join("playbook");
    fs::copy(env!("CARGO_BIN_EXE_playbook"), &dest).expect("copy playbook");
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(&dest, fs::Permissions::from_mode(0o755)).expect("chmod");
    }
    dir
}

const CURL_STUB: &str = r#"out=""; url=""; fail_on_http_error=0
while [ $# -gt 0 ]; do
  case "$1" in
    -o) out="$2"; shift 2 ;;
    -w) shift 2 ;;
    -*) case "$1" in *f*) fail_on_http_error=1 ;; esac; shift ;;
    *)  url="$1"; shift ;;
  esac
done
if [ -n "${CURL_LOG:-}" ]; then printf '%s\n' "$url" >> "$CURL_LOG"; fi
case "$url" in
  *api.github.com/repos/*/releases/latest)
    if [ "${STUB_TRANSPORT_FAIL:-0}" = "1" ]; then exit 7; fi
    code="${STUB_CODE:-200}"
    if [ "$fail_on_http_error" = "1" ] && [ "$code" -ge 400 ]; then exit 22; fi
    if [ -n "$out" ]; then
      printf '%s' "${STUB_BODY:-}" > "$out"
      printf '%s' "$code"
    else
      printf '%s' "${STUB_BODY:-}"
    fi
    exit 0 ;;
  */releases/download/*/SHA256SUMS)
    if [ -n "$out" ]; then printf '%s' "${STUB_SUMS_BODY:-}" > "$out"
    else printf '%s' "${STUB_SUMS_BODY:-}"; fi
    exit 0 ;;
  */releases/download/*)
    if [ -n "$out" ]; then printf '%s' "${STUB_ASSET_BODY:-}" > "$out"
    else printf '%s' "${STUB_ASSET_BODY:-}"; fi
    exit 0 ;;
  *) exit 22 ;;
esac
"#;

const UNAME_STUB: &str = r#"case "$1" in
  -s) printf '%s\n' "${STUB_UNAME_S:-Linux}" ;;
  -m) printf '%s\n' "${STUB_UNAME_M:-x86_64}" ;;
  *) command -p uname "$@" ;;
esac
"#;

const GH_STUB: &str = r#"case "${STUB_GH:-absent}" in
  absent) exit 127 ;;
  *) [ "$1 $2" = "attestation --help" ] && exit 0
     [ "$1 $2" = "attestation verify" ] || exit 1
     echo "gh $*" >> "${GH_LOG:-/dev/null}"
     [ "$STUB_GH" = pass ] && exit 0
     [ "$STUB_GH" = missing ] && { echo "no attestations found for subject" >&2; exit 1; }
     echo "no matching attestation" >&2; exit 1 ;;
esac
"#;

/// Write the `curl`, `uname` and `gh` stubs `install.sh` is driven with into
/// `dir`. `curl` serves the releases API, the release asset and SHA256SUMS from
/// `STUB_*` env vars and fails on anything else, `uname` is fixed by
/// `STUB_UNAME_S` and `STUB_UNAME_M`, and `gh` plays the attestation check.
pub fn install_stubs(dir: &Path) {
    write_exec(&dir.join("curl"), CURL_STUB);
    write_exec(&dir.join("uname"), UNAME_STUB);
    write_exec(&dir.join("gh"), GH_STUB);
}

/// Lowercase hex SHA-256 of `data`.
pub fn sha256_hex(data: &[u8]) -> String {
    use sha2::{Digest, Sha256};
    Sha256::digest(data)
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect()
}

/// Stdout then stderr, as one string.
pub fn both(out: &Output) -> String {
    format!("{}{}", stdout(out), stderr(out))
}
