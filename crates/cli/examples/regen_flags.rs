// SPDX-FileCopyrightText: 2026 Igor Santos
// SPDX-License-Identifier: Apache-2.0

//! Regenerates `crates/cli/tests/golden/flags.txt`. It is the only writer of
//! that file. It refuses to run in CI, where a regenerated golden would hide a
//! flag change from the check.
//!
//! Run: `cargo run -p playbook --example regen_flags`

use clap::CommandFactory;
use playbook::flags_snapshot::write_flags_snapshot;
use playbook::Cli;
use std::path::Path;
use std::process::ExitCode;

const GOLDEN: &str = "tests/golden/flags.txt";

fn main() -> ExitCode {
    if std::env::var_os("CI").is_some() {
        eprintln!("regen_flags: refusing to run when CI is set");
        return ExitCode::FAILURE;
    }
    let path = Path::new(env!("CARGO_MANIFEST_DIR")).join(GOLDEN);
    match write_flags_snapshot(&Cli::command(), &path) {
        Ok(()) => ExitCode::SUCCESS,
        Err(err) => {
            eprintln!("regen_flags: cannot write {}: {err}", path.display());
            ExitCode::FAILURE
        }
    }
}
