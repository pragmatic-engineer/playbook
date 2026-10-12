// SPDX-FileCopyrightText: 2026 Igor Santos
// SPDX-License-Identifier: Apache-2.0

//! `playbook code`: a local, disposable index of the repository's source that
//! answers with exact paths, line ranges and short excerpts.
//!
//! The public functions here are the library surface. The command line layer
//! in `cli` calls them and only renders. This module reaches the host program
//! only through `host`.

pub mod cli;
pub mod db;
pub mod host;
pub mod index;
pub mod location;
pub mod output;
pub mod walker;

pub use cli::{run, CodeCommand, Env};

use std::path::{Path, PathBuf};

/// The result of an indexing run.
pub struct IndexOutcome {
    pub index_path: PathBuf,
    pub report: index::Report,
}

/// Builds or refreshes the index for the repository containing `cwd`.
pub fn index(cwd: &Path, rebuild: bool) -> Result<IndexOutcome, String> {
    let root = location::repo_root(cwd)?;
    let index_path = location::index_path()?;
    let report = index::run(&root, &index_path, rebuild, &index::Options::default())?;
    Ok(IndexOutcome { index_path, report })
}
