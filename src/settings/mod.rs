// SPDX-FileCopyrightText: 2026 Igor Santos
// SPDX-License-Identifier: MIT

//! Settings seed generation and validation, backing `playbook settings`.
//! Both are ports: `gen` of the retired shell original, `check` of the
//! now-deleted the retired shell original.

pub mod check;
pub mod gen;
pub mod keys;
