// SPDX-FileCopyrightText: 2026 Igor Santos
// SPDX-License-Identifier: Apache-2.0

//! The `playbook usage` commands' shared steps: resolve where the local data
//! lives, ingest incrementally, render.

use super::account::{account_label, UNKNOWN_ACCOUNT};
use super::backfill;
use super::claude_code::ClaudeCodeSource;
use super::codex::CodexSource;
use super::db::{load_tool_events, load_usage_events, open_db};
use super::ingest::{ingest, IngestStats};
use super::summary;
use crate::common::paths::{playbook_root_from, usage_db_dir};
use rusqlite::Connection;
use std::path::{Path, PathBuf};

/// Where Claude Code's and Codex's data and playbook's usage store live for one home.
#[derive(Clone)]
pub struct Paths {
    pub claude_projects: PathBuf,
    pub claude_json: PathBuf,
    pub codex_sessions: PathBuf,
    pub db: PathBuf,
    pub lock: PathBuf,
}

impl Paths {
    pub fn from_home(home: &Path) -> Self {
        Self {
            claude_projects: home.join(".claude").join("projects"),
            claude_json: home.join(".claude.json"),
            codex_sessions: home.join(".codex").join("sessions"),
            db: playbook_root_from(home).join("usage").join("usage.db"),
            lock: playbook_root_from(home)
                .join("usage")
                .join("dashboard.lock"),
        }
    }

    pub fn real() -> Self {
        let mut paths = Self::from_home(&crate::common::home_dir());
        paths.db = usage_db_dir().join("usage.db");
        paths.lock = usage_db_dir().join("dashboard.lock");
        paths
    }
}

/// Ingests new transcript events into the store and returns what was added.
pub fn ingest_new(paths: &Paths) -> Result<(Connection, IngestStats), String> {
    let conn = open_db(&paths.db)?;
    let source = ClaudeCodeSource::new(paths.claude_projects.clone());
    backfill::run_once(&source, &conn)?;
    let account = account_label(&paths.claude_json);
    let mut stats = ingest(&source, &account, &conn)?;
    if paths.codex_sessions.is_dir() {
        let codex = CodexSource::new(paths.codex_sessions.clone());
        let more = ingest(&codex, UNKNOWN_ACCOUNT, &conn)?;
        stats.usage_inserted += more.usage_inserted;
        stats.usage_updated += more.usage_updated;
        stats.tools_inserted += more.tools_inserted;
        stats.duplicates_skipped += more.duplicates_skipped;
    }
    Ok((conn, stats))
}

/// `playbook usage ingest`.
pub fn run_ingest(paths: &Paths) -> Result<String, String> {
    let (_, stats) = ingest_new(paths)?;
    Ok(format!(
        "usage ingest: added {} usage events and {} tool events ({} already stored)",
        stats.usage_inserted, stats.tools_inserted, stats.duplicates_skipped
    ))
}

/// Bare `playbook usage`: ingest, then render.
pub fn run_summary(paths: &Paths) -> Result<String, String> {
    let (conn, _) = ingest_new(paths)?;
    let usage = load_usage_events(&conn)?;
    let tools = load_tool_events(&conn)?;
    Ok(summary::render(&usage, &tools))
}
