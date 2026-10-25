// SPDX-FileCopyrightText: 2026 Igor Santos
// SPDX-License-Identifier: Apache-2.0

//! A copy of the built `playbook` for `install.sh` runs that need a real one.

use super::shell_fixture::Work;
use std::fs;
use std::path::PathBuf;

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
