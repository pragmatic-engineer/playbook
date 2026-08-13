// SPDX-FileCopyrightText: 2026 Igor Santos
// SPDX-License-Identifier: MIT

//! Stub for the `precommit-check` hook (ports hooks/precommit-check.sh). Real
//! behaviour lands in a later Work Unit; only this file's body changes when
//! it does.

use crate::common::payload::Payload;

pub fn run(_payload: &Payload) {}
