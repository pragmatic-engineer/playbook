// SPDX-FileCopyrightText: 2026 Igor Santos
// SPDX-License-Identifier: MIT

//! Stub for the `rebuild-memory-graph` hook (ports
//! hooks/rebuild-memory-graph.py). Real behaviour lands in a later Work
//! Unit; only this file's body changes when it does.

use crate::common::payload::Payload;

pub fn run(_payload: &Payload) {}
