// SPDX-FileCopyrightText: 2026 Igor Santos
// SPDX-License-Identifier: Apache-2.0

//! Seeds the global and repo config under a scratch home.

use super::auto_env::Scratch;
use std::fs;
use std::path::Path;

impl Scratch {
    /// Plant `{"mode": <mode>}` as the global config under the scratch HOME.
    // Not every hook test seeds a mode.
    #[allow(dead_code)]
    pub fn seed_mode_config(&self, mode: &str) {
        seed_global_config(&self.home, &serde_json::json!({ "mode": mode }));
    }
}

impl Scratch {
    /// Plant `{"mode": <mode>}` as the repo tier config for `owner/repo`.
    // Not every hook test seeds a repo tier.
    #[allow(dead_code)]
    pub fn seed_repo_mode_config(&self, owner: &str, repo: &str, mode: &str) {
        let dir = self
            .home
            .join(".config/playbook/repos")
            .join(owner)
            .join(repo)
            .join(".config");
        fs::create_dir_all(&dir).expect("repo config dir should be creatable");
        fs::write(dir.join("config.json"), format!(r#"{{"mode":"{mode}"}}"#))
            .expect("repo config should be writable");
    }
}

/// Replace the global config under `home` with the leaves of `doc` (a nested
/// object such as `{"auto": {"warnPct": 70}}`), through the config store.
pub fn seed_global_config(home: &Path, doc: &serde_json::Value) {
    clear_config(home);
    fn walk(prefix: &str, node: &serde_json::Value, home: &Path) {
        match node.as_object() {
            Some(obj) => {
                for (k, v) in obj {
                    let key = if prefix.is_empty() {
                        k.clone()
                    } else {
                        format!("{prefix}.{k}")
                    };
                    walk(&key, v, home);
                }
            }
            None => playbook::config::write::set(
                playbook::config::write::Tier::Global,
                prefix,
                node.clone(),
                home,
                None,
            )
            .expect("seeded config value should be valid"),
        }
    }
    walk("", doc, home);
}

/// Remove every stored config value under `home`.
pub fn clear_config(home: &Path) {
    let dir = home.join(".config").join("playbook");
    for name in [
        "playbook.db",
        "playbook.db-wal",
        "playbook.db-shm",
        "config.json",
        "data/config.db",
        "data/config.db-wal",
        "data/config.db-shm",
    ] {
        let _ = fs::remove_file(dir.join(name));
    }
}

/// Replace the global config with `text` written as a legacy `config.json`,
/// which the store imports on its next open. For values `config set` refuses
/// (an invalid budget) or a file that is not JSON at all.
// Not every hook test seeds raw text.
#[allow(dead_code)]
pub fn seed_raw_global_config(home: &Path, text: &str) {
    clear_config(home);
    let dir = home.join(".config").join("playbook");
    fs::create_dir_all(&dir).expect("config dir should be creatable");
    fs::write(dir.join("config.json"), text).expect("config file should be writable");
}
