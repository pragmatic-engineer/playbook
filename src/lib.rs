// SPDX-FileCopyrightText: 2026 Igor Santos
// SPDX-License-Identifier: MIT

//! Library root for the `playbook` binary. Exposes the CLI shape that
//! `main.rs` parses and dispatches on, plus the `common` helpers and `hooks`
//! stubs every hook module builds on.

pub mod agents;
pub mod cc;
pub mod ci;
pub mod common;
pub mod config;
pub mod doctor;
pub mod gate;
pub mod handoff;
pub mod hooks;
pub mod init;
pub mod json;
pub mod manifest;
pub mod pr;
pub mod settings;
pub mod trust;
pub mod usage;
pub mod worktree;

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
    Init {
        /// Also install `prompts/SYSTEM_PROMPT.md` into `~/.config/playbook/prompts/`.
        /// Opt-in, matching `shell/setup-local.sh`'s flag of the same name:
        /// without it an already-installed copy is still refreshed, but one
        /// is never installed for a user who did not ask.
        #[arg(long)]
        system_prompt: bool,
        /// Also install the shell launcher shim (`cc`/`ccd`) and wire the rc
        /// file. Opt-in, matching `shell/setup-local.sh`'s `--aliases` flag:
        /// without it the `shim` step is skipped entirely, never partially
        /// touched.
        #[arg(long)]
        aliases: bool,
    },
    /// Shared-settings seed subcommands (`gen` and `check`).
    Settings {
        #[command(subcommand)]
        sub: SettingsCommand,
    },
    /// Tracked-file manifest subcommands, backing `src/manifest/`.
    Manifest {
        #[command(subcommand)]
        sub: ManifestCommand,
    },
    /// Memory-graph subcommands.
    Memory {
        #[command(subcommand)]
        sub: MemoryCommand,
    },
    /// Agent-definition validation subcommands, backing `src/agents/`.
    Agents {
        #[command(subcommand)]
        sub: AgentsCommand,
    },
    /// Token and cost usage across sessions, backing `src/usage/`. Bare
    /// `playbook usage` prints a summary.
    Usage {
        #[command(subcommand)]
        sub: Option<UsageCommand>,
    },
    /// Gate-check database subcommands, backing `src/gate/`.
    Gate {
        #[command(subcommand)]
        sub: GateCommand,
    },
    /// The mechanical half of `/playbook:create-pull-request`, backing
    /// `src/pr/`.
    Pr {
        #[command(subcommand)]
        sub: PrCommand,
    },
    /// Run the model-free repo checks (manifest, agents, settings) in one step.
    /// Prints one line per check and exits 1 if any fails; safe in CI.
    Ci {
        /// Print one JSON object instead of text.
        #[arg(long)]
        json: bool,
        /// Run in this directory instead of the cwd.
        #[arg(long)]
        dir: Option<String>,
    },
    /// Playbook's own tiered (repo < org < global < default) config
    /// subcommands, backing `src/config/`.
    Config {
        #[command(subcommand)]
        sub: ConfigCommand,
    },
    /// Print the resolved absolute path to one of this repo's
    /// worktree-scoped storage directories.
    Path {
        /// Which worktree-scoped directory to resolve.
        kind: PathKind,
    },
    /// Single-field JSON reads backing `/playbook:doctor`, `src/doctor/`.
    Doctor {
        #[command(subcommand)]
        sub: DoctorCommand,
    },
    /// Reap landed worktrees across every creation convention, backing
    /// `src/worktree/`.
    Worktree {
        #[command(subcommand)]
        sub: WorktreeCommand,
    },
    /// Hand-written `jq` replacements over a piped session, `gh`, JSONL, or
    /// file JSON payload, backing `src/json/`.
    Json {
        #[command(subcommand)]
        sub: JsonCommand,
    },
    /// Pre-trust an absolute directory in `~/.claude.json` so Claude Code's
    /// first-launch trust dialog never blocks `cc`/`ccd`. Always exits 0.
    Trust {
        /// Absolute path of the directory to trust.
        path: String,
    },
    /// Save and show session handoffs, backing `src/handoff/`.
    Handoff {
        #[command(subcommand)]
        sub: HandoffCommand,
    },
}

/// `playbook handoff` subcommands.
#[derive(Subcommand, Debug)]
pub enum HandoffCommand {
    /// Save the handoff markdown on stdin so the next session in this
    /// directory loads it.
    Save {
        /// Directory the handoff belongs to; defaults to the current one.
        #[arg(long)]
        dir: Option<String>,
    },
    /// Print this directory's handoff without consuming it.
    Show {
        /// Print every waiting handoff, not just the freshest.
        #[arg(long)]
        all: bool,
        /// Directory to show; defaults to the current one.
        #[arg(long)]
        dir: Option<String>,
    },
    /// Show the last SessionStart events and this directory's handoff counts.
    Status,
}

/// Which worktree-scoped storage directory `playbook path` resolves under
/// `repo_scoped_dir(RepoScope::Worktree)`.
#[derive(ValueEnum, Debug, Clone, Copy)]
pub enum PathKind {
    Plans,
    Implement,
    Worktrees,
}

impl PathKind {
    /// The exact subdirectory name under the worktree-scoped root.
    pub fn dir_name(&self) -> &'static str {
        match self {
            PathKind::Plans => "plans",
            PathKind::Implement => "implement",
            PathKind::Worktrees => "worktrees",
        }
    }
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

/// `playbook memory` subcommands.
#[derive(Subcommand, Debug)]
pub enum MemoryCommand {
    /// Rebuild `~/.config/playbook/memory/memory.graph.json` from every fact on disk.
    ///
    /// The PostToolUse hook rebuilds automatically when a fact is saved, so
    /// this is only needed after hand-editing fact files. Forcing it through
    /// the hook is not possible: the hook skips unless its payload names a
    /// file under the memory dir.
    Rebuild,
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

/// `playbook usage` subcommands, backing `src/usage/`.
#[derive(Subcommand, Debug)]
pub enum UsageCommand {
    /// Read new session history into the usage store, without printing a summary.
    Ingest,
    /// Open the local usage dashboard, starting its server if needed.
    Dashboard {
        /// Internal: run as the detached server process.
        #[arg(long, hide = true)]
        serve: bool,
        #[command(subcommand)]
        sub: Option<DashboardCommand>,
    },
}

/// `playbook usage dashboard` subcommands.
#[derive(Subcommand, Debug)]
pub enum DashboardCommand {
    /// Stop the running dashboard server.
    Stop,
}

/// `playbook pr` subcommands, backing `src/pr/`.
#[derive(Subcommand, Debug)]
pub enum PrCommand {
    /// Resolve the branch and base, run the pre-flight checks, write the diff
    /// to a scratch file, and print labeled lines for the PR drafter.
    Prepare {
        /// Base branch for the PR; defaults to the repo's default branch.
        #[arg(long)]
        base: Option<String>,
        /// Ticket id to cite; defaults to one found in the branch name.
        #[arg(long)]
        ticket: Option<String>,
        /// Work in this directory (inside the repo or worktree) instead of the cwd.
        #[arg(long)]
        dir: Option<String>,
    },
    /// Push the branch and open a draft PR from a drafted title and body.
    Create {
        /// PR title, at most 72 characters.
        #[arg(long)]
        title: String,
        /// Path to the file holding the PR body.
        #[arg(long)]
        body_file: String,
        /// Base branch for the PR; defaults to the repo's default branch.
        #[arg(long)]
        base: Option<String>,
        /// Work in this directory (inside the repo or worktree) instead of the cwd.
        #[arg(long)]
        dir: Option<String>,
    },
}

/// `playbook gate` subcommands, backing `src/gate/`.
#[derive(Subcommand, Debug)]
pub enum GateCommand {
    /// Parse a phase agent's raw output for a `VERDICT:` line and upsert it
    /// into the gate-check database under `~/.config/playbook`.
    Record {
        /// Plan slug the recorded phase belongs to.
        plan_slug: String,
        /// The command that produced this verdict, stored alongside it.
        command: String,
        /// Which phase this verdict is for.
        phase: String,
        /// Path to the phase agent's raw output, or "-" to read stdin.
        input: String,
        /// Path to the source content this verdict is evidence for; hashed
        /// and stored so a later `check` can detect a stale verdict.
        #[arg(long)]
        source: String,
    },
    /// Query one or more previously recorded phase verdicts for a plan;
    /// exit 0 only if every named phase is PASS or WARN.
    Check {
        /// Plan slug to query recorded phases for.
        plan_slug: String,
        /// The command this check invocation is running under, accepted for
        /// CLI-shape parity with `Record` though `check::run` does not use
        /// it to look up rows.
        command: String,
        /// One or more phase names to check. `Vec<String>` with no minimum
        /// `num_args`, so zero phase names still parses at the clap level;
        /// `check::run` rejects an empty list itself with a pinned message
        /// and exit code 1, since clap's own missing-argument usage error
        /// exits with a different code (2) than the plan requires here.
        phases: Vec<String>,
        /// Path to the current source content, hashed and compared against
        /// each recorded verdict's stored hash to detect staleness. Named,
        /// not positional: `phases` has no `num_args` bound and would
        /// otherwise silently absorb it.
        #[arg(long)]
        source: String,
    },
}

/// `playbook config` subcommands, backing `src/config/`.
#[derive(Subcommand, Debug)]
pub enum ConfigCommand {
    /// Print a key's effective value and which tier supplied it.
    Get {
        /// Dotted config key, e.g. `autoReview.enabled`.
        key: String,
    },
    /// Write a key's value into one tier's config file.
    Set {
        /// Dotted config key, e.g. `autoReview.enabled`.
        key: String,
        /// Raw value to parse into the key's expected type before writing.
        value: String,
        /// Write into the org tier instead of the default repo tier.
        #[arg(long)]
        org: bool,
        /// Write into the global tier instead of the default repo tier.
        #[arg(long)]
        global: bool,
    },
    /// Print every known key's effective value and source tier.
    List,
}

/// `playbook doctor` subcommands, backing `src/doctor/`.
#[derive(Subcommand, Debug)]
pub enum DoctorCommand {
    /// Print `.version` from a `plugin.json`-shaped file, or an empty line
    /// if it is missing, unreadable, or not a string. Backs Layer 6.
    PluginVersion {
        /// Path to the plugin manifest to read.
        path: PathBuf,
    },
    /// Print `.statusLine.command` from a `settings.json`-shaped file, or an
    /// empty line if it is missing, unreadable, or not a string. Backs
    /// Layer 5.
    StatuslineCommand {
        /// Path to the settings.json to read.
        path: PathBuf,
    },
    /// Print every hook `.command` string nested under `settings.json`'s
    /// `.hooks`, one per line, or nothing if the file is unreadable, invalid,
    /// or has no `.hooks` object. Backs Layer 7.
    HookCommands {
        /// Path to the settings.json to read.
        path: PathBuf,
    },
    /// Print one `guard=count` line per guard, each count the number of
    /// `.command` entries wired to that guard under one event. Backs Layer 2.
    HookCommandsForEvent {
        /// Path to the settings.json to read.
        path: PathBuf,
        /// Event name the counts are scoped to, e.g. `PreToolUse`.
        event: String,
        /// Bare guard names, e.g. `rm-workspace-guard`.
        guards: Vec<String>,
    },
    /// Print a single count of `.command` entries matching a regex pattern,
    /// across every event or scoped to one.
    HookCommandsMatching {
        /// Path to the settings.json to read.
        path: PathBuf,
        /// Regex pattern, matched as an unanchored substring search.
        pattern: String,
        /// Event name to scope the count to; omit to count across every event.
        event: Option<String>,
    },
}

/// `playbook worktree` subcommands, backing `src/worktree/`.
#[derive(Subcommand, Debug)]
pub enum WorktreeCommand {
    /// Scan every registered worktree, classify it, and remove any that
    /// have landed and are not locked (or are locked by a dead process).
    Sweep {
        /// Print what would be removed without touching any worktree.
        #[arg(long)]
        dry_run: bool,
    },
    /// Apply the same landed/lock checks to one worktree path.
    Remove {
        /// Worktree path to check, as printed by `git worktree list`.
        path: PathBuf,
    },
}

/// `playbook json` subcommands, backing `src/json/`. Every subcommand
/// reads its JSON payload from stdin rather than a path or inline argument,
/// matching the piped shape a real call site uses:
/// `gh pr checks ... --json bucket | playbook json bucket-counts pass fail`.
#[derive(Subcommand, Debug)]
pub enum JsonCommand {
    /// Tally a piped JSON array's `"bucket"` field against the named
    /// buckets, printing one `bucket=count` line per bucket in the order
    /// given. Backs `src/json/ghjson.rs::bucket_counts`.
    BucketCounts {
        /// Bucket names to tally, e.g. `pass fail pending`.
        buckets: Vec<String>,
    },
    /// Print a piped JSON array's length as a bare integer, 0 for anything
    /// else. Backs `src/json/ghjson.rs::array_length`.
    ArrayLength,
    /// Print one field from a piped flat JSON object, empty for a missing
    /// or malformed field. Backs `src/json/ghjson.rs::field`.
    Field {
        /// The object key to read, e.g. `number` or `headRefOid`.
        key: String,
    },
    /// True/false for whether the piped input is syntactically valid JSON.
    /// Backs `evalfixture::is_valid_json`.
    IsValidJson,
    /// Compact JSON text of the element at `index` in a piped top-level
    /// array, empty for an out-of-bounds index or a non-array top-level
    /// value. Backs `evalfixture::indexed_element`.
    IndexedElement {
        /// Zero-based index into the piped array.
        index: usize,
    },
    /// One field from a piped top-level JSON object as display text, empty
    /// for a missing key or malformed input. Backs
    /// `evalfixture::string_field`.
    StringField {
        /// The object key to read, e.g. `id` or `pr`.
        key: String,
    },
    /// Keys of the piped input's `.lenses` object, sorted alphabetically and
    /// joined with `", "`. Backs `evalfixture::lens_names_joined`.
    LensNamesJoined,
    /// Keys of the piped input's `.lenses` object, sorted alphabetically,
    /// one per line. Backs `evalfixture::lens_names`.
    LensNames,
    /// Reserializes the piped input without extra whitespace, empty for
    /// malformed input. Backs `evalfixture::is_valid_json_compact`.
    IsValidJsonCompact,
    /// Raw text of `.lenses[lens].found` in the piped input; prints `null`
    /// for a missing lens key or missing `found` field. Backs
    /// `evalfixture::lens_found`.
    LensFound {
        /// Lens name to read, e.g. `correctness`.
        lens: String,
    },
    /// Raw text of `.[lens].tier` in the piped input; empty for a
    /// non-object top-level value, a missing lens key, or a missing/null
    /// `tier` field. Backs `evalfixture::lens_tier`.
    LensTier {
        /// Lens name to read, e.g. `correctness`.
        lens: String,
    },
    /// Print the fifteen `key=value` session fields `statusline.sh`'s
    /// `eval` consumes. Backs `src/json/statusline.rs::session_fields`.
    SessionFields {
        /// `$PWD` fallback used when the piped JSON has no `.cwd` or
        /// `.workspace.current_dir`.
        pwd: String,
    },
    /// Reshape a piped raw `gh api graphql` CI status response into
    /// `{"statusCheckRollup": [...]}`, ready to pipe into `ci-rollup`. Backs
    /// `src/json/statusline.rs::graphql_ci_checks`.
    GraphqlCiChecks {},
    /// Print a piped `.statusCheckRollup` array's rollup as one line,
    /// `state failed running total`. Backs
    /// `src/json/statusline.rs::ci_rollup`.
    CiRollup {},
    /// Print the ten `key=value` PR fields `statusline.sh`'s
    /// `render_pr_right` consumes. Backs
    /// `src/json/statusline.rs::pr_fields`.
    PrFields {},
    /// Drop every `"type":"system"` line whose `content` matches `pattern`,
    /// passing every other line through unchanged. Backs
    /// `src/json/jsonl.rs::filter_system_lines`.
    FilterSystemLines {
        /// Regex tested against each system line's `content` field.
        pattern: String,
    },
    /// Rewrite every line's string `sessionId` field to `new_sid`, passing
    /// through a line with no `sessionId` or a non-string one unchanged.
    /// Backs `src/json/jsonl.rs::rewrite_session_id`.
    RewriteSessionId {
        /// Replacement session id.
        new_sid: String,
    },
    /// Exit 0 if stdin parses as JSON, 1 otherwise. Backs
    /// `src/json/validate.rs::is_valid_json`.
    ValidJson,
    /// Exit 0 if the value at `path` inside stdin's JSON is the string
    /// `expected`, 1 otherwise. Backs `src/json/fieldeq.rs::field_equals`.
    FieldEquals {
        /// Dot-separated path; a numeric segment indexes into an array.
        path: String,
        /// Expected string value at `path`.
        expected: String,
    },
    /// Print the length (array element count or object key count) of
    /// `field` in stdin's JSON. Backs `src/json/count.rs::field_length`.
    FieldLength {
        /// Top-level field to measure.
        field: String,
    },
    /// Print the raw string value of `key` in stdin's JSON, no trailing
    /// newline added. Backs `src/json/fields.rs::string_field`. Distinct
    /// from `StringField`: this variant is used where the call site needs
    /// no trailing newline on the printed value.
    RawStringField {
        /// Top-level field to read.
        key: String,
    },
    /// Print stdin's JSON re-serialized with every object's keys sorted
    /// recursively, optionally dropping one top-level key first. Backs
    /// `src/json/canon.rs::canonical_json`.
    CanonicalJson {
        /// Top-level key to drop before sorting, if any.
        #[arg(long)]
        del: Option<String>,
    },
    /// Print every hook `.command` string nested under stdin's JSON's
    /// `.hooks.<event>`, one per line. Backs
    /// `src/json/hookevents.rs::hook_commands_for_event`.
    HookCommandsForEvent {
        /// Event key under `.hooks` to read, e.g. `PreToolUse`.
        event: String,
    },
    /// Print `.projects[project_path].field` from stdin's JSON, where
    /// `project_path` is used verbatim as an object key. Backs
    /// `src/json/claudejson.rs::project_field`.
    ProjectField {
        /// Project path key under `.projects`.
        project_path: String,
        /// Field to read from that project's entry.
        field: String,
    },

    /// Structural JSON equality between two files, ignoring key order and
    /// any named top-level keys. Backs `json::jsoncmp::json_equal`.
    Equal {
        /// First file to compare.
        a: PathBuf,
        /// Second file to compare.
        b: PathBuf,
        /// Top-level keys to drop from both documents before comparing.
        #[arg(long, value_delimiter = ',')]
        ignore_keys: Vec<String>,
    },
    /// Removes top-level keys from a JSON document read on stdin and prints
    /// the result on stdout. Backs `json::settingsjson::remove_keys_print`.
    RemoveKeys {
        /// Top-level keys to drop.
        keys: Vec<String>,
    },
    /// Prints a JSON document's top-level keys, sorted, one per line, read
    /// on stdin. Backs `json::keylist::top_level_keys_sorted`.
    KeysSorted,
    /// Adds one boolean-true marker key to a JSON document read on stdin
    /// and prints the result on stdout. Backs
    /// `json::settingsjson::add_marker_key_print`.
    AddMarkerKey {
        /// Top-level key to set to `true`.
        key: String,
    },
    /// Render the repo-scoped `Facts:`/`Edges:`/`Anchors:` markdown slice of
    /// a memory graph, ported from `shell/memory-context.sh`'s `jq` filter.
    /// Prints nothing (rather than a blank line) when the render is empty,
    /// so the shell script's own `-n "$output"` gate still short-circuits.
    MemoryContext {
        /// Path to the memory graph JSON file.
        graph_file: PathBuf,
        /// Repo slug (`owner/name`) to scope facts to.
        repo: String,
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
    NoSlopGuard,
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
            "no-slop-guard",
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
    fn path_help_lists_all_three_kinds() {
        // Arrange
        let kinds = ["plans", "implement", "worktrees"];

        // Act
        let result = Cli::command().try_get_matches_from(["playbook", "path", "--help"]);

        // Assert
        let err = result.expect_err("--help should short-circuit parsing with a help message");
        let help = err.to_string();
        for kind in kinds {
            assert!(help.contains(kind), "path --help is missing '{kind}'");
        }
    }

    #[test]
    fn path_kind_dir_name_matches_each_variant() {
        // Arrange, Act, Assert
        assert_eq!(PathKind::Plans.dir_name(), "plans");
        assert_eq!(PathKind::Implement.dir_name(), "implement");
        assert_eq!(PathKind::Worktrees.dir_name(), "worktrees");
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
