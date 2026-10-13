// SPDX-FileCopyrightText: 2026 Igor Santos
// SPDX-License-Identifier: Apache-2.0

//! Helpers for the integration tests under `tests/`. Not part of the public
//! interface. They live in `playbook-common`.

pub use playbook_common::testing::*;

/// Stores the graph in `json` (`{"nodes": [...], "edges": [...]}`) in the
/// `memory.db` beside the memory folder `mem_dir`, as a fixture for the tools
/// that read the graph. Panics on a document that is not JSON.
pub fn write_graph_fixture(mem_dir: &std::path::Path, json: &str) {
    if let Err(e) = crate::hooks::rebuild_memory_graph::import_json(mem_dir, json) {
        panic!("could not store the graph fixture: {e}");
    }
}
