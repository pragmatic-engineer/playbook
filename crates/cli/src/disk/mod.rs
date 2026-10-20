// SPDX-FileCopyrightText: 2026 Igor Santos
// SPDX-License-Identifier: Apache-2.0

//! The disk guard (ADR 0047): reads free space, compares it with the
//! `disk.*` thresholds, and keeps the disk from filling while agents build.

pub mod backups;
pub mod ballast;
pub mod classify;
pub mod clean;
pub mod policy;
pub mod probe;
pub mod size;
pub mod status;
pub mod targets;

pub use policy::{Level, Policy};
