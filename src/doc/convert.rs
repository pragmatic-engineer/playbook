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
use std::os::unix::process::CommandExt;
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
    Interrupted(i32),
    Failed(String),
    Io(String),
}

impl DocError {
    /// A plain sentence for the person or model reading the result.
    pub fn message(&self) -> String {
        match self {
            Self::Absent => format!("anydoc is not installed. {}", install_hint()),
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
            Self::Interrupted(_) => "conversion interrupted".to_string(),
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
            Self::Interrupted(_) => "interrupted",
            Self::Failed(_) => "failed",
            Self::Io(_) => "io",
        }
    }
}

pub fn install_hint() -> &'static str {
    "To enable document conversion, install it yourself with: npm install --global @firecrawl/anydoc"
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

/// The arguments passed to the converter. Output goes to a file in the
/// private directory. No OCR, API key or URL flag is ever added here.
pub fn argv(out: &Path, input: &Path) -> Vec<OsString> {
    vec![
        OsString::from("-o"),
        out.as_os_str().to_os_string(),
        OsString::from("--"),
        input.as_os_str().to_os_string(),
    ]
}

/// The converter executable: the configured path when set, else the first
/// executable called `anydoc` on `path_var`.
pub fn find_anydoc(configured: &str, path_var: Option<&OsStr>) -> Option<PathBuf> {
    if !configured.is_empty() {
        let p = PathBuf::from(configured);
        return is_executable(&p).then_some(p);
    }
    std::env::split_paths(path_var?)
        .map(|dir| dir.join("anydoc"))
        .find(|p| is_executable(p))
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
    anydoc: &Path,
    input: &Path,
    limits: &Limits,
    scratch_root: &Path,
) -> Result<Converted, DocError> {
    format_of(input)?;
    let meta = fs::metadata(input).map_err(|e| DocError::Io(e.kind().to_string()))?;
    if !meta.is_file() {
        return Err(DocError::NotAFile);
    }
    if meta.len() > limits.max_input_bytes {
        return Err(DocError::TooLarge {
            bytes: meta.len(),
            max: limits.max_input_bytes,
        });
    }
    let input = fs::canonicalize(input).map_err(|e| DocError::Io(e.kind().to_string()))?;
    let scratch = TempDir::create(scratch_root).map_err(|e| DocError::Io(e.to_string()))?;
    let out = scratch.path().join("out.md");
    let err_file = scratch.path().join("err.txt");
    let isolation = detect_isolation();
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
    cmd.args(argv(&out, &input)).env_clear();
    for (k, v) in child_env(scratch.path()) {
        cmd.env(k, v);
    }
    let stderr = fs::File::create(&err_file).map_err(|e| DocError::Io(e.to_string()))?;
    cmd.stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::from(stderr))
        .current_dir(scratch.path())
        .process_group(0);
    // SAFETY: only `setrlimit` runs between fork and exec, which is async
    // signal safe. It caps the size of any file the child writes and turns
    // off core dumps. RLIMIT_AS is not used because Node reserves a large
    // virtual range, so the memory cap is the resident size watchdog below.
    unsafe {
        cmd.pre_exec(|| {
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
                return Err(DocError::MemoryLimit(limits.max_rss_kb / 1024));
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
        if status.code() == Some(3) {
            return Err(DocError::NeedsOcr);
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
