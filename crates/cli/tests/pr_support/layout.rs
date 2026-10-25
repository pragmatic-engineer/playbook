// SPDX-FileCopyrightText: 2026 Igor Santos
// SPDX-License-Identifier: Apache-2.0

//! Where `Fixture::new` puts the bare `origin` and the state directory, for
//! the tests that read them. `pr_cli.rs` does not, so it leaves this out.

use super::fixture::Fixture;
use std::path::PathBuf;

impl Fixture {
    pub fn bare(&self) -> PathBuf {
        self.work.with_file_name("origin.git")
    }

    pub fn state(&self) -> PathBuf {
        self.work.with_file_name("state")
    }
}
