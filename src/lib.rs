// SPDX-FileCopyrightText: 2026 Igor Santos
// SPDX-License-Identifier: MIT

//! Library root for the `playbook` binary. Exposes the CLI shape that
//! `main.rs` parses and dispatches on, plus the `common` helpers and `hooks`
//! stubs every hook module builds on.

pub mod agents;
pub mod cc;
pub mod common;
pub mod hooks;
pub mod init;
pub mod manifest;
pub mod settings;

use clap::{Parser, Subcommand, ValueEnum};
use std::path::PathBuf;

/// The `playbook` command-line entry point.
#[derive(Parser, Debug)]
#[command(name = "playbook", version)]
pub struct Cli {
    #[command(subcommand)]
    pub command: Command,
}

/// Top-level subcommand groups.
#[derive(Subcommand, Debug)]
pub enum Command {
    /// Run a named hook.
    Hook {
        /// Which hook to run, matching the name Claude Code passes from
        /// hooks.json.
        name: HookName,
    },
    /// Launcher subcommands (session, worktree, retention, and so on).
    Cc {
        #[command(subcommand)]
        sub: Option<CcCommand>,
    },
    /// Print the Claude Code status line.
    Statusline,
    /// Install or repair the local Claude Code configuration.
    Init,
    /// Shared-settings seed subcommands (`gen` today; `check` from WU-21).
    Settings {
        #[command(subcommand)]
        sub: SettingsCommand,
    },
    /// Tracked-file manifest subcommands, backing `src/manifest/`.
    Manifest {
        #[command(subcommand)]
        sub: ManifestCommand,
    },
    /// Agent-definition validation subcommands, backing `src/agents/`.
    Agents {
        #[command(subcommand)]
        sub: AgentsCommand,
    },
}

/// `playbook settings` subcommands, backing `src/settings/`.
#[derive(Subcommand, Debug)]
pub enum SettingsCommand {
    /// Derive the tracked settings.shared.json seed from a live settings.json,
    /// ported from `shell/gen-shared-settings.py`.
    Gen {
        /// Path to the live settings.json to derive the seed from.
        src: PathBuf,
        /// Path to the canned permissions object.
        perms: PathBuf,
    },
    /// Validate the tracked settings.shared.json seed, ported from
    /// `shell/check-shared-settings.py`.
    Check {
        /// Path to the settings.shared.json template to validate.
        template: PathBuf,
        /// Path to the tracked permissions.shared.json.
        perms: PathBuf,
        /// Repo root that every hook command must resolve inside.
        repo_root: PathBuf,
    },
}

/// `playbook manifest` subcommands, backing `src/manifest/`.
#[derive(Subcommand, Debug)]
pub enum ManifestCommand {
    /// Validate every tracked file lives at an allowlisted top-level path,
    /// ported from `shell/check-manifest.sh`.
    Check {
        /// Repo root whose tracked files (`git ls-files`) are checked.
        ///
        /// Optional, like the shell's `${1:-}`: omitting it falls back to
        /// `git rev-parse --show-toplevel`, so `playbook manifest check` run
        /// from anywhere inside the repo behaves as the script did.
        repo_root: Option<PathBuf>,
    },
}

/// `playbook agents` subcommands, backing `src/agents/`.
#[derive(Subcommand, Debug)]
pub enum AgentsCommand {
    /// Validate every `agents/*.md` definition against the house agent
    /// contract, ported from `shell/check-agents.sh`.
    Check {
        /// Directory holding the agent definitions to validate.
        ///
        /// Optional, like the shell's `[AGENTS_DIR]`: omitting it falls back
        /// to `<repo root>/agents`, where repo root is resolved the same way
        /// `check-agents.sh` did, via `git rev-parse --show-toplevel` from
        /// the current directory.
        agents_dir: Option<PathBuf>,
    },
}

/// Every hook Claude Code can invoke, one per entry in hooks.json. Kebab-case
/// on the CLI (clap's default `ValueEnum` casing) so a typo in hooks.json
/// surfaces immediately via clap's possible-value error rather than the hook
/// silently doing nothing.
#[derive(ValueEnum, Debug, Clone, Copy)]
pub enum HookName {
    SessionInit,
    PrereadEditCheck,
    PrereadSizeCheck,
    SearchCounter,
    MemoryAnchors,
    PostEditTrack,
    RebuildMemoryGraph,
    AutoModelDetect,
    PrecompactWarn,
    SessionCleanExit,
    MemoryCapture,
    RmWorkspaceGuard,
    BgAwaitGuard,
    NoDashGuard,
    PrecommitCheck,
}

/// `cc` launcher subcommands, matching `shell/shared/dispatch.sh:59-100`. No
/// subcommand at all (`Cc { sub: None }`) replicates the default path there:
/// resume the most recent session for this project by its custom title.
#[derive(Subcommand, Debug)]
pub enum CcCommand {
    /// Clean and resume the most recent matching session.
    Clean,
    /// Start a fresh session; no resume, settings.json re-applied.
    Fresh,
    /// Resume raw, optionally by session id; no fork, overrides preserved.
    Raw { sid: Option<String> },
    /// List sessions for the current project.
    #[command(alias = "ls")]
    List,
    /// Prune stale runtime state.
    Prune,
    /// Clear caches that would otherwise freeze stale settings into a session.
    #[command(name = "bust-cache")]
    BustCache,
    /// Create a worktree and resume into it.
    #[command(alias = "new")]
    Worktree {
        branch: String,
        /// Folder (relative to the repo root) holding the `.env` to copy in.
        env_base: Option<String>,
    },
}

#[cfg(test)]
mod tests {
    use super::*;
    use clap::CommandFactory;

    #[test]
    fn version_flag_prints_cargo_toml_version() {
        // Arrange
        let expected = env!("CARGO_PKG_VERSION");

        // Act
        let result = Cli::command().try_get_matches_from(["playbook", "--version"]);

        // Assert
        let err =
            result.expect_err("--version should short-circuit parsing with a version message");
        assert!(err.to_string().contains(expected));
    }

    #[test]
    fn hook_help_lists_all_fifteen_hook_names() {
        // Arrange
        let names = [
            "session-init",
            "preread-edit-check",
            "preread-size-check",
            "search-counter",
            "memory-anchors",
            "post-edit-track",
            "rebuild-memory-graph",
            "auto-model-detect",
            "precompact-warn",
            "session-clean-exit",
            "memory-capture",
            "rm-workspace-guard",
            "bg-await-guard",
            "no-dash-guard",
            "precommit-check",
        ];

        // Act
        let result = Cli::command().try_get_matches_from(["playbook", "hook", "--help"]);

        // Assert
        let err = result.expect_err("--help should short-circuit parsing with a help message");
        let help = err.to_string();
        for name in names {
            assert!(help.contains(name), "hook --help is missing '{name}'");
        }
    }

    #[test]
    fn cc_subcommands_parse_including_aliases() {
        // Arrange, Act
        let list = Cli::command().try_get_matches_from(["playbook", "cc", "list"]);
        let ls = Cli::command().try_get_matches_from(["playbook", "cc", "ls"]);
        let worktree =
            Cli::command().try_get_matches_from(["playbook", "cc", "worktree", "my-branch"]);
        let worktree_with_env_base =
            Cli::command().try_get_matches_from(["playbook", "cc", "worktree", "my-branch", "env"]);
        let new = Cli::command().try_get_matches_from(["playbook", "cc", "new", "my-branch"]);
        let raw_no_sid = Cli::command().try_get_matches_from(["playbook", "cc", "raw"]);
        let raw_with_sid =
            Cli::command().try_get_matches_from(["playbook", "cc", "raw", "sid-123"]);
        let default = Cli::command().try_get_matches_from(["playbook", "cc"]);

        // Assert
        assert!(list.is_ok(), "cc list should parse");
        assert!(ls.is_ok(), "cc ls alias should parse");
        assert!(worktree.is_ok(), "cc worktree BRANCH should parse");
        assert!(
            worktree_with_env_base.is_ok(),
            "cc worktree BRANCH ENV_BASE should parse"
        );
        assert!(new.is_ok(), "cc new alias should parse");
        assert!(raw_no_sid.is_ok(), "cc raw with no sid should parse");
        assert!(raw_with_sid.is_ok(), "cc raw SID should parse");
        assert!(default.is_ok(), "cc with no subcommand should parse");
    }
}
