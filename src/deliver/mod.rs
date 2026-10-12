// SPDX-FileCopyrightText: 2026 Igor Santos
// SPDX-License-Identifier: Apache-2.0

//! `deliver`: commit, rebase, push and open or update a pull request from any
//! worktree. This part holds the text rules, the process adapter and the
//! `gh` pull request helpers. It imports only `common` helpers.

pub mod check;
pub mod pr;
pub mod sys;
