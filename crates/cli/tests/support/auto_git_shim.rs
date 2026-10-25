// SPDX-FileCopyrightText: 2026 Igor Santos
// SPDX-License-Identifier: Apache-2.0

//! A `git` shim for tests that read the origin URL through `git`.

#[cfg(unix)]
use super::auto_env::Scratch;
#[cfg(unix)]
use super::auto_warm_up::warm_up;
#[cfg(unix)]
use std::fs;
#[cfg(unix)]
use std::path::PathBuf;

/// A `git` shim that logs each call to a file and answers every call with the
/// `acme/widgets` origin URL, plus the PATH that puts it first.
#[cfg(unix)]
pub struct GitShim {
    pub path: String,
    pub log: PathBuf,
}

#[cfg(unix)]
impl GitShim {
    pub fn calls(&self) -> usize {
        fs::read_to_string(&self.log).map_or(0, |log| log.lines().count())
    }
}

#[cfg(unix)]
pub fn git_shim(scratch: &Scratch) -> GitShim {
    use std::os::unix::fs::PermissionsExt;
    let bin = scratch.home.join("shim-bin");
    fs::create_dir_all(&bin).expect("shim dir should be creatable");
    let log = scratch.home.join("git-calls.log");
    let script = format!(
        "#!/bin/sh\necho \"$@\" >> '{}'\necho git@github.com:acme/widgets.git\n",
        log.display()
    );
    let shim = bin.join("git");
    fs::write(&shim, script).expect("shim should be writable");
    fs::set_permissions(&shim, fs::Permissions::from_mode(0o755)).expect("shim should be exec");
    warm_up(&shim);
    let _ = fs::remove_file(&log);
    let path = format!(
        "{}:{}",
        bin.display(),
        std::env::var("PATH").unwrap_or_default()
    );
    GitShim { path, log }
}
