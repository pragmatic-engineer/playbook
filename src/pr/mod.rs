// SPDX-FileCopyrightText: 2026 Igor Santos
// SPDX-License-Identifier: Apache-2.0

//! `playbook pr`: the mechanical half of `/playbook:create-pull-request`.

pub mod comments;
pub mod create;
pub mod guard;
pub mod prepare;
pub mod rules;
pub mod shared;
pub mod stack;
pub mod triage;
pub mod wait;
