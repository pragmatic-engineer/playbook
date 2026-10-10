// SPDX-FileCopyrightText: 2026 Igor Santos
// SPDX-License-Identifier: Apache-2.0

//! Shared helpers ported from the retired shell originals.
//! Every hook in `src/hooks` uses this module instead of re-deriving these
//! primitives, so there is exactly one implementation to keep correct.

pub mod atomic;
pub mod attribution;
pub mod cli_opts;
pub mod config_hash;
pub mod counter;
pub mod emit;
pub mod git;
pub mod gitfacts;
pub mod headless;
pub mod mode;
pub mod par;
pub mod paths;
pub mod payload;
pub mod proc;
pub mod prose;
pub mod re;
pub mod repo;
pub mod session;
pub mod shell;
pub mod time;

pub use atomic::atomic_append;
pub use config_hash::config_hash;
pub use counter::incr_counter;
pub use emit::{
    emit_block, emit_pre_context, emit_pre_deny, emit_pre_updated_input, emit_prompt_context,
    emit_system_message,
};
pub use paths::{
    cc_state_dir, memory_dir, playbook_root, repo_scoped_dir, runtime_root, usage_db_dir, RepoScope,
};
pub use payload::Payload;
pub use proc::{run_with_input, run_with_timeout};
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
    static CWD_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

    /// Serialises tests that change the process-global cwd.
    pub(crate) fn lock_cwd() -> std::sync::MutexGuard<'static, ()> {
        CWD_LOCK.lock().unwrap_or_else(|e| e.into_inner())
    }

    pub(crate) fn scratch_dir(tag: &str) -> PathBuf {
        let n = COUNTER.fetch_add(1, Ordering::Relaxed);
        std::env::temp_dir().join(format!(
            "playbook-test-{}-{tag}-{n}",
            crate::testing::run_id()
        ))
    }
}
