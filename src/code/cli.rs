// SPDX-FileCopyrightText: 2026 Igor Santos
// SPDX-License-Identifier: Apache-2.0

//! The thin command line layer: argument shapes, and rendering of the results
//! the library functions in `code` return.

use super::output::{Envelope, IndexState};
use clap::Subcommand;
use serde_json::json;
use std::path::PathBuf;

/// `playbook code` subcommands.
#[derive(Subcommand, Debug)]
pub enum CodeCommand {
    /// Build or refresh the code index for this repository
    ///
    /// Hashes every file git lists, and reads only the ones that changed.
    /// Only one run at a time is allowed.
    ///
    /// Example: `playbook code index --json`
    Index {
        /// Print one JSON document
        #[arg(long)]
        json: bool,
        /// Delete the cache first and index everything again
        #[arg(long)]
        rebuild: bool,
    },
}

/// What a command needs from its surroundings.
pub struct Env {
    pub cwd: PathBuf,
}

impl Env {
    pub fn real() -> Self {
        Self {
            cwd: std::env::current_dir().unwrap_or_default(),
        }
    }
}

/// Runs one subcommand and returns what to print.
pub fn run(sub: CodeCommand, env: &Env) -> Result<String, String> {
    match sub {
        CodeCommand::Index { json, rebuild } => render_index(env, json, rebuild),
    }
}

fn render_index(env: &Env, json: bool, rebuild: bool) -> Result<String, String> {
    let outcome = super::index(&env.cwd, rebuild)?;
    let report = &outcome.report;
    let mut warnings = Vec::new();
    if report.failed > 0 {
        warnings.push(format!(
            "{} files could not be read and keep their previous index rows",
            report.failed
        ));
    }
    if json {
        let mut envelope = Envelope::new(
            "index",
            IndexState::Fresh,
            outcome.index_path.display().to_string(),
            json!({
                "indexed": report.indexed,
                "changed": report.changed,
                "unchanged": report.unchanged,
                "parsed": report.parsed,
                "removed": report.removed,
                "skipped": report.skipped,
                "failed": report.failed,
            }),
        );
        envelope.warnings = warnings;
        return Ok(envelope.to_json());
    }
    for warning in &warnings {
        eprintln!("code: {warning}");
    }
    Ok(format!(
        "indexed {} files: {} changed, {} unchanged, {} removed, {} skipped, {} failed\n",
        report.indexed,
        report.changed,
        report.unchanged,
        report.removed,
        report.skipped,
        report.failed
    ))
}
