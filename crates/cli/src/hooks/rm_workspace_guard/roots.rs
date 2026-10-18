// SPDX-FileCopyrightText: 2026 Igor Santos
// SPDX-License-Identifier: Apache-2.0

//! The roots the rm guard allows with no config, and the playbook data it
//! never allows. Everything here comes from one `RootEnv`, so the guard and
//! the tests see the same list, and a test can inject the home folder and the
//! environment for any OS.
//!
//! Three kinds of allowance:
//! - temp and cache folders per OS, contents only;
//! - playbook's own scratch folders under `~/.config/playbook`, contents only;
//! - playbook's install files by exact name (the binary, its links, its
//!   backups and staging files, the collector's service file), so playbook
//!   can repair its own install.
//!
//! The protected list beats every allowance, the configured ones included.

use std::path::{Path, PathBuf};

/// The OS whose temp folders apply.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Os {
    Mac,
    Linux,
    Windows,
    Other,
}

impl Os {
    pub fn current() -> Os {
        if cfg!(target_os = "macos") {
            Os::Mac
        } else if cfg!(target_os = "linux") {
            Os::Linux
        } else if cfg!(windows) {
            Os::Windows
        } else {
            Os::Other
        }
    }
}

/// Everything the built-in roots depend on.
pub struct RootEnv {
    pub os: Os,
    pub home: PathBuf,
    pub tmpdir: Option<PathBuf>,
    pub xdg_cache_home: Option<PathBuf>,
    /// `TEMP`, `TMP` and `LOCALAPPDATA`, read on Windows only.
    pub windows_temp: Vec<PathBuf>,
    pub local_app_data: Option<PathBuf>,
    pub temp_dir: PathBuf,
    pub uid: u32,
    pub bin_dir: PathBuf,
}

impl RootEnv {
    /// The values this process sees.
    pub fn from_process(home: &str) -> RootEnv {
        let var = |name: &str| {
            std::env::var_os(name)
                .filter(|v| !v.is_empty())
                .map(PathBuf::from)
        };
        let home = PathBuf::from(home);
        RootEnv {
            os: Os::current(),
            tmpdir: var("TMPDIR"),
            xdg_cache_home: var("XDG_CACHE_HOME"),
            windows_temp: ["TEMP", "TMP"].iter().filter_map(|n| var(n)).collect(),
            local_app_data: var("LOCALAPPDATA"),
            temp_dir: std::env::temp_dir(),
            uid: crate::service_manager::uid(),
            bin_dir: crate::uninstall::install_dir(&home),
            home,
        }
    }

    fn playbook_root(&self) -> PathBuf {
        crate::common::paths::playbook_root_from(&self.home)
    }
}

/// The temp and cache folders whose contents may be deleted, for the OS in
/// `env`. A value from the environment counts only when it is absolute.
pub fn temp_roots(env: &RootEnv) -> Vec<PathBuf> {
    let home = &env.home;
    let mut roots = vec![env.temp_dir.clone()];
    let mut push_env = |value: &Option<PathBuf>| {
        if let Some(path) = value.as_ref().filter(|p| is_absolute(p)) {
            roots.push(path.clone());
        }
    };
    match env.os {
        Os::Mac => {
            push_env(&env.tmpdir);
            roots.extend(["/tmp", "/private/tmp"].map(PathBuf::from));
            roots.push(home.join("Library/Caches"));
            roots.push(home.join(".cache"));
        }
        Os::Linux => {
            push_env(&env.tmpdir);
            roots.extend(["/tmp", "/var/tmp"].map(PathBuf::from));
            roots.push(home.join(".cache"));
            roots.push(PathBuf::from(format!("/run/user/{}", env.uid)));
            // Only under home: a cache home of `/` or `/usr` would hand the
            // allowance to the whole system.
            if let Some(cache) = env
                .xdg_cache_home
                .as_ref()
                .filter(|c| c.starts_with(home) && *c != home)
            {
                roots.push(cache.clone());
            }
        }
        Os::Windows => {
            for temp in &env.windows_temp {
                push_env(&Some(temp.clone()));
            }
            if let Some(local) = env.local_app_data.as_ref().filter(|p| is_absolute(p)) {
                roots.push(local.join("Temp"));
            }
        }
        Os::Other => {
            push_env(&env.tmpdir);
            roots.push(PathBuf::from("/tmp"));
            roots.push(home.join(".cache"));
        }
    }
    roots
}

/// Whether `path` is absolute here, or is a Windows path with a drive
/// letter, which a test on another OS still has to accept.
fn is_absolute(path: &Path) -> bool {
    let text = path.to_string_lossy();
    let drive = text.as_bytes();
    path.is_absolute()
        || (drive.len() >= 3
            && drive[0].is_ascii_alphabetic()
            && drive[1] == b':'
            && matches!(drive[2], b'/' | b'\\'))
}

/// The per-user macOS temp folder, `/private/var/folders/<xx>/<hash>/T`,
/// matched by shape because its name is a hash. `path` is resolved, so the
/// `/var` link is already followed.
pub fn macos_temp_base(path: &Path) -> Option<PathBuf> {
    let parts: Vec<_> = path.components().collect();
    let shape =
        parts.len() >= 7 && path.starts_with("/private/var/folders") && parts[6].as_os_str() == "T";
    shape.then(|| parts[..7].iter().collect())
}

/// The folders under `~/.config/playbook` that hold only scratch files
/// playbook makes again on demand: document scratch (`tmp`), a staging
/// folder (`staging`), session state (`runtime`), files unpacked from the
/// binary (`embedded`) and config drift markers (`cc-state`).
pub fn playbook_scratch_roots(env: &RootEnv) -> Vec<PathBuf> {
    let root = env.playbook_root();
    ["tmp", "staging", "runtime", "embedded", "cc-state"]
        .iter()
        .map(|name| root.join(name))
        .collect()
}

/// The `tmp` folder at any depth under `playbook_root` that `path` sits in,
/// for the `~/.config/playbook/**/tmp/**` allowance. The protected list is
/// checked first, so a `tmp` inside the memory store does not count.
pub fn playbook_tmp_base(path: &Path, playbook_root: &Path) -> Option<PathBuf> {
    let rest = path.strip_prefix(playbook_root).ok()?;
    let mut base = playbook_root.to_path_buf();
    for part in rest.components() {
        base.push(part);
        if part.as_os_str() == "tmp" {
            return Some(base);
        }
    }
    None
}

/// The playbook data that is never deleted, whatever root covers it: the
/// memory store, the databases, plans and handoffs, the effort ledger, the
/// system prompt and the config file. A parent of any of these is protected
/// too, which the caller checks.
pub fn protected(env: &RootEnv) -> Vec<PathBuf> {
    let root = env.playbook_root();
    [
        "memory",
        "data",
        "repos",
        "runtime/handoff",
        "effort",
        "prompts",
        "playbook.db",
        "playbook.db-wal",
        "playbook.db-shm",
        "config.json",
    ]
    .iter()
    .map(|name| root.join(name))
    .collect()
}

/// A file playbook installs and may remove or replace by name.
#[derive(Debug, PartialEq, Eq)]
pub enum InstallFile {
    /// This exact path.
    Exact(PathBuf),
    /// Any file in `dir` whose name starts with `prefix`.
    Prefix { dir: PathBuf, prefix: &'static str },
}

/// The binary and its links, backups (`playbook.<version>.bak`) and staging
/// files (`.playbook.*`) in the install folder, and the collector's service
/// file. A Homebrew install is left to Homebrew.
pub fn install_files(env: &RootEnv) -> Vec<InstallFile> {
    let mut files = Vec::new();
    let bin = &env.bin_dir;
    if bin.is_absolute() && !crate::update::is_homebrew(bin) {
        let exe = if env.os == Os::Windows {
            "playbook.exe"
        } else {
            "playbook"
        };
        for name in [exe, "pb", "pbtop"] {
            files.push(InstallFile::Exact(bin.join(name)));
        }
        for prefix in ["playbook.", ".playbook."] {
            files.push(InstallFile::Prefix {
                dir: bin.clone(),
                prefix,
            });
        }
    }
    use playbook_telemetry::service::Kind;
    match env.os {
        Os::Mac => files.push(InstallFile::Exact(Kind::Launchd.file(&env.home))),
        Os::Linux => files.push(InstallFile::Exact(Kind::Systemd.file(&env.home))),
        Os::Windows | Os::Other => {}
    }
    files
}

impl InstallFile {
    /// Whether `path`, resolved, is this file. `glob` is the last component
    /// when it holds a glob, which must keep this file's name prefix.
    pub fn covers(&self, path: &Path, glob: Option<&str>) -> bool {
        match (self, glob) {
            (InstallFile::Exact(file), None) => path == file,
            (InstallFile::Exact(_), Some(_)) => false,
            (InstallFile::Prefix { dir, prefix }, None) => {
                path.parent() == Some(dir.as_path())
                    && path
                        .file_name()
                        .is_some_and(|n| n.to_string_lossy().starts_with(prefix))
            }
            (InstallFile::Prefix { dir, prefix }, Some(pattern)) => {
                path == dir && pattern.starts_with(prefix)
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn env(os: Os) -> RootEnv {
        RootEnv {
            os,
            home: PathBuf::from("/h/me"),
            tmpdir: Some(PathBuf::from("/var/folders/ab/xyz/T/")),
            xdg_cache_home: Some(PathBuf::from("/h/me/.xdg-cache")),
            windows_temp: vec![PathBuf::from("C:/Users/me/AppData/Local/Temp")],
            local_app_data: Some(PathBuf::from("C:/Users/me/AppData/Local")),
            temp_dir: PathBuf::from("/sys-temp"),
            uid: 501,
            bin_dir: PathBuf::from("/h/me/.local/bin"),
        }
    }

    fn shown(roots: &[PathBuf]) -> Vec<String> {
        roots.iter().map(|r| r.display().to_string()).collect()
    }

    #[test]
    fn each_os_has_its_own_temp_roots() {
        let table: [(Os, &[&str]); 4] = [
            (
                Os::Mac,
                &[
                    "/sys-temp",
                    "/var/folders/ab/xyz/T/",
                    "/tmp",
                    "/private/tmp",
                    "/h/me/Library/Caches",
                    "/h/me/.cache",
                ],
            ),
            (
                Os::Linux,
                &[
                    "/sys-temp",
                    "/var/folders/ab/xyz/T/",
                    "/tmp",
                    "/var/tmp",
                    "/h/me/.cache",
                    "/run/user/501",
                    "/h/me/.xdg-cache",
                ],
            ),
            (
                Os::Windows,
                &[
                    "/sys-temp",
                    "C:/Users/me/AppData/Local/Temp",
                    "C:/Users/me/AppData/Local/Temp",
                ],
            ),
            (
                Os::Other,
                &[
                    "/sys-temp",
                    "/var/folders/ab/xyz/T/",
                    "/tmp",
                    "/h/me/.cache",
                ],
            ),
        ];
        for (os, want) in table {
            assert_eq!(shown(&temp_roots(&env(os))), want, "{os:?}");
        }
    }

    #[test]
    fn a_cache_home_outside_home_or_a_relative_tmpdir_is_ignored() {
        let mut linux = env(Os::Linux);
        linux.xdg_cache_home = Some(PathBuf::from("/"));
        linux.tmpdir = Some(PathBuf::from("relative/tmp"));

        let roots = shown(&temp_roots(&linux));

        assert!(!roots.contains(&"/".to_string()), "{roots:?}");
        assert!(
            !roots.iter().any(|r| r.starts_with("relative")),
            "{roots:?}"
        );
        linux.xdg_cache_home = Some(PathBuf::from("/h/me"));
        assert!(!shown(&temp_roots(&linux)).contains(&"/h/me".to_string()));
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn this_build_uses_the_macos_roots() {
        assert_eq!(Os::current(), Os::Mac);
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn this_build_uses_the_linux_roots() {
        assert_eq!(Os::current(), Os::Linux);
    }

    #[cfg(windows)]
    #[test]
    fn this_build_uses_the_windows_roots() {
        assert_eq!(Os::current(), Os::Windows);
    }

    #[test]
    fn the_macos_temp_folder_is_matched_by_shape() {
        let base = Some(PathBuf::from("/private/var/folders/ab/xyz/T"));
        assert_eq!(
            macos_temp_base(Path::new("/private/var/folders/ab/xyz/T/x")),
            base
        );
        assert_eq!(
            macos_temp_base(Path::new("/private/var/folders/ab/xyz/C/x")),
            None
        );
        assert_eq!(macos_temp_base(Path::new("/private/var/folders/ab")), None);
    }

    #[test]
    fn a_tmp_folder_at_any_depth_under_the_playbook_root_is_found() {
        let root = Path::new("/h/me/.config/playbook");
        let cases = [
            (
                "/h/me/.config/playbook/tmp/doc-1",
                Some("/h/me/.config/playbook/tmp"),
            ),
            (
                "/h/me/.config/playbook/a/b/tmp/x",
                Some("/h/me/.config/playbook/a/b/tmp"),
            ),
            ("/h/me/.config/playbook/a/tmpx/y", None),
            ("/h/me/.config/other/tmp/y", None),
        ];
        for (path, want) in cases {
            assert_eq!(
                playbook_tmp_base(Path::new(path), root),
                want.map(PathBuf::from),
                "{path}"
            );
        }
    }

    #[test]
    fn the_memory_store_and_databases_are_protected() {
        let shown = shown(&protected(&env(Os::Mac)));
        for want in ["memory", "data", "repos", "effort", "playbook.db"] {
            assert!(
                shown.contains(&format!("/h/me/.config/playbook/{want}")),
                "{want}: {shown:?}"
            );
        }
    }

    #[test]
    fn install_files_are_matched_by_exact_name_or_prefix() {
        let files = install_files(&env(Os::Mac));
        let any =
            |path: &str, glob: Option<&str>| files.iter().any(|f| f.covers(Path::new(path), glob));
        assert!(any("/h/me/.local/bin/playbook", None));
        assert!(any("/h/me/.local/bin/pb", None));
        assert!(any("/h/me/.local/bin/playbook.0.36.0.bak", None));
        assert!(any("/h/me/.local/bin/.playbook.update.1.2", None));
        assert!(any("/h/me/.local/bin", Some("playbook.*.bak")));
        assert!(any(
            "/h/me/Library/LaunchAgents/io.github.pragmatic-engineer.playbook.collector.plist",
            None
        ));
        assert!(!any("/h/me/.local/bin/other", None));
        assert!(!any("/h/me/.local/bin", Some("*")));
        assert!(!any("/h/me/.local/bin", Some("p*")));
        assert!(!any("/h/me/.local/bin/sub/playbook", None));
        assert!(!any("/h/me/Library/LaunchAgents/other.plist", None));
    }

    #[test]
    fn a_homebrew_install_is_left_to_homebrew() {
        let mut brew = env(Os::Mac);
        brew.bin_dir = PathBuf::from("/opt/homebrew/bin");

        let files = install_files(&brew);

        assert!(!files
            .iter()
            .any(|f| f.covers(Path::new("/opt/homebrew/bin/playbook"), None)));
    }

    #[test]
    fn linux_uses_the_systemd_unit() {
        let files = install_files(&env(Os::Linux));
        assert!(files.iter().any(|f| f.covers(
            Path::new("/h/me/.config/systemd/user/playbook-collector.service"),
            None
        )));
    }
}
