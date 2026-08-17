// SPDX-FileCopyrightText: 2026 Igor Santos
// SPDX-License-Identifier: MIT

//! Settings seed generation and drift-check modules, backing the `playbook
//! settings` subcommand. `gen` ports `shell/gen-shared-settings.py`; `check`
//! is filled in by WU-21, ported from `shell/check-shared-settings.py`.

pub mod check;
pub mod gen;
