// SPDX-FileCopyrightText: 2026 Igor Santos
// SPDX-License-Identifier: Apache-2.0

//! Run the external `anydoc` converter in a bounded child process. The child
//! gets a scrubbed environment, no network where the OS can give that, a
//! wall clock timeout, a resident memory watchdog and a file size limit. The
//! converter is never given an OCR or hosted flag: its default is to refuse
//! pages that need OCR, which is the behavior wanted here.

use super::tmp::TempDir;
use std::ffi::{OsStr, OsString};
use std::fs;
use std::io::Read;
use std::os::unix::process::{CommandExt, ExitStatusExt};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicI32, Ordering};
use std::time::{Duration, Instant};

/// Formats the converter supports, by lowercase extension.
pub const FORMATS: &[&str] = &[
    "pdf", "docx", "doc", "odt", "rtf", "epub", "pptx", "ppt", "odp", "xlsx", "ods", "csv",
];

/// The converter version these safeguards were tested with.
pub const TESTED_VERSION: &str = "0.2";

/// The largest input accepted whatever `docs.maxBytes` says. Equal to the
/// ceiling the bundled converter enforces itself.
pub const HARD_MAX_INPUT_BYTES: u64 = 104_857_600;

/// Hard ceiling on the bytes read back from the converter's output file.
const READ_CEILING: u64 = 32 * 1024 * 1024;
/// Hard ceiling on any file the child may write.
const FILE_SIZE_LIMIT: u64 = 64 * 1024 * 1024;
const POLL: Duration = Duration::from_millis(20);
const RSS_EVERY: Duration = Duration::from_millis(100);

#[derive(Debug, Clone)]
pub struct Limits {
    pub max_input_bytes: u64,
    pub timeout: Duration,
    pub max_output_chars: usize,
    pub max_rss_kb: u64,
}

#[derive(Debug, PartialEq, Eq)]
pub enum DocError {
    Absent,
    UnsupportedExtension(String),
    NotAFile,
    TooLarge { bytes: u64, max: u64 },
    NeedsOcr,
    Timeout(u64),
    MemoryLimit(u64),
    ResourceLimit(String),
    Interrupted(i32),
    OutOfDate,
    Failed(String),
    Io(String),
}

impl DocError {
    /// A plain sentence for the person or model reading the result.
    pub fn message(&self) -> String {
        match self {
            Self::Absent => format!("no document converter found. {INSTALL_HINT}"),
            Self::UnsupportedExtension(ext) => format!(
                "unsupported file type `{ext}`. Supported: {}",
                FORMATS.join(", ")
            ),
            Self::NotAFile => "the path is not a regular file".to_string(),
            Self::TooLarge { bytes, max } => {
                format!("the file is {bytes} bytes, over the limit of {max} (docs.maxBytes)")
            }
            Self::NeedsOcr => {
                "needs OCR, not supported locally. The document has pages with no text layer. \
                 Playbook never sends documents to a hosted service."
                    .to_string()
            }
            Self::Timeout(s) => {
                format!("conversion stopped after {s} seconds (docs.timeoutSeconds)")
            }
            Self::MemoryLimit(mb) => {
                format!(
                    "conversion stopped, it used more than {mb} MB of memory (docs.maxMemoryMb)"
                )
            }
            Self::ResourceLimit(reason) => format!("the document hit a safety limit: {reason}"),
            Self::Interrupted(_) => "conversion interrupted".to_string(),
            Self::OutOfDate => OUT_OF_DATE_MESSAGE.to_string(),
            Self::Failed(why) => format!("conversion failed: {why}"),
            Self::Io(why) => format!("could not prepare the conversion: {why}"),
        }
    }

    /// Short stable code for `--json`.
    pub fn code(&self) -> &'static str {
        match self {
            Self::Absent => "absent",
            Self::UnsupportedExtension(_) => "unsupported",
            Self::NotAFile => "not_a_file",
            Self::TooLarge { .. } => "too_large",
            Self::NeedsOcr => "needs_ocr",
            Self::Timeout(_) => "timeout",
            Self::MemoryLimit(_) => "memory_limit",
            Self::ResourceLimit(_) => "resource_limit",
            Self::Interrupted(_) => "interrupted",
            Self::OutOfDate => "out_of_date",
            Self::Failed(_) => "failed",
            Self::Io(_) => "io",
        }
    }
}

/// Shown when no converter is found.
pub const INSTALL_HINT: &str = "the bundled converter pbdoc is missing next to the playbook binary. Reinstall playbook with install.sh or `brew reinstall playbook`, or run `playbook update`.";

/// Shown when a bundled converter exits 2 (it rejected its arguments).
pub const OUT_OF_DATE_MESSAGE: &str =
    "the bundled converter pbdoc is out of date for this playbook. Run `playbook update`.";

pub const STATUS_BUNDLED_PREFIX: &str = "converter: pbdoc (bundled) at ";
pub const STATUS_EXTERNAL_PREFIX: &str = "converter: anydoc (external) at ";
pub const STATUS_VERSION_PREFIX: &str = "version: pbdoc ";
pub const STATUS_EXTERNAL_VERSION_PREFIX: &str = "version: ";
pub const STATUS_ANYDOC_LINE: &str = "anydoc: 0.2.4";
pub const STATUS_NOT_FOUND_PREFIX: &str = "converter: not found. ";
pub const STATUS_OVERRIDE_NOTE: &str = "note: override active: the configured converter runs instead of the bundled one, and playbook does not check that it is offline";
pub const STATUS_NPM_NOTE: &str = "note: the bundled converter is used; you can remove the npm package with npm uninstall --global @firecrawl/anydoc";

pub const DOCTOR_ROW_BUNDLED: &str = "doc converter: pbdoc (bundled)";
pub const DOCTOR_ROW_EXTERNAL: &str =
    "doc converter: external anydoc, playbook does not check that it is offline";
pub const DOCTOR_ROW_NONE: &str = "doc converter: not found";
pub const DOCTOR_ROW_SKEW: &str = "doc converter: pbdoc version differs from playbook";

/// The bare version in the output of `pbdoc --version` (`pbdoc 1.2.3`).
pub fn pbdoc_number(version_output: &str) -> &str {
    version_output
        .strip_prefix(BUNDLED_NAME)
        .map_or(version_output, str::trim_start)
}

/// The status note for a bundled converter whose version is not playbook's.
pub fn version_skew_note(pbdoc: &str, playbook: &str) -> String {
    format!("note: pbdoc {pbdoc} does not match playbook {playbook}. Run playbook update.")
}

/// How the child was kept off the network.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Isolation {
    MacosSandbox,
    LinuxNamespace,
    /// No sandbox tool worked here. Only the environment was scrubbed.
    EnvironmentOnly,
}

impl Isolation {
    pub fn label(self) -> &'static str {
        match self {
            Self::MacosSandbox => "network blocked by the macOS sandbox",
            Self::LinuxNamespace => "network blocked by a Linux network namespace",
            Self::EnvironmentOnly => {
                "network isolation unavailable here, only the environment was scrubbed"
            }
        }
    }
}

pub struct Converted {
    pub markdown: String,
    pub total_chars: usize,
    pub truncated: bool,
    pub isolation: Isolation,
    pub version: String,
}

/// The extension in lowercase when it is a supported format.
pub fn format_of(path: &Path) -> Result<&'static str, DocError> {
    let ext = path
        .extension()
        .map(|e| e.to_string_lossy().to_lowercase())
        .unwrap_or_default();
    FORMATS
        .iter()
        .find(|f| **f == ext)
        .copied()
        .ok_or(DocError::UnsupportedExtension(ext))
}

const FLAG_OUTPUT: &str = "-o";
const FLAG_END_OF_FLAGS: &str = "--";
const FLAG_MAX_INPUT_BYTES: &str = "--max-input-bytes";
const FLAG_MAX_MEMORY_MB: &str = "--max-memory-mb";
const KB_PER_MB: u64 = 1024;
const BYTES_PER_KB: u64 = 1024;
/// A converter exits with this code when it rejects its arguments.
const EXIT_USAGE: i32 = 2;
const EXIT_NEEDS_OCR: i32 = 3;
/// The bundled helper exits with this code when it hits a safety limit.
const EXIT_RESOURCE_LIMIT: i32 = 4;
/// Added to `docs.maxMemoryMb` for the address space cap, so mapped libraries
/// and thread stacks do not trip it before the memory cap does.
const ADDRESS_SPACE_HEADROOM: u64 = 512 * 1024 * 1024;
const REASON_MEMORY: &str = "memory";
const REASON_CRASH: &str = "memory limit or crash";

/// The signals a child gets when it runs out of address space or memory.
fn is_memory_crash(sig: i32) -> bool {
    matches!(sig, libc::SIGABRT | libc::SIGSEGV | libc::SIGKILL)
}

/// The arguments passed to the converter. Output goes to a file in the
/// private directory. No OCR, API key or URL flag is ever added here. The
/// bundled helper also gets the size and memory limits.
pub fn argv(kind: Kind, limits: &Limits, out: &Path, input: &Path) -> Vec<OsString> {
    let mut args = Vec::new();
    if kind == Kind::Bundled {
        args.extend([
            OsString::from(FLAG_MAX_INPUT_BYTES),
            OsString::from(limits.max_input_bytes.to_string()),
            OsString::from(FLAG_MAX_MEMORY_MB),
            OsString::from((limits.max_rss_kb / KB_PER_MB).to_string()),
        ]);
    }
    args.extend([
        OsString::from(FLAG_OUTPUT),
        out.as_os_str().to_os_string(),
        OsString::from(FLAG_END_OF_FLAGS),
        input.as_os_str().to_os_string(),
    ]);
    args
}

/// Where a converter came from.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    Bundled,
    External,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Converter {
    pub path: PathBuf,
    pub kind: Kind,
}

const BUNDLED_NAME: &str = "pbdoc";
const EXTERNAL_NAME: &str = "anydoc";

/// The converter to run: the configured path when set, else `pbdoc` next to
/// `exe`, else the first executable called `anydoc` on `path_var`.
pub fn find_converter(
    configured: &str,
    exe: Option<&Path>,
    path_var: Option<&OsStr>,
) -> Option<Converter> {
    if !configured.is_empty() {
        let p = PathBuf::from(configured);
        return is_executable(&p).then_some(Converter {
            path: p,
            kind: Kind::External,
        });
    }
    find_bundled(exe?).or_else(|| find_on_path(path_var?))
}

/// The `pbdoc` beside `exe` as invoked, else beside its canonical location,
/// so a symlinked `playbook` still finds the helper in the install directory.
pub fn find_bundled(exe: &Path) -> Option<Converter> {
    let as_invoked = exe.parent().map(Path::to_path_buf);
    let canonical = fs::canonicalize(exe)
        .ok()
        .and_then(|p| p.parent().map(Path::to_path_buf));
    as_invoked
        .into_iter()
        .chain(canonical)
        .map(|dir| dir.join(BUNDLED_NAME))
        .find(|p| is_executable(p))
        .map(|path| Converter {
            path,
            kind: Kind::Bundled,
        })
}

/// The first executable `anydoc` on `path_var`.
pub fn find_on_path(path_var: &OsStr) -> Option<Converter> {
    std::env::split_paths(path_var)
        .map(|dir| dir.join(EXTERNAL_NAME))
        .find(|p| is_executable(p))
        .map(|path| Converter {
            path,
            kind: Kind::External,
        })
}

fn is_executable(path: &Path) -> bool {
    use std::os::unix::fs::PermissionsExt;
    path.metadata()
        .is_ok_and(|m| m.is_file() && m.permissions().mode() & 0o111 != 0)
}

/// The environment the child sees. Only what a Node or native program needs
/// to start. Proxy variables, API keys and `NODE_OPTIONS` are never passed.
fn child_env(scratch: &Path) -> Vec<(&'static str, OsString)> {
    let mut env: Vec<(&'static str, OsString)> = Vec::new();
    for key in ["PATH", "HOME", "LANG", "LC_ALL"] {
        if let Some(v) = std::env::var_os(key) {
            env.push((key, v));
        }
    }
    env.push(("TMPDIR", scratch.as_os_str().to_os_string()));
    env
}

const MACOS_PROFILE: &str = "(version 1)(allow default)(deny network*)";
const MACOS_SANDBOX: &str = "/usr/bin/sandbox-exec";

fn probe(mut cmd: Command) -> bool {
    cmd.stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null());
    crate::common::run_with_timeout(&mut cmd, Duration::from_secs(5))
        .is_some_and(|o| o.status.success())
}

fn detect_isolation() -> Isolation {
    if cfg!(target_os = "macos") && Path::new(MACOS_SANDBOX).exists() {
        let mut c = Command::new(MACOS_SANDBOX);
        c.args(["-p", MACOS_PROFILE, "/usr/bin/true"]);
        if probe(c) {
            return Isolation::MacosSandbox;
        }
    }
    if cfg!(target_os = "linux") {
        let mut c = Command::new("unshare");
        c.args(["--net", "--map-root-user", "true"]);
        if probe(c) {
            return Isolation::LinuxNamespace;
        }
    }
    Isolation::EnvironmentOnly
}

static SIGNAL: AtomicI32 = AtomicI32::new(0);

extern "C" fn on_signal(sig: libc::c_int) {
    SIGNAL.store(sig, Ordering::SeqCst);
}

/// Make SIGINT and SIGTERM set a flag the wait loop sees, so the child is
/// stopped and the scratch directory removed before the process exits.
pub fn install_signal_handlers() {
    SIGNAL.store(0, Ordering::SeqCst);
    for sig in [libc::SIGINT, libc::SIGTERM] {
        // SAFETY: the handler only stores into an atomic, which is signal safe.
        unsafe {
            libc::signal(sig, on_signal as *const () as libc::sighandler_t);
        }
    }
}

/// Resident memory in KB of every process in group `pgid`.
fn group_rss_kb(pgid: u32) -> u64 {
    let out = Command::new("ps")
        .args(["-axo", "pgid=,rss="])
        .stdin(Stdio::null())
        .stderr(Stdio::null())
        .output();
    let Ok(out) = out else { return 0 };
    String::from_utf8_lossy(&out.stdout)
        .lines()
        .filter_map(|l| {
            let mut it = l.split_whitespace();
            let g: u32 = it.next()?.parse().ok()?;
            let r: u64 = it.next()?.parse().ok()?;
            (g == pgid).then_some(r)
        })
        .sum()
}

fn kill_group(child: &mut std::process::Child) {
    // SAFETY: `killpg` only reads its integer arguments.
    unsafe {
        libc::killpg(child.id() as libc::pid_t, libc::SIGKILL);
    }
    let _ = child.kill();
    let _ = child.wait();
}

/// The converter's version, or an empty string when it will not say.
pub fn version_of(anydoc: &Path, scratch: &Path) -> String {
    let mut cmd = Command::new(anydoc);
    cmd.arg("--version").env_clear().stdin(Stdio::null());
    for (k, v) in child_env(scratch) {
        cmd.env(k, v);
    }
    crate::common::run_with_timeout(&mut cmd, Duration::from_secs(5))
        .filter(|o| o.status.success())
        .map(|o| {
            String::from_utf8_lossy(&o.stdout)
                .trim()
                .chars()
                .take(32)
                .collect()
        })
        .unwrap_or_default()
}

/// Convert `input` and return the Markdown. The scratch directory lives under
/// `scratch_root` and is gone when this returns, whatever the outcome.
pub fn convert(
    converter: &Converter,
    input: &Path,
    limits: &Limits,
    scratch_root: &Path,
) -> Result<Converted, DocError> {
    format_of(input)?;
    let meta = fs::metadata(input).map_err(|e| DocError::Io(e.kind().to_string()))?;
    if !meta.is_file() {
        return Err(DocError::NotAFile);
    }
    let max_input = limits.max_input_bytes.min(HARD_MAX_INPUT_BYTES);
    if meta.len() > max_input {
        return Err(DocError::TooLarge {
            bytes: meta.len(),
            max: max_input,
        });
    }
    let input = fs::canonicalize(input).map_err(|e| DocError::Io(e.kind().to_string()))?;
    let scratch = TempDir::create(scratch_root).map_err(|e| DocError::Io(e.to_string()))?;
    let out = scratch.path().join("out.md");
    let err_file = scratch.path().join("err.txt");
    let isolation = detect_isolation();
    let anydoc = converter.path.as_path();
    let version = version_of(anydoc, scratch.path());

    let mut cmd = match isolation {
        Isolation::MacosSandbox => {
            let mut c = Command::new(MACOS_SANDBOX);
            c.args(["-p", MACOS_PROFILE]).arg(anydoc);
            c
        }
        Isolation::LinuxNamespace => {
            let mut c = Command::new("unshare");
            c.args(["--net", "--map-root-user"]).arg(anydoc);
            c
        }
        Isolation::EnvironmentOnly => Command::new(anydoc),
    };
    cmd.args(argv(converter.kind, limits, &out, &input))
        .env_clear();
    for (k, v) in child_env(scratch.path()) {
        cmd.env(k, v);
    }
    let stderr = fs::File::create(&err_file).map_err(|e| DocError::Io(e.to_string()))?;
    cmd.stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::from(stderr))
        .current_dir(scratch.path())
        .process_group(0);
    // Only the bundled helper gets an address space cap, because Node reserves
    // a large virtual range and would never start under one. macOS accepts
    // neither RLIMIT_AS nor RLIMIT_DATA, so there the helper's own heap cap and
    // watchdog plus the resident size watchdog below are the memory limits.
    let address_space_cap = (cfg!(target_os = "linux") && converter.kind == Kind::Bundled)
        .then(|| (limits.max_rss_kb * BYTES_PER_KB + ADDRESS_SPACE_HEADROOM) as libc::rlim_t);
    // SAFETY: only `setrlimit` runs between fork and exec, which is async
    // signal safe. It caps the size of any file the child writes, turns off
    // core dumps and applies the address space cap when there is one.
    unsafe {
        cmd.pre_exec(move || {
            #[cfg(target_os = "linux")]
            if let Some(bytes) = address_space_cap {
                let cap = libc::rlimit {
                    rlim_cur: bytes,
                    rlim_max: bytes,
                };
                libc::setrlimit(libc::RLIMIT_AS, &cap);
            }
            #[cfg(not(target_os = "linux"))]
            let _ = address_space_cap;
            let cap = libc::rlimit {
                rlim_cur: FILE_SIZE_LIMIT as libc::rlim_t,
                rlim_max: FILE_SIZE_LIMIT as libc::rlim_t,
            };
            libc::setrlimit(libc::RLIMIT_FSIZE, &cap);
            let none = libc::rlimit {
                rlim_cur: 0,
                rlim_max: 0,
            };
            libc::setrlimit(libc::RLIMIT_CORE, &none);
            Ok(())
        });
    }
    let mut child = cmd.spawn().map_err(|e| DocError::Io(e.to_string()))?;

    let start = Instant::now();
    let mut last_rss = Instant::now();
    let status = loop {
        let sig = SIGNAL.load(Ordering::SeqCst);
        if sig != 0 {
            kill_group(&mut child);
            return Err(DocError::Interrupted(sig));
        }
        match child.try_wait() {
            Ok(Some(status)) => break status,
            Ok(None) => {}
            Err(e) => {
                kill_group(&mut child);
                return Err(DocError::Io(e.to_string()));
            }
        }
        if start.elapsed() >= limits.timeout {
            kill_group(&mut child);
            return Err(DocError::Timeout(limits.timeout.as_secs()));
        }
        if last_rss.elapsed() >= RSS_EVERY {
            last_rss = Instant::now();
            if group_rss_kb(child.id()) > limits.max_rss_kb {
                kill_group(&mut child);
                return Err(if converter.kind == Kind::Bundled {
                    DocError::ResourceLimit(REASON_MEMORY.to_string())
                } else {
                    DocError::MemoryLimit(limits.max_rss_kb / 1024)
                });
            }
        }
        std::thread::sleep(POLL);
    };
    // Anything the converter left running in its group goes too.
    // SAFETY: `killpg` only reads its integer arguments.
    unsafe {
        libc::killpg(child.id() as libc::pid_t, libc::SIGKILL);
    }

    if !status.success() {
        match status.code() {
            Some(EXIT_NEEDS_OCR) => return Err(DocError::NeedsOcr),
            Some(EXIT_RESOURCE_LIMIT) if converter.kind == Kind::Bundled => {
                return Err(DocError::ResourceLimit(first_line(&err_file)));
            }
            Some(EXIT_USAGE) if converter.kind == Kind::Bundled => return Err(DocError::OutOfDate),
            _ => {}
        }
        if converter.kind == Kind::Bundled && status.signal().is_some_and(is_memory_crash) {
            return Err(DocError::ResourceLimit(REASON_CRASH.to_string()));
        }
        return Err(DocError::Failed(first_line(&err_file)));
    }

    let meta = fs::symlink_metadata(&out).map_err(|_| DocError::Failed("no output".into()))?;
    if !meta.is_file() {
        return Err(DocError::Failed("the output is not a regular file".into()));
    }
    let mut bytes = Vec::new();
    fs::File::open(&out)
        .and_then(|f| f.take(READ_CEILING).read_to_end(&mut bytes))
        .map_err(|e| DocError::Io(e.to_string()))?;
    let text = String::from_utf8_lossy(&bytes);
    let total_chars = text.chars().count();
    let (markdown, truncated) = if total_chars > limits.max_output_chars {
        (text.chars().take(limits.max_output_chars).collect(), true)
    } else {
        (text.into_owned(), false)
    };
    Ok(Converted {
        markdown,
        total_chars,
        truncated: truncated || meta.len() > READ_CEILING,
        isolation,
        version,
    })
}

/// The first line of the converter's error output, printable and short.
fn first_line(path: &Path) -> String {
    let mut buf = Vec::new();
    let _ = fs::File::open(path).and_then(|f| f.take(2048).read_to_end(&mut buf));
    let text = String::from_utf8_lossy(&buf);
    let line: String = text
        .lines()
        .next()
        .unwrap_or("the converter exited with an error")
        .chars()
        .filter(|c| !c.is_control())
        .take(200)
        .collect();
    if line.is_empty() {
        "the converter exited with an error".to_string()
    } else {
        line
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::os::unix::fs::{symlink, PermissionsExt};
    use std::sync::atomic::AtomicU64;

    static COUNTER: AtomicU64 = AtomicU64::new(0);

    /// A scratch directory removed on drop.
    struct Scratch(PathBuf);

    impl Scratch {
        fn new() -> Self {
            let n = COUNTER.fetch_add(1, Ordering::Relaxed);
            let dir = std::env::temp_dir().join(format!(
                "playbook-find-converter-{}-{n}",
                std::process::id()
            ));
            fs::create_dir_all(&dir).unwrap();
            // Resolve the temp dir symlink (macOS /var) so canonical paths compare equal.
            Self(dir.canonicalize().unwrap())
        }

        fn file(&self, rel: &str, mode: u32) -> PathBuf {
            let p = self.0.join(rel);
            fs::create_dir_all(p.parent().unwrap()).unwrap();
            fs::write(&p, "#!/bin/sh\n").unwrap();
            fs::set_permissions(&p, fs::Permissions::from_mode(mode)).unwrap();
            p
        }
    }

    impl Drop for Scratch {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    const EXEC: u32 = 0o755;
    const PLAIN: u32 = 0o644;

    fn bundled(path: PathBuf) -> Option<Converter> {
        Some(Converter {
            path,
            kind: Kind::Bundled,
        })
    }

    fn external(path: PathBuf) -> Option<Converter> {
        Some(Converter {
            path,
            kind: Kind::External,
        })
    }

    #[test]
    fn a_configured_path_wins_over_a_bundled_sibling() {
        // Arrange
        let s = Scratch::new();
        let exe = s.file("bin/playbook", EXEC);
        s.file("bin/pbdoc", EXEC);
        let configured = s.file("custom/anydoc", EXEC);

        // Act
        let found = find_converter(configured.to_str().unwrap(), Some(&exe), None);

        // Assert
        assert_eq!(found, external(configured));
    }

    #[test]
    fn a_bundled_sibling_wins_over_anydoc_on_path() {
        // Arrange
        let s = Scratch::new();
        let exe = s.file("bin/playbook", EXEC);
        let pbdoc = s.file("bin/pbdoc", EXEC);
        s.file("npm/anydoc", EXEC);
        let path_var = s.0.join("npm");

        // Act
        let found = find_converter("", Some(&exe), Some(path_var.as_os_str()));

        // Assert
        assert_eq!(found, bundled(pbdoc));
    }

    #[test]
    fn a_sibling_that_is_not_executable_falls_back_to_anydoc_on_path() {
        // Arrange
        let s = Scratch::new();
        let exe = s.file("bin/playbook", EXEC);
        s.file("bin/pbdoc", PLAIN);
        let anydoc = s.file("npm/anydoc", EXEC);
        let path_var = s.0.join("npm");

        // Act
        let found = find_converter("", Some(&exe), Some(path_var.as_os_str()));

        // Assert
        assert_eq!(found, external(anydoc));
    }

    #[test]
    fn nothing_found_gives_none() {
        // Arrange
        let s = Scratch::new();
        let exe = s.file("bin/playbook", EXEC);
        let empty_path = s.0.join("empty");

        // Act
        let found = find_converter("", Some(&exe), Some(empty_path.as_os_str()));

        // Assert
        assert_eq!(found, None);
    }

    #[test]
    fn an_absolute_symlink_to_the_binary_finds_pbdoc_in_the_canonical_dir() {
        // Arrange
        let s = Scratch::new();
        let target = s.file("cellar/bin/playbook", EXEC);
        let pbdoc = s.file("cellar/bin/pbdoc", EXEC);
        fs::create_dir_all(s.0.join("bin")).unwrap();
        let link = s.0.join("bin/playbook");
        symlink(&target, &link).unwrap();

        // Act
        let found = find_converter("", Some(&link), None);

        // Assert
        assert_eq!(found, bundled(pbdoc));
    }

    #[test]
    fn a_relative_symlink_to_the_binary_finds_pbdoc_in_the_canonical_dir() {
        // Arrange
        let s = Scratch::new();
        s.file("cellar/bin/playbook", EXEC);
        let pbdoc = s.file("cellar/bin/pbdoc", EXEC);
        fs::create_dir_all(s.0.join("bin")).unwrap();
        let link = s.0.join("bin/playbook");
        symlink("../cellar/bin/playbook", &link).unwrap();

        // Act
        let found = find_converter("", Some(&link), None);

        // Assert
        assert_eq!(found, bundled(pbdoc));
    }

    #[test]
    fn a_pbdoc_next_to_the_symlink_is_found_before_the_canonical_dir() {
        // Arrange
        let s = Scratch::new();
        let target = s.file("cellar/bin/playbook", EXEC);
        s.file("cellar/bin/pbdoc", EXEC);
        fs::create_dir_all(s.0.join("bin")).unwrap();
        let link = s.0.join("bin/playbook");
        symlink(&target, &link).unwrap();
        let beside_link = s.file("bin/pbdoc", EXEC);

        // Act
        let found = find_converter("", Some(&link), None);

        // Assert
        assert_eq!(found, bundled(beside_link));
    }
}
