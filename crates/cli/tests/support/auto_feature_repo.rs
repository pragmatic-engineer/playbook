// SPDX-FileCopyrightText: 2026 Igor Santos
// SPDX-License-Identifier: Apache-2.0

//! A scratch git repo on a feature branch.

use super::auto_env::Scratch;

/// A git repo under the scratch cwd on branch `feat/x` with one commit, made
/// once, for tests whose tool calls need a real checkout to run in.
pub fn feature_repo(s: &Scratch) -> std::path::PathBuf {
    let dir = s.cwd.join("feature-repo");
    if dir.join(".git").exists() {
        return dir;
    }
    std::fs::create_dir_all(&dir).expect("repo dir");
    for args in [
        vec!["init", "-q", "-b", "feat/x"],
        vec!["commit", "-q", "--allow-empty", "-m", "seed"],
    ] {
        let status = std::process::Command::new("git")
            .args(&args)
            .current_dir(&dir)
            .env("GIT_CONFIG_GLOBAL", "/dev/null")
            .env("GIT_CONFIG_SYSTEM", "/dev/null")
            .env("GIT_AUTHOR_NAME", "t")
            .env("GIT_AUTHOR_EMAIL", "t@t")
            .env("GIT_COMMITTER_NAME", "t")
            .env("GIT_COMMITTER_EMAIL", "t@t")
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .status()
            .expect("git");
        assert!(status.success(), "git {args:?}");
    }
    dir
}
