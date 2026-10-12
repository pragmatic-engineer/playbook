// SPDX-FileCopyrightText: 2026 Igor Santos
// SPDX-License-Identifier: Apache-2.0

//! SessionStart and SessionEnd hook that warms the usage store. It starts a
//! detached, low priority, time bounded ingest and returns at once, with no
//! output, so it never slows or blocks a session. The plugin-disabled pause is
//! handled before any hook runs. See `usage::warm`.

use crate::common::payload::Payload;
use crate::usage::run::Paths;

pub fn run(payload: &Payload) {
    // A real hook call always names its session; a malformed one does nothing.
    if payload.field(".session_id").is_empty() {
        return;
    }
    let Ok(exe) = std::env::current_exe() else {
        return;
    };
    let home = crate::common::home_dir();
    let mut paths = Paths::from_home(&home);
    paths.db = crate::common::paths::usage_db_dir().join("usage.db");
    crate::usage::warm::maybe_spawn(&home, &paths, &exe);
}
