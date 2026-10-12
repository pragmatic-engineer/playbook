// SPDX-FileCopyrightText: 2026 Igor Santos
// SPDX-License-Identifier: Apache-2.0

//! The generic helpers `src/usage` takes from the host crate: clock, private
//! file writes, git output and the test scratch directory. Like `host`, this
//! is one of the two files that name the host crate; a standalone build
//! replaces it with its own copies.

pub use crate::common::{atomic, git, time};

#[cfg(test)]
pub(crate) use crate::common::test_support::scratch_dir;
