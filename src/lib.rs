// SPDX-FileCopyrightText: 2026 Igor Santos
// SPDX-License-Identifier: Apache-2.0

//! Library root for the `playbook` binary. Exposes the CLI shape that
//! `main.rs` parses and dispatches on, plus the `common` helpers and the
//! `hooks` modules, one per hook.

pub mod agents;
pub mod cc;
pub mod ci;
pub mod common;
pub mod config;
pub mod deps;
pub mod doctor;
pub mod effort;
pub mod eval;
pub mod gate;
pub mod handoff;
pub mod hooks;
pub mod init;
pub mod json;
pub mod manifest;
pub mod memory_import;
pub mod mode;
pub mod models;
pub mod plans;
pub mod pr;
pub mod release;
pub mod review;
pub mod sanitize;
pub mod settings;
pub mod statusline;
pub mod trust;
pub mod uninstall;
pub mod update;
pub mod usage;
pub mod worktree;

use clap::{Parser, Subcommand, ValueEnum};
use std::path::PathBuf;

/// Configure and support Claude Code from the command line.
///
/// playbook installs hooks and a session launcher, shows a usage dashboard,
/// and helps with pull requests, reviews, worktrees and session handoffs.
#[derive(Parser, Debug)]
#[command(
    name = "playbook",
    version,
    after_help = "Run `playbook <command> --help` for details on one command.\n\
Check your setup with `/playbook:doctor` inside Claude Code (`playbook doctor --help` lists its helpers).\n\
Docs: https://github.com/pragmatic-engineer/playbook"
)]
pub struct Cli {
    #[command(subcommand)]
    pub command: Command,
}

// Variant order is the order users see in `--help`; nothing else depends on it.
/// `playbook effort` subcommands.
#[derive(Subcommand, Debug)]
pub enum EffortCommand {
    /// Show one component's shipped effort, ceiling and effective level
    ///
    /// For an agent it also names the file to dispatch and every file allowed
    /// under the ceiling.
    ///
    /// Example: `playbook effort resolve agents reviewer --json`
    Resolve {
        /// agents, commands or skills
        kind: String,
        /// The component name, such as `reviewer` or `deep-review`
        name: String,
        /// Print JSON
        #[arg(long)]
        json: bool,
    },
    /// List every component with its shipped effort and effective level
    List {
        /// Print JSON
        #[arg(long)]
        json: bool,
    },
}

/// Top-level subcommand groups.
#[derive(Subcommand, Debug)]
pub enum Command {
    /// Install or repair your local Claude Code setup
    ///
    /// Safe to run again: it only touches what is missing or out of date.
    /// In a terminal it asks about each optional part (hooks, settings, PATH,
    /// the `ccc` launcher, the system prompt) unless you answer with a flag.
    /// Each part has `--x` and `--no-x`. Without a terminal, or with `--yes`,
    /// unanswered parts use their defaults: hooks, settings and PATH on, the
    /// launcher and system prompt off.
    ///
    /// Example: `playbook init --no-hooks --aliases`
    Init {
        #[command(flatten)]
        flags: init::choices::Flags,
    },
    /// Remove what playbook installed from this machine
    ///
    /// Takes out the hook entries and status line that `playbook init` wrote
    /// into `settings.json`, the launcher block in your shell rc files, and
    /// the files playbook placed under `~/.config/playbook`. Everything else
    /// in `settings.json` and the rc files stays, each changed file is backed
    /// up first, and memory, sessions and history are never touched. The
    /// `playbook` binary stays unless you pass `--remove-binary`. It changes
    /// nothing without `--yes`; `--dry-run` lists what would go.
    ///
    /// Example: `playbook uninstall --dry-run`
    Uninstall {
        /// Go ahead and remove (without it, nothing is changed)
        #[arg(long)]
        yes: bool,
        /// List what would be removed and change nothing
        #[arg(long)]
        dry_run: bool,
        /// Also remove the `playbook` binary, its backups and the PATH line the installer added
        #[arg(long)]
        remove_binary: bool,
    },
    /// Print the shell functions `ccc` and `ccd`
    ///
    /// `ccc` starts a session (named so it does not shadow the C compiler,
    /// `cc`), and `ccd` does the same with permission prompts skipped. Load
    /// them from your rc file with `eval "$(playbook shell-init)"`.
    ShellInit {
        /// Target shell (default: the one named by `$SHELL`)
        #[arg(long, value_parser = ["bash", "zsh"])]
        shell: Option<String>,
    },
    /// Update playbook to a published release
    ///
    /// Downloads the release, checks its SHA256 sum and, when `gh` is
    /// installed, its build attestation, then swaps the binary and keeps the
    /// old one as a backup. In auto mode it refuses unless you pass `--yes`.
    /// `--check` and `--list` change nothing. `--check` exits 0 whether or not
    /// an update exists, and 1 on an error such as an unreachable release list
    /// or an unknown version.
    ///
    /// Example: `playbook update --check`
    #[command(alias = "upgrade")]
    Update {
        /// Version to install (default: the latest stable release)
        version: Option<String>,
        /// Only report whether an update is available
        #[arg(long)]
        check: bool,
        /// List published releases
        #[arg(long)]
        list: bool,
        /// Consider pre-releases when picking the latest
        #[arg(long)]
        pre: bool,
        /// Confirm the update when auto mode is on
        #[arg(long)]
        yes: bool,
    },
    /// Print the installed version, same as `--version`
    Version,
    /// Install the tools a Brewfile lists, keeping any already on PATH
    ///
    /// Example: `playbook deps ensure Brewfile`
    Deps {
        #[command(subcommand)]
        sub: DepsCommand,
    },
    /// Read single facts that the health check uses
    ///
    /// The full health check is the `/playbook:doctor` command inside Claude
    /// Code. These subcommands print one fact each, such as pending
    /// migrations, plugin version or hook wiring, and help when a layer
    /// reports a miss.
    ///
    /// Example: `playbook doctor pending-migrations`
    Doctor {
        #[command(subcommand)]
        sub: DoctorCommand,
    },
    /// Choose whether playbook asks questions or decides on its own
    ///
    /// `ask` is the default. `auto` lets commands pick the recommended option
    /// and keep going. The choice is stored per repo.
    ///
    /// Example: `playbook mode auto`
    Mode {
        #[command(subcommand)]
        sub: ModeCommand,
    },
    /// Read and change playbook settings
    ///
    /// Settings come from tiers, highest first: repo, org, global, then the
    /// built-in default. `get` and `list` show which tier supplied a value.
    ///
    /// Example: `playbook config set autoReview.enabled true`
    Config {
        #[command(subcommand)]
        sub: ConfigCommand,
    },
    /// Set playbook's own ceiling on effort, or inspect one component's
    ///
    /// `playbook effort list` shows every command, skill and agent with its
    /// shipped effort and effective level. `playbook effort resolve agents
    /// reviewer` says which agent file to dispatch under the ceilings. Set a
    /// per-component ceiling with `playbook config set --global
    /// effort.agents.reviewer medium`.
    ///
    /// `auto` (the default) sets no ceiling, so Claude Code's own
    /// `maxEffortLevel` decides how high effort may go. Each skill, command and
    /// agent still keeps the effort it ships with. `low`, `medium`, `high`,
    /// `xhigh` or `max` set playbook's own ceiling. The key is
    /// `maxEffortLevel`, named like Claude Code's. The effective ceiling is the lower of this and Claude Code's own
    /// `maxEffortLevel`, which playbook only reads and never changes. Sessions
    /// started with `ccc` or `ccd` get the ceiling. With no level, shows both
    /// values and the one that wins.
    ///
    /// Example: `playbook effort xhigh`
    #[command(args_conflicts_with_subcommands = true)]
    Effort {
        #[command(subcommand)]
        sub: Option<EffortCommand>,
        /// auto, low, medium, high, xhigh or max
        level: Option<String>,
        /// Print the status as JSON (with no level)
        #[arg(long)]
        json: bool,
    },
    /// Launcher helpers behind the `ccc` shell shortcut
    ///
    /// Sessions are started by the `ccc` shell shortcut (install it with
    /// `playbook init --aliases`). These subcommands do its housekeeping:
    /// list sessions, prune state, clear caches, and create worktrees.
    ///
    /// Example: `playbook cc worktree my-branch`
    Cc {
        #[command(subcommand)]
        sub: Option<CcCommand>,
    },
    /// Show token and cost usage across your sessions
    ///
    /// With no subcommand, prints a summary of estimated cost across all
    /// recorded sessions.
    ///
    /// Example: `playbook usage dashboard`
    Usage {
        #[command(subcommand)]
        sub: Option<UsageCommand>,
    },
    /// Render the files the release job pushes to the Homebrew tap and marketplace
    ///
    /// Run by the release workflow after a tag. Prints the file to stdout and
    /// never touches the network.
    Release {
        #[command(subcommand)]
        sub: ReleaseCommand,
    },
    /// Check how well the review triage classifier sorts real pull requests
    ///
    /// Each case is a pull request with a known answer per review lens. Costs
    /// one `gh pr diff` fetch and one live `claude` call per case, so run it
    /// by hand or on a schedule.
    ///
    /// Example: `playbook eval review-triage`
    Eval {
        #[command(subcommand)]
        sub: EvalCommand,
    },
    /// Prepare and open pull requests, and pick a review depth
    ///
    /// These are the mechanical steps behind `/playbook:create-pull-request`.
    ///
    /// Example: `playbook pr prepare --base main`
    Pr {
        #[command(subcommand)]
        sub: PrCommand,
    },
    /// Clean up git worktrees whose branches have landed
    ///
    /// Locked worktrees are kept unless the process that locked them is gone.
    ///
    /// Example: `playbook worktree sweep --dry-run`
    Worktree {
        #[command(subcommand)]
        sub: WorktreeCommand,
    },
    /// Manage the memory graph that sessions read from
    Memory {
        #[command(subcommand)]
        sub: MemoryCommand,
    },
    /// Save and show notes that carry a session over to the next one
    ///
    /// A handoff is a markdown note for one directory. The next session started
    /// there loads it.
    Handoff {
        #[command(subcommand)]
        sub: HandoffCommand,
    },
    /// Run one Claude Code hook by name
    ///
    /// Claude Code calls this from its settings, so you rarely type it.
    /// Hidden from the command list but fully working.
    #[command(hide = true)]
    Hook {
        /// Which hook to run, matching the name Claude Code passes from its hook settings
        name: HookName,
    },
    /// Print the Claude Code status line
    ///
    /// Claude Code runs this to draw the line at the bottom of a session.
    Statusline,
    /// Create or check the shared settings template
    Settings {
        #[command(subcommand)]
        sub: SettingsCommand,
    },
    /// Check that tracked files sit in allowed top-level paths
    Manifest {
        #[command(subcommand)]
        sub: ManifestCommand,
    },
    /// Check agent definition files
    Agents {
        #[command(subcommand)]
        sub: AgentsCommand,
    },
    /// Record and check plan verdicts used as gates
    ///
    /// Verdicts are stored per worktree, keyed by plan.
    Gate {
        #[command(subcommand)]
        sub: GateCommand,
    },
    /// Remove AI attribution lines from message text
    ///
    /// For callers that are not an agent hook, such as a git `commit-msg`
    /// hook or CI. Each subcommand rewrites FILE in place unless `--check` is
    /// given, and prints one line per removed line to stderr: its number and
    /// the shape of the attribution, never its text. Without `--check` the exit
    /// code is always 0, even when FILE cannot be read or written.
    Sanitize {
        #[command(subcommand)]
        sub: SanitizeCommand,
    },
    /// Run the repo checks that need no model (manifest, agents, settings)
    ///
    /// Prints one line per check. Exits 0 when all pass and 1 if any fails,
    /// so it is safe in CI. A skipped check passes unless `--strict` is set.
    ///
    /// Example: `playbook ci --strict`
    Ci {
        /// Print one JSON object instead of text
        #[arg(long)]
        json: bool,
        /// Also exit 1 when a check is skipped (for example in the wrong directory)
        #[arg(long)]
        strict: bool,
        /// Run in this directory instead of the current one
        #[arg(long)]
        dir: Option<String>,
    },
    /// Print the path of one of this repo's playbook storage folders
    ///
    /// First moves any legacy folders from the old repo-local location.
    Path {
        /// Which folder to print
        kind: PathKind,
        /// Create the folder when it does not exist
        #[arg(long)]
        create: bool,
    },
    /// Setup for the review commands
    Review {
        #[command(subcommand)]
        sub: ReviewCommand,
    },
    /// List the plans and ADR blueprints `/playbook:implement` can run
    ///
    /// Prints `path<TAB>[status]<TAB>title` per file, or `NO_PLANS`.
    Plans {
        /// Print JSON instead of text
        #[arg(long)]
        json: bool,
    },
    /// Small JSON helpers for shell scripts
    ///
    /// Reads JSON on stdin. Hidden from the command list, still works, and is
    /// slated for removal.
    #[command(hide = true)]
    Json {
        #[command(subcommand)]
        sub: JsonCommand,
    },
    /// Mark a directory as trusted in Claude Code
    ///
    /// Writes to `~/.claude.json` so Claude Code's first-launch trust prompt
    /// does not block `ccc` or `ccd`. Always exits 0, even on failure, so a
    /// script can call it without checking.
    Trust {
        /// Absolute path of the directory to trust
        path: String,
    },
}

/// `playbook eval` subcommands.
#[derive(Subcommand, Debug)]
pub enum EvalCommand {
    /// Run the review triage classifier on each case and compare with the known answers
    ///
    /// Needs network access, a logged-in `gh`, and a working `claude` with a
    /// live API key. Exits 0 when no lens was wrongly skipped and no case
    /// errored, and 1 otherwise. Re-run it whenever the triage prompt changes.
    ///
    /// Example: `playbook eval review-triage`
    ReviewTriage {
        /// Case file to run; defaults to the cases shipped in the playbook repo
        fixtures: Option<PathBuf>,
        /// Triage prompt to send; defaults to the one shipped in the playbook repo
        #[arg(long)]
        prompt: Option<PathBuf>,
    },
}

/// `playbook release` subcommands.
#[derive(Subcommand, Debug)]
pub enum ReleaseCommand {
    /// Print the Homebrew formula for VERSION, filled from a SHA256SUMS file
    RenderFormula { version: String, sums: PathBuf },
    /// Print marketplace.json with the playbook plugin pinned to VERSION's archive
    PinMarketplace {
        version: String,
        marketplace: PathBuf,
        sha256: String,
        /// The `owner/name` repository the archive URL points at
        #[arg(long, default_value = "pragmatic-engineer/playbook")]
        repo: String,
    },
}

/// `playbook mode` subcommands.
#[derive(Subcommand, Debug)]
pub enum ModeCommand {
    /// Turn auto mode on for this repo
    Auto,
    /// Turn auto mode off for this repo, so playbook asks again
    Ask,
    /// Show the current mode and where it came from
    Status {
        /// Print one JSON object instead of a line of text
        #[arg(long)]
        json: bool,
        /// Report as if a command ran with `--auto` or `--ask`, and warn when the hooks would disagree
        #[arg(long, value_enum)]
        flag: Option<ModeArg>,
    },
}

/// A mode as typed on the command line.
#[derive(ValueEnum, Debug, Clone, Copy)]
pub enum ModeArg {
    Ask,
    Auto,
}

/// `playbook handoff` subcommands.
#[derive(Subcommand, Debug)]
pub enum HandoffCommand {
    /// Save the handoff note read from stdin for the next session
    Save {
        /// Directory the handoff belongs to (default: the current one)
        #[arg(long)]
        dir: Option<String>,
    },
    /// Print this directory's handoff without using it up
    Show {
        /// Print every waiting handoff, not just the newest
        #[arg(long)]
        all: bool,
        /// Directory to show (default: the current one)
        #[arg(long)]
        dir: Option<String>,
    },
    /// Show recent session starts and this directory's handoff counts
    Status,
}

/// Which worktree-scoped storage directory `playbook path` prints.
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

/// `playbook settings` subcommands.
#[derive(Subcommand, Debug)]
pub enum SettingsCommand {
    /// Create the shared settings template from a live settings.json
    Gen {
        /// Live settings.json to start from
        src: PathBuf,
        /// File holding the default permissions
        perms: PathBuf,
    },
    /// Check the shared settings template is valid
    ///
    /// Exits 0 when valid and 1 with a message when not.
    Check {
        /// Shared settings template to check
        template: PathBuf,
        /// Shared permissions file
        perms: PathBuf,
        /// Repo root that every hook command must resolve inside
        repo_root: PathBuf,
    },
}

/// `playbook memory` subcommands.
#[derive(Subcommand, Debug)]
pub enum MemoryCommand {
    /// Rebuild the memory graph from every fact on disk
    ///
    /// Memory is rebuilt on its own when a fact is saved, so you only need
    /// this after editing fact files by hand.
    Rebuild,
    /// Copy notes from Claude Code's auto memory into playbook memory
    ///
    /// One way and opt in. It only reads Claude Code's memory for this
    /// project (`~/.claude/projects/<project>/memory/*.md`) and never changes,
    /// moves or deletes anything there. Each note becomes a playbook fact in
    /// this repo's project memory. A note already imported (tracked by content
    /// hash on the playbook side) and a fact that already exists are skipped,
    /// so running it again copies nothing.
    ///
    /// Example: `playbook memory import-claude --dry-run`
    ImportClaude {
        /// Show what would be copied and write nothing
        #[arg(long)]
        dry_run: bool,
        /// Read this directory instead of the project's Claude Code memory
        #[arg(long)]
        from: Option<PathBuf>,
    },
    /// Print the repo-scoped markdown slice of the memory graph (facts in
    /// scope, typed edges, anchor index). Prints nothing, exit 0, when the
    /// graph is missing or unreadable.
    Context {
        /// Repo slug (`owner/name`); defaults to the origin remote's slug.
        #[arg(long)]
        repo: Option<String>,
        /// Graph file; defaults to `~/.config/playbook/memory/memory.graph.json`.
        #[arg(long)]
        graph: Option<PathBuf>,
    },
}

/// `playbook manifest` subcommands.
#[derive(Subcommand, Debug)]
pub enum ManifestCommand {
    /// Check every tracked file is at an allowed top-level path
    ///
    /// Exits 0 when all pass and 1 otherwise.
    Check {
        /// Repo root to check (default: the repo you are in)
        repo_root: Option<PathBuf>,
    },
}

/// `playbook agents` subcommands.
#[derive(Subcommand, Debug)]
pub enum AgentsCommand {
    /// Check every agent definition follows the agent rules
    ///
    /// Exits 0 when all pass and 1 otherwise.
    Check {
        /// Folder of agent definitions (default: `agents` at the repo root)
        agents_dir: Option<PathBuf>,
    },
    /// Write the effort-tier variants (`reviewer-low`, `reviewer-xhigh`, and
    /// so on) from their base agent files. Idempotent and deterministic.
    Gen {
        /// Directory holding the agent definitions, default `<repo root>/agents`.
        agents_dir: Option<PathBuf>,
    },
}

/// `playbook usage` subcommands.
#[derive(Subcommand, Debug)]
pub enum UsageCommand {
    /// Read new session history into the usage store without printing a summary
    Ingest,
    /// Open the local usage dashboard in your browser
    ///
    /// Starts the dashboard server if it is not running. Stop it with
    /// `playbook usage dashboard stop`.
    Dashboard {
        // Internal flag: runs the detached server process.
        #[arg(long, hide = true)]
        serve: bool,
        #[command(subcommand)]
        sub: Option<DashboardCommand>,
    },
}

/// `playbook usage dashboard` subcommands.
#[derive(Subcommand, Debug)]
pub enum DashboardCommand {
    /// Stop the running dashboard server
    Stop,
}

/// `playbook sanitize` subcommands.
#[derive(Subcommand, Debug)]
pub enum SanitizeCommand {
    /// Clean a commit message file, such as git's `.git/COMMIT_EDITMSG`
    CommitMsg {
        /// File holding the message
        file: PathBuf,
        /// Report what would be removed and exit 1 if anything would, without writing (exits 2 if FILE cannot be read)
        #[arg(long)]
        check: bool,
    },
    /// Clean a PR title or body saved to a file
    PrText {
        /// File holding the text
        file: PathBuf,
        /// Report what would be removed and exit 1 if anything would, without writing (exits 2 if FILE cannot be read)
        #[arg(long)]
        check: bool,
    },
}

/// `playbook pr` subcommands.
#[derive(Subcommand, Debug)]
pub enum PrCommand {
    /// Run pre-flight checks and gather what a PR draft needs
    ///
    /// Finds the branch and base, checks the repo is ready, writes the diff
    /// to a scratch file, and prints labeled lines for the PR drafter.
    Prepare {
        /// Base branch for the PR (default: the repo's default branch)
        #[arg(long)]
        base: Option<String>,
        /// Ticket id to cite (default: one found in the branch name)
        #[arg(long)]
        ticket: Option<String>,
        /// Work in this directory instead of the current one
        #[arg(long)]
        dir: Option<String>,
    },
    /// Push the branch and open a PR from a drafted title and body
    ///
    /// Opens a draft unless the `pr.draft` setting is `false`.
    Create {
        /// PR title, at most 72 characters
        #[arg(long)]
        title: String,
        /// File holding the PR body
        #[arg(long)]
        body_file: String,
        /// Base branch for the PR (default: the repo's default branch)
        #[arg(long)]
        base: Option<String>,
        /// Work in this directory instead of the current one
        #[arg(long)]
        dir: Option<String>,
    },
    /// Choose a quick or deep self-review for a PR
    ///
    /// Asks a small model three times. Anything but a unanimous `quick`
    /// becomes `deep`.
    ReviewTriage {
        /// PR number (default: the current branch against its base)
        #[arg(long)]
        pr: Option<u64>,
        /// Base branch when no PR number is given (default: the repo's default branch)
        #[arg(long)]
        base: Option<String>,
        /// Work in this directory instead of the current one
        #[arg(long)]
        dir: Option<String>,
    },
    /// Wait for a PR's required checks to finish
    ///
    /// Prints a `checks total=.. pending=.. fail=.. cancel=..` line per read,
    /// then `CI_VERDICT=` one of NONE, FAIL, CANCELLED, PASS or TIMEOUT.
    CiWait {
        /// PR number
        pr: String,
        /// Give up after this many seconds
        #[arg(long, default_value_t = 1200)]
        timeout: u64,
        /// Seconds between reads
        #[arg(long, default_value_t = 20)]
        interval: u64,
    },
    /// Wait for a PR to merge or hit a gate
    ///
    /// Prints a status line per read (state, merge state, review, auto-merge),
    /// then `LAND_VERDICT=` one of MERGED, REVIEW_GATE, CHANGES_REQUESTED,
    /// RESTATE or TIMEOUT.
    LandWait {
        /// PR number
        pr: String,
        /// Give up after this many seconds
        #[arg(long, default_value_t = 1800)]
        timeout: u64,
        /// Seconds between reads
        #[arg(long, default_value_t = 20)]
        interval: u64,
    },
    /// Merge a PR with auto-merge, or as an admin squash
    ///
    /// Prints `merge_rc=<code>` (or `admin_rc=`) and then what `gh` said.
    /// Always exits 0, so the caller reads the code.
    Merge {
        /// PR number
        pr: String,
        /// Squash with admin rights instead of arming auto-merge
        #[arg(long)]
        admin: bool,
    },
}

/// `playbook review` subcommands.
#[derive(Subcommand, Debug)]
pub enum ReviewCommand {
    /// Resolve the PR, report-only or posting, and in-place or worktree
    ///
    /// Takes the raw arguments of the review command as one string. Prints
    /// `KEY=value` lines: PR, REPO, PR_NUMBER, HEAD_SHA, AUTHOR, SELF_REVIEW,
    /// SELF_MODE, MODE, WT and REVIEW_JSON. On a problem it prints
    /// `error: ...` and exits 1.
    ///
    /// Example: `playbook review prepare deep "#4265 --self"`
    Prepare {
        /// Which review: `quick` or `deep`
        #[arg(value_parser = ["quick", "deep"])]
        kind: String,
        /// The review command's arguments, as one string
        #[arg(default_value = "")]
        args: String,
        /// Treat the run mode as auto (implies report-only)
        #[arg(long)]
        auto: bool,
    },
    /// Run a project's own type check, lint and tests in a directory
    ///
    /// Prints their output. A failing check is output, not an error.
    Checks {
        /// The review worktree to run in
        dir: PathBuf,
    },
}

/// `playbook gate` subcommands.
#[derive(Subcommand, Debug)]
pub enum GateCommand {
    /// Save a phase verdict for a plan
    ///
    /// Reads a phase agent's output, finds its `VERDICT:` line, and stores it.
    Record {
        /// Plan slug the phase belongs to
        plan_slug: String,
        /// Command that produced this verdict
        command: String,
        /// Which phase this verdict is for
        phase: String,
        /// File with the phase agent's output, or "-" to read stdin
        input: String,
        /// File with the source content the verdict covers, used to spot a stale verdict later
        #[arg(long)]
        source: String,
    },
    /// Check that recorded phase verdicts still pass
    ///
    /// Exits 0 only if every named phase is PASS or WARN and its source is
    /// unchanged. Exits 1 on any other verdict, a stale one, or no phases.
    Check {
        /// Plan slug to check
        plan_slug: String,
        /// Command running this check
        command: String,
        /// Phase names to check
        phases: Vec<String>,
        /// File with the current source content, compared against what each verdict saw
        #[arg(long)]
        source: String,
        /// Print one JSON object instead of text (exit codes stay the same)
        #[arg(long)]
        json: bool,
    },
}

/// `playbook config` subcommands.
#[derive(Subcommand, Debug)]
pub enum ConfigCommand {
    /// Show a setting's value and which tier it came from
    Get {
        /// Dotted setting name, such as `autoReview.enabled`
        key: String,
    },
    /// Change a setting
    ///
    /// Writes to the repo tier unless you pass `--org` or `--global`.
    Set {
        /// Dotted setting name, such as `autoReview.enabled`
        key: String,
        /// New value; it is read as the type the setting expects
        value: String,
        /// Write to the org tier instead of the repo tier
        #[arg(long)]
        org: bool,
        /// Write to the global tier instead of the repo tier
        #[arg(long)]
        global: bool,
    },
    /// List every setting with its value and source tier
    List,
    /// Print every stored setting as one JSON document
    ///
    /// Settings live in a database, not hand-editable files. Export gives you
    /// a readable copy, and `config import` loads one back.
    ///
    /// Example: `playbook config export > playbook-config.json`
    Export,
    /// Load settings from a JSON document made by `config export`
    ///
    /// Every key and value is checked first, and nothing is stored if any is
    /// invalid. Use `-` to read standard input.
    ///
    /// Example: `playbook config import playbook-config.json`
    Import {
        /// The JSON file to read, or `-` for standard input
        file: String,
    },
}

/// `playbook doctor` subcommands.
#[derive(Subcommand, Debug)]
pub enum DoctorCommand {
    /// Check the seven playbook layers and print a status table
    ///
    /// Reads your settings, shell files and PATH, and lists a PASS, INFO, WARN or
    /// FAIL row for each layer, then the review config, stale worktrees and
    /// pending migration notices. Always exits 0 when it could run the checks.
    Check {
        /// Print the rows as JSON
        #[arg(long)]
        json: bool,
    },
    /// List files you edited after playbook placed them
    ///
    /// Prints one line per finding, or nothing when none is pending.
    PendingMigrations,
    /// Print the version from a plugin manifest
    ///
    /// Prints an empty line if the file is missing, unreadable, or has no version.
    PluginVersion {
        /// Plugin manifest to read
        path: PathBuf,
    },
    /// Print the status line command from a settings.json
    ///
    /// Prints an empty line if the file is missing, unreadable, or has none.
    StatuslineCommand {
        /// settings.json to read
        path: PathBuf,
    },
    /// List every hook command in a settings.json, one per line
    ///
    /// Prints nothing if the file is unreadable, invalid, or has no hooks.
    HookCommands {
        /// settings.json to read
        path: PathBuf,
    },
    /// Count the commands wired to each named guard for one event
    ///
    /// Prints one `guard=count` line per guard.
    HookCommandsForEvent {
        /// settings.json to read
        path: PathBuf,
        /// Event name, such as `PreToolUse`
        event: String,
        /// Guard names, such as `rm-workspace-guard`
        guards: Vec<String>,
    },
    /// Count hook commands that match a pattern
    ///
    /// Counts across every event, or only one if you name it.
    HookCommandsMatching {
        /// settings.json to read
        path: PathBuf,
        /// Regex to look for anywhere in a command
        pattern: String,
        /// Only count this event (default: every event)
        event: Option<String>,
    },
}

/// `playbook deps` subcommands.
#[derive(Subcommand, Debug)]
pub enum DepsCommand {
    /// Keep each tool on PATH and install only the missing ones with
    /// Homebrew, from the `brew` and `tap` lines of a Brewfile.
    Ensure {
        /// Brewfile to read; defaults to `Brewfile` in the current directory.
        brewfile: Option<PathBuf>,
    },
}

/// `playbook worktree` subcommands.
#[derive(Subcommand, Debug)]
pub enum WorktreeCommand {
    /// Remove every landed worktree that is not in use
    ///
    /// Scans all worktrees of this repo, whichever tool made them. Keeps
    /// locked ones unless the locking process is dead.
    Sweep {
        /// Show what would be removed without removing anything
        #[arg(long)]
        dry_run: bool,
    },
    /// Remove one worktree if it has landed and is not in use
    Remove {
        /// Worktree path, as shown by `git worktree list`
        path: PathBuf,
    },
    /// Create or remove the locked review worktrees `/playbook:quick-review`
    /// and `/playbook:deep-review` run in
    Review {
        #[command(subcommand)]
        sub: ReviewWorktreeCommand,
    },
}

/// `playbook worktree review` subcommands.
#[derive(Subcommand, Debug)]
pub enum ReviewWorktreeCommand {
    /// Fetch a PR head, add a locked detached worktree for it, and print its
    /// absolute path on stdout (warnings go to stderr).
    Setup {
        /// Pull request number.
        pr: String,
        /// Head commit the review was resolved against.
        head_sha: String,
    },
    /// Remove a review worktree, even a dirty one. Always exits 0.
    Teardown {
        /// Worktree path printed by `setup`.
        path: PathBuf,
    },
}

/// `playbook json` subcommands. Each reads its JSON from stdin, as in
/// `gh pr checks --json bucket | playbook json bucket-counts pass fail`.
#[derive(Subcommand, Debug)]
pub enum JsonCommand {
    /// Count array items per `bucket` value, one `bucket=count` line each
    BucketCounts {
        /// Bucket names to count, such as `pass fail pending`
        buckets: Vec<String>,
    },
    /// Print the length of a JSON array (0 for anything else)
    ArrayLength,
    /// Print one field of a flat JSON object (empty if missing)
    Field {
        /// Object key to read, such as `number`
        key: String,
    },
    /// Print the session fields the status line needs, as `key=value` lines
    SessionFields {
        /// Directory to use when the input names none
        pwd: String,
    },
    /// Reshape a raw GraphQL CI response into a `statusCheckRollup` object
    GraphqlCiChecks {},
    /// Print one line summarizing a `statusCheckRollup` array
    ///
    /// The line reads `state failed running total`.
    CiRollup {},
    /// Print the PR fields the status line needs, as `key=value` lines
    PrFields {},
    /// Drop system lines whose content matches a pattern
    ///
    /// Every other line passes through unchanged.
    FilterSystemLines {
        /// Regex tested against each system line's content
        pattern: String,
    },
    /// Set every line's `sessionId` to a new value
    ///
    /// Lines with no string `sessionId` pass through unchanged.
    RewriteSessionId {
        /// New session id
        new_sid: String,
    },
    /// Exit 0 if stdin is valid JSON, 1 otherwise
    ValidJson,
    /// Exit 0 if the value at a path is the expected string, 1 otherwise
    FieldEquals {
        /// Dot-separated path; a number indexes into an array
        path: String,
        /// Expected string value
        expected: String,
    },
    /// Print the length of a field (item count or key count)
    FieldLength {
        /// Top-level field to measure
        field: String,
    },
    /// Print a field's string value with no trailing newline
    RawStringField {
        /// Top-level field to read
        key: String,
    },
    /// Print `.projects[project_path].field`
    ProjectField {
        /// Project path, used as the object key
        project_path: String,
        /// Field to read from that project's entry
        field: String,
    },
    /// Check two JSON files hold the same data, ignoring key order
    Equal {
        /// First file
        a: PathBuf,
        /// Second file
        b: PathBuf,
        /// Top-level keys to ignore in both files, comma separated
        #[arg(long, value_delimiter = ',')]
        ignore_keys: Vec<String>,
    },
    /// Remove top-level keys from the input and print the result
    RemoveKeys {
        /// Top-level keys to remove
        keys: Vec<String>,
    },
    /// Print the input's top-level keys, sorted, one per line
    KeysSorted,
    /// Add one key set to `true` and print the result
    AddMarkerKey {
        /// Key to set to `true`
        key: String,
    },
    /// Print the repo's slice of a memory graph as markdown
    ///
    /// Prints nothing when the slice is empty.
    MemoryContext {
        /// Memory graph JSON file
        graph_file: PathBuf,
        /// Repo slug (`owner/name`) to scope facts to
        repo: String,
    },
}

// Kebab-case on the CLI, so a typo in the hook settings fails loudly.
/// Every hook Claude Code can run.
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
    CommitMessageSanitizer,
    PrecommitCheck,
    AutoGuard,
    AutoCost,
    WorktreeCreate,
    WorktreeRemove,
    MigrationCheck,
}

/// `playbook cc` subcommands.
#[derive(Subcommand, Debug)]
pub enum CcCommand {
    /// Mode of the `ccc` shortcut: clean up and resume (no action by itself)
    Clean,
    /// Mode of the `ccc` shortcut: start a fresh session (no action by itself)
    Fresh,
    /// Mode of the `ccc` shortcut: resume a session as it was (no action by itself)
    ///
    /// The `ccc` shortcut handles these modes. Running this command directly
    /// does nothing and exits 0.
    Raw {
        /// Session id to resume (default: the most recent)
        sid: Option<String>,
    },
    /// List sessions for the current project
    #[command(alias = "ls")]
    List,
    /// Remove stale runtime state
    Prune,
    /// Clear caches that would freeze old settings into a session
    #[command(name = "bust-cache")]
    BustCache,
    /// Run the launcher the `ccc` and `ccd` shell functions call
    Launch {
        /// Pass `--dangerously-skip-permissions` to claude (the `ccd` form)
        #[arg(long)]
        skip_permissions: bool,
        /// Subcommand and arguments, passed through to claude
        #[arg(trailing_var_arg = true, allow_hyphen_values = true)]
        args: Vec<String>,
    },
    /// Background half of `ccc worktree`, spawned by the launcher
    #[command(hide = true)]
    Housekeep {
        #[arg(long)]
        repo_root: String,
        #[arg(long)]
        worktree: String,
        #[arg(long)]
        branch: String,
        #[arg(long)]
        no_push: bool,
    },
    /// Create a git worktree for a branch and print its path
    ///
    /// Prints only the path, so a shell can run `cd "$(playbook cc worktree my-branch)"`.
    #[command(alias = "new")]
    Worktree {
        /// Branch to create the worktree for
        branch: String,
        /// Folder (relative to the repo root) holding the `.env` to copy in
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
    fn hook_help_lists_every_hook_name() {
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
            "auto-guard",
            "auto-cost",
            "precompact-warn",
            "session-clean-exit",
            "memory-capture",
            "rm-workspace-guard",
            "bg-await-guard",
            "no-slop-guard",
            "precommit-check",
            "commit-message-sanitizer",
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

    fn help_texts(cmd: &clap::Command, path: &str, out: &mut Vec<(String, String)>) {
        let here = format!("{path} {}", cmd.get_name());
        for text in [
            cmd.get_about(),
            cmd.get_long_about(),
            cmd.get_before_help(),
            cmd.get_before_long_help(),
            cmd.get_after_help(),
            cmd.get_after_long_help(),
        ]
        .into_iter()
        .flatten()
        {
            out.push((here.clone(), text.to_string()));
        }
        for arg in cmd.get_arguments() {
            for text in [arg.get_help(), arg.get_long_help()].into_iter().flatten() {
                out.push((format!("{here} --{}", arg.get_id()), text.to_string()));
            }
            for value in arg.get_possible_values() {
                if let Some(text) = value.get_help() {
                    out.push((format!("{here} {}", value.get_name()), text.to_string()));
                }
            }
        }
        for sub in cmd.get_subcommands() {
            help_texts(sub, &here, out);
        }
    }

    #[test]
    fn user_facing_help_never_names_source_files_or_rust_paths() {
        // Arrange
        let mut texts = Vec::new();

        // Act
        help_texts(&Cli::command(), "", &mut texts);

        // Assert
        assert!(texts.len() > 100, "the walk found too little help text");
        for (place, text) in texts {
            for banned in ["src/", "shell/", "::", ".rs", ".sh", ".py"] {
                assert!(
                    !text.contains(banned),
                    "help for `{place}` leaks `{banned}`: {text}"
                );
            }
            assert!(
                !text.contains('\u{2014}') && !text.contains('\u{2013}'),
                "help for `{place}` has a dash: {text}"
            );
        }
    }

    #[test]
    fn json_and_hook_are_hidden_but_still_parse() {
        // Arrange
        let cmd = Cli::command();

        // Act
        let hidden: Vec<_> = cmd
            .get_subcommands()
            .filter(|c| c.is_hide_set())
            .map(|c| c.get_name().to_string())
            .collect();

        // Assert
        let mut hidden = hidden;
        hidden.sort();
        assert_eq!(hidden, ["hook", "json"]);
        assert!(Cli::command()
            .try_get_matches_from(["playbook", "json", "array-length"])
            .is_ok());
        assert!(Cli::command()
            .try_get_matches_from(["playbook", "hook", "session-init"])
            .is_ok());
    }
}
