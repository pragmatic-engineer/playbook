// SPDX-FileCopyrightText: 2026 Igor Santos
// SPDX-License-Identifier: MIT

//! Stub for the `no-dash-guard` hook (ports hooks/no-dash-guard.sh). Real
//! behaviour lands in a later Work Unit; only this file's body changes when
//! it does.

use crate::common::payload::Payload;

pub fn run(_payload: &Payload) {}
