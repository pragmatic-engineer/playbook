// SPDX-FileCopyrightText: 2026 Igor Santos
// SPDX-License-Identifier: Apache-2.0

//! Puts the directory holding the `playbook` binary on PATH for every shell
//! start, so hooks and the `ccc` launcher find it whichever route installed it
//! (installer, Homebrew, or the Claude Code marketplace plus the first run of
//! `playbook init`).
//!
//! Hook commands run in a non-interactive shell, which never reads `.zshrc`.
//! So zsh gets `~/.zshenv`, bash gets `~/.bashrc` and its login file, and fish
//! gets its own `conf.d` file. Each block starts with the `# playbook binary`
//! marker, which `playbook uninstall --remove-binary` keys on.

use crate::init::shim::{self, RcStrip};
use std::ffi::OsStr;
use std::fs;
use std::io::{self, Write};
use std::path::{Path, PathBuf};

/// Comment that opens the block, shared with the installer script.
pub const MARKER: &str = "# playbook binary";

/// Directories every shell already has on PATH, so there is nothing to add.
const SYSTEM_DIRS: [&str; 4] = ["/usr/bin", "/bin", "/usr/sbin", "/sbin"];

/// The shell the PATH block is written for.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PathShell {
    Zsh,
    Bash,
    Fish,
    /// Any other shell: the POSIX `~/.profile`.
    Other,
}

impl PathShell {
    /// Detect the shell from a `$SHELL` value such as `/bin/zsh`.
    pub fn detect(shell_env: &str) -> PathShell {
        match shell_env.rsplit('/').next().unwrap_or(shell_env) {
            "zsh" => PathShell::Zsh,
            "bash" => PathShell::Bash,
            "fish" => PathShell::Fish,
            _ => PathShell::Other,
        }
    }
}

/// What the caller resolved: where the binary lives and which shell to wire.
#[derive(Debug, Clone)]
pub struct Setup {
    pub bin_dir: PathBuf,
    pub shell: PathShell,
}

/// What `ensure` did.
#[derive(Debug, Default, PartialEq, Eq)]
pub struct Outcome {
    /// Files this call changed.
    pub changed: Vec<PathBuf>,
    /// Files that already carried the directory.
    pub already: Vec<PathBuf>,
    /// Files left alone, with the reason.
    pub skipped: Vec<(PathBuf, String)>,
}

/// The directory to put on PATH: the directory of the first `playbook` on
/// `path_var` that is the running binary (so a Homebrew symlink dir wins over
/// the Cellar path), else the directory of the running binary itself.
pub fn resolve_bin_dir(path_var: &OsStr, exe: &Path) -> PathBuf {
    let real_exe = fs::canonicalize(exe).unwrap_or_else(|_| exe.to_path_buf());
    for dir in std::env::split_paths(path_var) {
        let candidate = dir.join("playbook");
        if dir.is_absolute() && fs::canonicalize(&candidate).is_ok_and(|c| c == real_exe) {
            return dir;
        }
    }
    exe.parent().map(Path::to_path_buf).unwrap_or_default()
}

/// The files the PATH block goes into for `shell`.
pub fn targets(home: &Path, shell: PathShell) -> Vec<PathBuf> {
    match shell {
        PathShell::Zsh => vec![home.join(".zshenv")],
        PathShell::Bash => {
            let login = [".bash_profile", ".bash_login", ".profile"]
                .iter()
                .map(|n| home.join(n))
                .find(|p| p.exists())
                .unwrap_or_else(|| home.join(".bash_profile"));
            vec![home.join(".bashrc"), login]
        }
        PathShell::Fish => vec![fish_file(home)],
        PathShell::Other => vec![home.join(".profile")],
    }
}

fn fish_file(home: &Path) -> PathBuf {
    home.join(".config/fish/conf.d/playbook.fish")
}

fn block(shell: PathShell, dir: &str) -> String {
    match shell {
        PathShell::Fish => format!("{MARKER}\nfish_add_path -g \"{dir}\"\n"),
        _ => format!("{MARKER}\nexport PATH=\"{dir}:$PATH\"\n"),
    }
}

/// A directory the block can quote safely.
fn quotable(dir: &str) -> bool {
    !dir.is_empty() && !dir.contains(['"', '$', '`', '\\', '\n'])
}

/// Make every target file for `setup.shell` carry `setup.bin_dir`. Idempotent:
/// a file that already names the directory is left as it is.
pub fn ensure(home: &Path, setup: &Setup) -> io::Result<Outcome> {
    let mut out = Outcome::default();
    let dir = setup.bin_dir.to_string_lossy().into_owned();
    if SYSTEM_DIRS.contains(&dir.as_str()) {
        return Ok(out);
    }
    for file in targets(home, setup.shell) {
        if !quotable(&dir) {
            out.skipped.push((
                file,
                format!("{dir} has characters a shell line cannot quote"),
            ));
            continue;
        }
        let existing = fs::read(&file).ok();
        if existing.as_deref().is_some_and(|c| names_dir(c, &dir)) {
            out.already.push(file);
            continue;
        }
        if existing.is_some() && !shim::owner_can_write(&file) {
            out.skipped.push((file, "not writable".to_string()));
            continue;
        }
        if let Some(parent) = file.parent() {
            fs::create_dir_all(parent)?;
        }
        let mut handle = fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(&file)?;
        write!(handle, "\n{}", block(setup.shell, &dir))?;
        out.changed.push(file);
    }
    Ok(out)
}

/// Whether some non-comment line of `content` mentions `dir`.
fn names_dir(content: &[u8], dir: &str) -> bool {
    content.split(|&b| b == b'\n').any(|line| {
        let line = line.trim_ascii();
        !line.starts_with(b"#") && line.windows(dir.len()).any(|w| w == dir.as_bytes())
    })
}

/// Remove the PATH block from every file `ensure` may have written, for
/// `playbook uninstall --remove-binary`. The fish file is deleted when it is
/// ours (it holds the marker). Every other file keeps its remaining content.
pub fn strip(home: &Path, stamp: u64, dry_run: bool) -> Vec<RcStrip> {
    let mut changed = shim::strip_binary_path_files(
        &[
            home.join(".zshenv"),
            home.join(".bash_profile"),
            home.join(".bash_login"),
            home.join(".profile"),
        ],
        stamp,
        dry_run,
    );
    let fish = fish_file(home);
    if fs::read(&fish).is_ok_and(|c| c.windows(MARKER.len()).any(|w| w == MARKER.as_bytes())) {
        let mut entry = RcStrip {
            rc_file: fish.clone(),
            backup: None,
            unwritable: false,
            error: None,
        };
        if !dry_run {
            let backup = fish.with_file_name(format!("playbook.fish.bak-{stamp}"));
            match fs::copy(&fish, &backup).and_then(|_| fs::remove_file(&fish)) {
                Ok(()) => entry.backup = Some(backup),
                Err(err) => entry.error = Some(err.to_string()),
            }
        }
        changed.push(entry);
    }
    changed
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::common::test_support::scratch_dir as raw_scratch;

    fn scratch_dir(tag: &str) -> PathBuf {
        let dir = raw_scratch(tag);
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn setup(shell: PathShell, dir: &str) -> Setup {
        Setup {
            bin_dir: PathBuf::from(dir),
            shell,
        }
    }

    #[test]
    fn zsh_gets_zshenv_so_non_interactive_shells_see_it() {
        let home = scratch_dir("path-zsh");
        let out = ensure(&home, &setup(PathShell::Zsh, "/opt/pb/bin")).unwrap();
        assert_eq!(out.changed, vec![home.join(".zshenv")]);
        let body = fs::read_to_string(home.join(".zshenv")).unwrap();
        assert!(body.contains("# playbook binary\nexport PATH=\"/opt/pb/bin:$PATH\"\n"));
    }

    #[test]
    fn bash_gets_bashrc_and_the_existing_login_file() {
        let home = scratch_dir("path-bash");
        fs::write(home.join(".profile"), "# mine\n").unwrap();
        let out = ensure(&home, &setup(PathShell::Bash, "/opt/pb/bin")).unwrap();
        assert_eq!(
            out.changed,
            vec![home.join(".bashrc"), home.join(".profile")]
        );
        assert!(!home.join(".bash_profile").exists());
    }

    #[test]
    fn bash_creates_bash_profile_when_no_login_file_exists() {
        let home = scratch_dir("path-bash-new");
        ensure(&home, &setup(PathShell::Bash, "/opt/pb/bin")).unwrap();
        assert!(home.join(".bash_profile").exists());
    }

    #[test]
    fn fish_gets_its_own_conf_d_file() {
        let home = scratch_dir("path-fish");
        ensure(&home, &setup(PathShell::Fish, "/opt/pb/bin")).unwrap();
        let body = fs::read_to_string(fish_file(&home)).unwrap();
        assert!(body.contains("fish_add_path -g \"/opt/pb/bin\""));
    }

    #[test]
    fn a_second_run_changes_nothing() {
        let home = scratch_dir("path-idem");
        let s = setup(PathShell::Zsh, "/opt/pb/bin");
        ensure(&home, &s).unwrap();
        let first = fs::read(home.join(".zshenv")).unwrap();
        let out = ensure(&home, &s).unwrap();
        assert!(out.changed.is_empty());
        assert_eq!(out.already, vec![home.join(".zshenv")]);
        assert_eq!(fs::read(home.join(".zshenv")).unwrap(), first);
    }

    #[test]
    fn a_dir_the_user_already_exports_is_left_alone() {
        let home = scratch_dir("path-user");
        fs::write(
            home.join(".zshenv"),
            "export PATH=\"$HOME/x:/opt/pb/bin:$PATH\"\n",
        )
        .unwrap();
        let out = ensure(&home, &setup(PathShell::Zsh, "/opt/pb/bin")).unwrap();
        assert!(out.changed.is_empty());
    }

    #[test]
    fn a_commented_mention_does_not_count() {
        let home = scratch_dir("path-comment");
        fs::write(home.join(".zshenv"), "# /opt/pb/bin\n").unwrap();
        let out = ensure(&home, &setup(PathShell::Zsh, "/opt/pb/bin")).unwrap();
        assert_eq!(out.changed.len(), 1);
    }

    #[test]
    fn system_dirs_need_nothing() {
        let home = scratch_dir("path-sys");
        let out = ensure(&home, &setup(PathShell::Zsh, "/usr/bin")).unwrap();
        assert_eq!(out, Outcome::default());
        assert!(!home.join(".zshenv").exists());
    }

    #[test]
    fn an_unquotable_dir_is_skipped_not_written() {
        let home = scratch_dir("path-quote");
        let out = ensure(&home, &setup(PathShell::Zsh, "/opt/$(x)/bin")).unwrap();
        assert_eq!(out.skipped.len(), 1);
        assert!(!home.join(".zshenv").exists());
    }

    #[test]
    fn strip_removes_only_the_block_and_the_fish_file() {
        let home = scratch_dir("path-strip");
        fs::write(home.join(".zshenv"), "export A=1\n").unwrap();
        ensure(&home, &setup(PathShell::Zsh, "/opt/pb/bin")).unwrap();
        ensure(&home, &setup(PathShell::Fish, "/opt/pb/bin")).unwrap();
        let changed = strip(&home, 7, false);
        assert_eq!(changed.len(), 2);
        assert_eq!(
            fs::read_to_string(home.join(".zshenv")).unwrap(),
            "export A=1\n"
        );
        assert!(!fish_file(&home).exists());
    }

    #[test]
    fn the_bin_dir_is_the_path_entry_that_resolves_to_the_binary() {
        let home = scratch_dir("path-resolve");
        let real = home.join("cellar");
        let link = home.join("bin");
        fs::create_dir_all(&real).unwrap();
        fs::create_dir_all(&link).unwrap();
        fs::write(real.join("playbook"), "x").unwrap();
        #[cfg(unix)]
        std::os::unix::fs::symlink(real.join("playbook"), link.join("playbook")).unwrap();
        let path = std::env::join_paths([&link]).unwrap();
        assert_eq!(resolve_bin_dir(&path, &real.join("playbook")), link);
        assert_eq!(
            resolve_bin_dir(OsStr::new(""), &real.join("playbook")),
            real
        );
    }
}
