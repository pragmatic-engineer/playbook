// SPDX-FileCopyrightText: 2026 Igor Santos
// SPDX-License-Identifier: MIT

//! Shared helpers ported from hooks/lib/common.py and hooks/lib/common.sh.
//! Every hook in `src/hooks` uses this module instead of re-deriving these
//! primitives, so there is exactly one implementation to keep correct.

pub mod atomic;
pub mod config_hash;
pub mod counter;
pub mod emit;
pub mod payload;
pub mod proc;
pub mod repo;
pub mod session;

pub use atomic::atomic_append;
pub use config_hash::config_hash;
pub use counter::incr_counter;
pub use emit::{
    emit_block, emit_pre_context, emit_pre_deny, emit_prompt_context, emit_system_message,
};
pub use payload::Payload;
pub use proc::run_with_timeout;
pub use repo::repo_slug;
pub use session::{abspath, home_dir, session_dir, session_id};

/// Test-only filesystem scratch space, shared by every `common` submodule's
/// tests that need a real directory on disk. Not created by this call;
/// callers create what they need inside it. Unique per call so parallel test
/// threads never collide on the same path.
#[cfg(test)]
pub(crate) mod test_support {
    use std::path::PathBuf;
    use std::sync::atomic::{AtomicU64, Ordering};

    static COUNTER: AtomicU64 = AtomicU64::new(0);

    pub(crate) fn scratch_dir(tag: &str) -> PathBuf {
        let n = COUNTER.fetch_add(1, Ordering::Relaxed);
        std::env::temp_dir().join(format!("playbook-test-{}-{tag}-{n}", std::process::id()))
    }
}
