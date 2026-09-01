// SPDX-FileCopyrightText: 2026 Igor Santos
// SPDX-License-Identifier: MIT

//! Migrates `~/.claude/memory/graph.json` (the pre-rename filename) to
//! `memory.graph.json`, backing `playbook init`'s sixth step.

use crate::init::run::{StepReport, StepStatus};
use std::fs;
use std::path::Path;

const OLD_FILE_NAME: &str = "graph.json";
const NEW_FILE_NAME: &str = "memory.graph.json";
const OLD_LOCK_NAME: &str = "graph.json.lock";
const NEW_LOCK_NAME: &str = "memory.graph.json.lock";

/// Renames the pre-rename `graph.json` (and its mkdir-based `.lock`
/// sibling, if present) to `memory.graph.json`. An old file left alongside an already-migrated store is never touched or deleted.
pub fn migrate_memory_store(claude_home: &Path) -> StepReport {
    let mem_dir = claude_home.join("memory");
    let old_path = mem_dir.join(OLD_FILE_NAME);
    let new_path = mem_dir.join(NEW_FILE_NAME);

    if new_path.exists() {
        return StepReport {
            name: "memory-migrate",
            status: StepStatus::AlreadyCorrect,
            detail: format!("already at {}", new_path.display()),
        };
    }

    if !old_path.exists() {
        return StepReport {
            name: "memory-migrate",
            status: StepStatus::Skipped,
            detail: "no legacy graph.json to migrate".to_string(),
        };
    }

    if let Err(err) = fs::rename(&old_path, &new_path) {
        return StepReport {
            name: "memory-migrate",
            status: StepStatus::Failed,
            detail: format!(
                "could not rename {} to {}: {err}",
                old_path.display(),
                new_path.display()
            ),
        };
    }

    let old_lock = mem_dir.join(OLD_LOCK_NAME);
    if old_lock.exists() {
        let _ = fs::rename(&old_lock, mem_dir.join(NEW_LOCK_NAME));
    }

    StepReport {
        name: "memory-migrate",
        status: StepStatus::Wired,
        detail: format!("renamed {} to {}", old_path.display(), new_path.display()),
    }
}
