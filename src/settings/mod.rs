// SPDX-FileCopyrightText: 2026 Igor Santos
// SPDX-License-Identifier: MIT

//! Settings seed generation and validation, backing `playbook settings`.
//! Both are ports: `gen` of `shell/gen-shared-settings.py`, `check` of the
//! now-deleted `shell/check-shared-settings.py`.

pub mod check;
pub mod gen;
