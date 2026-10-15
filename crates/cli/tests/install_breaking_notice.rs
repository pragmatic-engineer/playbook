// SPDX-FileCopyrightText: 2026 Igor Santos
// SPDX-License-Identifier: Apache-2.0

//! The breaking changes notice and the `playbook migrate` hint that
//! `install.sh` gives when it replaces an older binary (ADR 0035). The
//! installer runs as the real script under bash, with a stub `curl` serving a
//! fake release and a stub `uname` pinning the platform. `$HOME` and
//! `$PLAYBOOK_BIN_DIR` are scratch directories and every inherited
//! `PLAYBOOK_*`, `CLAUDE_*`, `XDG_*` and `CI` variable is removed.

#![cfg(unix)]

use std::fs;
use std::io::Write;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicU64, Ordering};

static SCRATCH_COUNTER: AtomicU64 = AtomicU64::new(0);

const CURL_STUB: &str = r#"#!/usr/bin/env bash
out=""; url=""
while [ $# -gt 0 ]; do
  case "$1" in
    -o) out="$2"; shift 2 ;;
    -w) shift 2 ;;
    -*) shift ;;
    *)  url="$1"; shift ;;
  esac
done
emit() { if [ -n "$out" ]; then printf '%s' "$1" > "$out"; else printf '%s' "$1"; fi; }
case "$url" in
  *api.github.com/repos/*/releases/latest)
    emit "$STUB_BODY"
    if [ -n "$out" ]; then printf '200'; fi
    exit 0 ;;
  */releases/download/*/SHA256SUMS) emit "$STUB_SUMS_BODY"; exit 0 ;;
  */releases/download/*)            emit "$STUB_ASSET_BODY"; exit 0 ;;
  *) exit 22 ;;
esac
"#;

const UNAME_STUB: &str = r#"#!/usr/bin/env bash
case "$1" in
  -s) printf 'Linux\n' ;;
  -m) printf 'x86_64\n' ;;
  *) command -p uname "$@" ;;
esac
"#;

/// A 0.17.0 binary that knows `breaking` and `migrate`. It lists one entry
/// only for `breaking --since 0.16.0`, the version it replaces.
const NEW_BINARY: &str = r#"#!/usr/bin/env bash
case "$*" in
  "breaking --since 0.16.0") printf 'Breaking changes in this upgrade:\n  playbook 0.17.0: The old flag is gone.\n    Action: playbook init\n' ;;
  "migrate --help") exit 0 ;;
  breaking*|migrate*) exit 2 ;;
  *) echo "playbook 0.17.0" ;;
esac"#;

/// A 0.17.0 binary from before the `breaking` command: it answers every
/// call with its version.
const OLD_STYLE_BINARY: &str = "#!/usr/bin/env bash\necho \"playbook 0.17.0\"";

struct Sandbox {
    root: PathBuf,
    stubs: PathBuf,
    home: PathBuf,
    bin_dir: PathBuf,
}

impl Sandbox {
    fn new(tag: &str) -> Self {
        let n = SCRATCH_COUNTER.fetch_add(1, Ordering::Relaxed);
        let root = std::env::temp_dir().join(format!(
            "playbook-install-breaking-{}-{tag}-{n}",
            playbook::testing::run_id()
        ));
        let stubs = root.join("stubs");
        let home = root.join("home");
        let bin_dir = home.join("bin");
        write_executable(&stubs.join("curl"), CURL_STUB);
        write_executable(&stubs.join("uname"), UNAME_STUB);
        fs::create_dir_all(&home).expect("scratch home should be creatable");
        Self {
            root,
            stubs,
            home,
            bin_dir,
        }
    }

    /// Puts an installed playbook `version` in the bin dir.
    fn with_installed(self, version: &str) -> Self {
        write_executable(
            &self.bin_dir.join("playbook"),
            &format!("#!/usr/bin/env bash\necho \"playbook {version}\"\n"),
        );
        self
    }

    /// Installs `body` as release 0.17.0, returning stdout then stderr.
    fn install(&self, body: &str) -> String {
        let sums = format!(
            "{}  playbook-0.17.0-x86_64-unknown-linux-musl",
            sha256_hex(body)
        );
        let mut path = vec![self.stubs.clone()];
        path.extend(std::env::split_paths(
            &std::env::var_os("PATH").unwrap_or_default(),
        ));
        let mut command = Command::new("bash");
        command
            .arg(Path::new(env!("PLAYBOOK_REPO_ROOT")).join("install.sh"))
            .arg("--no-init")
            .current_dir(&self.root)
            .stdin(Stdio::null());
        for (key, _) in std::env::vars_os() {
            let key_text = key.to_string_lossy();
            if ["PLAYBOOK_", "CLAUDE_", "XDG_"]
                .iter()
                .any(|prefix| key_text.starts_with(prefix))
                || key_text == "CI"
            {
                command.env_remove(&key);
            }
        }
        command
            .env("HOME", &self.home)
            .env("CLAUDE_HOME", self.home.join(".claude"))
            .env("PLAYBOOK_BIN_DIR", &self.bin_dir)
            .env(
                "PATH",
                std::env::join_paths(path).expect("PATH should join"),
            )
            .env("SHELL", "/bin/bash")
            .env("STUB_BODY", "{\"tag_name\": \"v0.17.0\"}")
            .env("STUB_ASSET_BODY", body)
            .env("STUB_SUMS_BODY", sums);
        let out = command.output().expect("bash should spawn");
        let text = format!(
            "{}{}",
            String::from_utf8_lossy(&out.stdout),
            String::from_utf8_lossy(&out.stderr)
        );
        assert!(
            text.contains("Installed playbook 0.17.0"),
            "install did not reach the end of the binary step: {text}"
        );
        text
    }
}

impl Drop for Sandbox {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.root);
    }
}

fn write_executable(path: &Path, content: &str) {
    fs::create_dir_all(path.parent().unwrap()).expect("parent dir should be creatable");
    fs::write(path, content).expect("scratch file should be writable");
    fs::set_permissions(path, fs::Permissions::from_mode(0o755))
        .expect("scratch file should be chmod-able");
}

fn sha256_hex(content: &str) -> String {
    let (program, args): (&str, &[&str]) =
        if Command::new("shasum").arg("--version").output().is_ok() {
            ("shasum", &["-a", "256"])
        } else {
            ("sha256sum", &[])
        };
    let mut child = Command::new(program)
        .args(args)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .spawn()
        .expect("a sha256 tool should spawn");
    child
        .stdin
        .take()
        .unwrap()
        .write_all(content.as_bytes())
        .expect("sha256 tool should accept input");
    let out = child.wait_with_output().expect("sha256 tool should finish");
    String::from_utf8_lossy(&out.stdout)
        .split_whitespace()
        .next()
        .expect("sha256 tool should print a digest")
        .to_string()
}

#[test]
fn an_upgrade_prints_the_breaking_changes_since_the_old_version() {
    let sandbox = Sandbox::new("upgrade").with_installed("0.16.0");

    let out = sandbox.install(NEW_BINARY);

    assert!(
        out.contains("Breaking changes in this upgrade:\n  playbook 0.17.0: The old flag is gone.\n    Action: playbook init"),
        "{out}"
    );
}

#[test]
fn a_fresh_install_prints_no_breaking_changes_and_no_migrate_step() {
    let sandbox = Sandbox::new("fresh");

    let out = sandbox.install(NEW_BINARY);

    assert!(!out.contains("Breaking changes"), "{out}");
    assert!(!out.contains("playbook migrate"), "{out}");
}

#[test]
fn a_binary_without_the_breaking_command_prints_nothing_extra() {
    let sandbox = Sandbox::new("old-style").with_installed("0.16.0");

    let out = sandbox.install(OLD_STYLE_BINARY);

    assert!(!out.contains("Breaking changes"), "{out}");
    assert!(!out.lines().any(|l| l == "playbook 0.17.0"), "{out}");
}

#[test]
fn an_upgrade_with_no_init_names_playbook_migrate_without_running_it() {
    let sandbox = Sandbox::new("migrate").with_installed("0.16.0");

    let out = sandbox.install(NEW_BINARY);

    assert!(out.contains("Run: playbook migrate"), "{out}");
    assert!(!out.contains("Running playbook migrate"), "{out}");
}
