// SPDX-FileCopyrightText: 2026 Igor Santos
// SPDX-License-Identifier: Apache-2.0

//! PreToolUse guard for free disk space (ADR 0047 decision 3).
//!
//! Below `disk.blockFreeGb` it denies build commands and new agents and
//! leaves every other call open, so the model can still clean up. Below
//! `disk.warnFreeGb` it warns, at most once every 10 minutes per session,
//! and with `disk.autoCleanup` on starts `playbook disk clean --auto` in the
//! background, at most once per window and never while one runs. Below 1 GB
//! on the home volume it deletes the ballast file, which gives space back
//! with one `unlink`. It decides from one config read and a `statvfs` per
//! volume, and writes its throttle stamps only after it has decided, so it
//! still works when the disk is full.

use crate::common::emit::{emit_pre_context, emit_pre_deny};
use crate::common::payload::Payload;
use crate::disk::policy::{self, Level, Policy};
use crate::disk::{ballast, size::gb};
use std::path::{Path, PathBuf};

/// How long a warning or a background cleanup stays quiet in one session.
pub const QUIET_SECS: u64 = 10 * 60;
const WARNED_FILE: &str = "disk-warned";
const CLEANED_FILE: &str = "disk-cleaned";

/// What the hook does with one call.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Verdict {
    Allow,
    Warn,
    /// Deny, naming what was stopped: a build command or a new agent.
    Deny(String),
}

/// The tool a call uses, as far as this guard cares.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Tool<'a> {
    Bash(&'a str),
    Agent,
    Other,
}

/// The verdict for one call at `level`.
pub fn decide(level: Level, tool: Tool) -> Verdict {
    let stopped = match tool {
        Tool::Bash(command) => crate::disk::classify::build_command(command),
        Tool::Agent => Some("starting a new agent".to_string()),
        Tool::Other => None,
    };
    match (level, stopped) {
        (Level::Ok, _) => Verdict::Allow,
        (Level::Critical, Some(what)) => Verdict::Deny(what),
        _ => Verdict::Warn,
    }
}

/// What one measurement found, shared by the PreToolUse and SessionStart
/// paths.
struct Reading {
    home: PathBuf,
    policy: Policy,
    free: u64,
    level: Level,
    released: Option<u64>,
    cwd: Option<PathBuf>,
    session: Option<PathBuf>,
    now: u64,
}

/// Measures the tightest volume, deleting the ballast first when less than
/// its size is free. `None` when the guard is off or nothing can be read.
fn read(payload: &Payload) -> Option<Reading> {
    if std::env::var("DISK_GUARD").as_deref() == Ok("0") {
        return None;
    }
    let home = crate::common::home_dir();
    let cwd = PathBuf::from(payload.field(".cwd"));
    let cwd = cwd.is_absolute().then_some(cwd);
    let policy = policy::load(&home);
    let volumes = policy::volumes(&policy::measured_paths(&policy, &home, cwd.as_deref()));
    let tight = policy::tightest(&volumes)?.space;
    let mut free = tight.free;
    // The ballast gives space back only on its own volume.
    let on_ballast_volume =
        || crate::disk::probe::space(&ballast::dir(&home)).is_ok_and(|s| s.device == tight.device);
    let released = (free < ballast::BALLAST_BYTES && on_ballast_volume())
        .then(|| ballast::release(&home).ok().flatten())
        .flatten();
    if let Some(bytes) = released {
        free = free.saturating_add(bytes);
    }
    let level = policy.level(free);
    // The session folder holds the throttle stamps, which only a low disk
    // needs, so a call with enough space writes nothing.
    let session = if level == Level::Ok {
        String::new()
    } else {
        crate::common::session::session_dir(payload)
    };
    Some(Reading {
        home,
        level,
        policy,
        free,
        released,
        cwd,
        session: (!session.is_empty()).then(|| PathBuf::from(session)),
        now: u64::try_from(crate::common::time::now_secs()).unwrap_or(0),
    })
}

impl Reading {
    /// Starts a background cleanup when space is low, `disk.autoCleanup` is
    /// on and none started in this session in the last 10 minutes.
    fn maybe_clean(&self) -> bool {
        let started = self.level != Level::Ok
            && self.policy.auto_cleanup
            && quiet_over(self.session.as_deref(), CLEANED_FILE, self.now)
            && !cleanup_running(&self.home)
            && spawn_cleanup(self.cwd.as_deref());
        if started {
            stamp(self.session.as_deref(), CLEANED_FILE, self.now);
        }
        started
    }

    fn warning(&self) -> String {
        warning(self.free, self.level, &self.policy, self.released)
    }
}

pub fn run(payload: &Payload) {
    let Some(r) = read(payload) else {
        return;
    };
    let tool_name = payload.field(".tool_name");
    let command = payload.field(".tool_input.command");
    let tool = match tool_name.as_str() {
        "Bash" if !command.is_empty() => Tool::Bash(&command),
        "Agent" | "Task" => Tool::Agent,
        _ => Tool::Other,
    };
    match decide(r.level, tool) {
        Verdict::Allow => {}
        Verdict::Deny(what) => {
            let cleanup = r.maybe_clean();
            emit_pre_deny(&denial(&what, r.free, &r.policy, r.released, cleanup));
        }
        Verdict::Warn => {
            r.maybe_clean();
            if r.released.is_some() || quiet_over(r.session.as_deref(), WARNED_FILE, r.now) {
                emit_pre_context("PreToolUse", &r.warning());
                stamp(r.session.as_deref(), WARNED_FILE, r.now);
            }
        }
    }
}

/// Whether `settings.json` under `claude_home` wires the guard for Bash.
/// SessionStart and the status line act on the guard only then, so its side
/// effects and its "blocked" wording follow the user's consent.
pub fn wired(claude_home: &Path) -> bool {
    let Ok(text) = std::fs::read_to_string(claude_home.join("settings.json")) else {
        return false;
    };
    let Ok(doc) = serde_json::from_str::<serde_json::Value>(&text) else {
        return false;
    };
    let runs_guard = |group: &serde_json::Value| {
        group["hooks"].as_array().is_some_and(|hooks| {
            hooks
                .iter()
                .any(|h| h["command"].as_str() == Some("playbook hook disk-guard"))
        })
    };
    doc["hooks"]["PreToolUse"].as_array().is_some_and(|groups| {
        groups.iter().any(|g| {
            g["matcher"]
                .as_str()
                .is_some_and(|m| m.split('|').any(|tool| tool == "Bash"))
                && runs_guard(g)
        })
    })
}

/// The SessionStart line: the warning when space is below
/// `disk.warnFreeGb`, after starting a background cleanup when one is due
/// and the session is not headless. `None` when space is fine or the guard
/// is off.
pub fn session_start(payload: &Payload, headless: bool) -> Option<String> {
    let r = read(payload)?;
    if r.level == Level::Ok {
        return None;
    }
    if !headless {
        r.maybe_clean();
    }
    stamp(r.session.as_deref(), WARNED_FILE, r.now);
    Some(r.warning())
}

/// Whether the quiet window for `file` has passed. With no session folder
/// there is nowhere to remember, so it has.
fn quiet_over(session: Option<&Path>, file: &str, now: u64) -> bool {
    let Some(dir) = session else {
        return true;
    };
    std::fs::read_to_string(dir.join(file))
        .ok()
        .and_then(|t| t.trim().parse::<u64>().ok())
        .is_none_or(|last| now.saturating_sub(last) >= QUIET_SECS)
}

/// Records `now` for `file`. On a full disk the write fails, and the next
/// call warns again, which costs words and no space.
fn stamp(session: Option<&Path>, file: &str, now: u64) {
    if let Some(dir) = session {
        let _ = std::fs::write(dir.join(file), now.to_string());
    }
}

/// Whether a cleanup holds its lock now. This needs no free space, so it
/// keeps a full disk, where the throttle stamp cannot be written, from
/// starting one cleanup per tool call.
fn cleanup_running(home: &Path) -> bool {
    std::fs::File::open(crate::disk::clean::lock_path(home))
        .is_ok_and(|file| !crate::disk::clean::try_lock(&file))
}

/// Starts `playbook disk clean --auto` in the background, detached, with no
/// output. `disk clean` takes its own lock, so a second start exits at once.
fn spawn_cleanup(cwd: Option<&Path>) -> bool {
    // A test that injects free space must also point the cleanup at its own
    // scratch folder, or the cleanup would scan the machine's `/tmp`.
    if cfg!(debug_assertions)
        && std::env::var_os(crate::disk::probe::TEST_FREE_GB_VAR).is_some()
        && std::env::var_os(crate::disk::clean::TEST_TMP_VAR).is_none()
    {
        return false;
    }
    let Ok(exe) = std::env::current_exe() else {
        return false;
    };
    let mut cmd = std::process::Command::new(exe);
    cmd.args(["disk", "clean", "--auto"])
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null());
    if let Some(dir) = cwd {
        cmd.current_dir(dir);
    }
    #[cfg(unix)]
    {
        use std::os::unix::process::CommandExt;
        cmd.process_group(0);
    }
    cmd.spawn().is_ok()
}

fn ballast_note(released: Option<u64>) -> String {
    released.map_or_else(String::new, |bytes| {
        format!(
            " The disk guard deleted the ballast file to give back {} so a shell and a cleanup can run. After the cleanup, `playbook disk reserve` puts it back.",
            gb(bytes)
        )
    })
}

/// The denial the model reads.
pub fn denial(
    what: &str,
    free: u64,
    policy: &Policy,
    released: Option<u64>,
    cleanup_started: bool,
) -> String {
    let started = if cleanup_started {
        " A background `playbook disk clean` has started."
    } else {
        ""
    };
    format!(
        "Disk guard: {what} is blocked. Only {} is free, below disk.blockFreeGb ({} GB). Builds and new agents stop here so the disk keeps room for the shell and for cleanup.{}{started} Free space with commands that stay allowed: `playbook disk clean` (removes idle cargo target folders, old scratch and landed worktrees; add `--dry-run` to preview), `cargo clean` in a worktree you are done with, `git worktree remove` for a merged worktree, and `playbook disk status` to see what uses space. Then retry. The threshold is the user's to change: ask them to run `! playbook config set disk.blockFreeGb <gb>`.",
        gb(free),
        policy.block_free_gb,
        ballast_note(released)
    )
}

/// The warning added to the model's context.
pub fn warning(free: u64, level: Level, policy: &Policy, released: Option<u64>) -> String {
    let limit = if level == Level::Critical {
        format!(
            "below disk.blockFreeGb ({} GB): build commands and new agents are blocked",
            policy.block_free_gb
        )
    } else if policy.block_free_gb == 0 {
        format!(
            "below disk.warnFreeGb ({} GB). Blocking is off",
            policy.warn_free_gb
        )
    } else {
        format!(
            "below disk.warnFreeGb ({} GB). Builds stop below {} GB",
            policy.warn_free_gb, policy.block_free_gb
        )
    };
    format!(
        "Disk guard: {} free, {limit}.{} Run `playbook disk clean` to remove idle cargo target folders, old scratch and landed worktrees, and run `cargo clean` in worktrees you are done with.",
        gb(free),
        ballast_note(released)
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::disk::probe::GIB;

    const fn gib(n: u64) -> u64 {
        n * GIB
    }

    fn level(gb: f64) -> Level {
        Policy::default().level((gb * GIB as f64) as u64)
    }

    #[test]
    fn builds_are_denied_only_below_the_floor() {
        let build = Tool::Bash("cargo build");
        let cases = [
            (0.0, Verdict::Deny("cargo build".into())),
            (1.0, Verdict::Deny("cargo build".into())),
            (5.9, Verdict::Deny("cargo build".into())),
            (6.0, Verdict::Warn),
            (14.9, Verdict::Warn),
            (15.0, Verdict::Allow),
        ];
        for (gb, want) in cases {
            assert_eq!(decide(level(gb), build), want, "{gb} GB");
        }
    }

    #[test]
    fn agents_are_denied_below_the_floor() {
        assert_eq!(
            decide(level(5.9), Tool::Agent),
            Verdict::Deny("starting a new agent".into())
        );
        assert_eq!(decide(level(6.0), Tool::Agent), Verdict::Warn);
        assert_eq!(decide(level(15.0), Tool::Agent), Verdict::Allow);
    }

    #[test]
    fn recovery_commands_only_warn_even_at_zero() {
        for command in [
            "rm -rf /w/.worktrees/cargo-old",
            "cargo clean",
            "playbook disk clean",
            "df -g /",
            "du -sh .worktrees/*",
            "git status",
            "git worktree remove ../wt",
        ] {
            assert_eq!(
                decide(level(0.0), Tool::Bash(command)),
                Verdict::Warn,
                "{command}"
            );
        }
        assert_eq!(decide(level(0.0), Tool::Other), Verdict::Warn);
    }

    #[test]
    fn the_denial_names_free_space_threshold_cleanup_and_the_user() {
        let text = denial("cargo build", gib(5), &Policy::default(), None, true);
        for part in [
            "cargo build is blocked",
            "Only 5.0 GB is free",
            "disk.blockFreeGb (6 GB)",
            "`playbook disk clean`",
            "`cargo clean`",
            "`git worktree remove`",
            "background `playbook disk clean` has started",
            "! playbook config set disk.blockFreeGb <gb>",
        ] {
            assert!(text.contains(part), "missing {part:?} in {text}");
        }
        assert!(!text.contains('\u{2014}') && !text.contains('\u{2013}'));
    }

    #[test]
    fn messages_say_when_the_ballast_was_released() {
        let text = denial(
            "cargo test",
            gib(1),
            &Policy::default(),
            Some(gib(1)),
            false,
        );
        assert!(
            text.contains("deleted the ballast file to give back 1.0 GB"),
            "{text}"
        );
        assert!(text.contains("`playbook disk reserve`"), "{text}");
        assert!(!text.contains("has started"), "{text}");
        let warn = warning(gib(10), Level::Low, &Policy::default(), None);
        assert!(
            warn.contains("10.0 GB free, below disk.warnFreeGb (15 GB)"),
            "{warn}"
        );
        assert!(warn.contains("Builds stop below 6 GB"), "{warn}");
        let off = Policy {
            block_free_gb: 0,
            ..Policy::default()
        };
        let warn = warning(gib(10), Level::Low, &off, None);
        assert!(warn.contains("Blocking is off"), "{warn}");
        assert!(!warn.contains("Builds stop"), "{warn}");
    }

    #[test]
    fn the_quiet_window_holds_for_ten_minutes() {
        let dir =
            std::env::temp_dir().join(format!("pb-disk-guard-quiet-{}", crate::testing::run_id()));
        std::fs::create_dir_all(&dir).unwrap();
        assert!(quiet_over(Some(&dir), WARNED_FILE, 1_000));
        stamp(Some(&dir), WARNED_FILE, 1_000);
        assert!(!quiet_over(Some(&dir), WARNED_FILE, 1_000 + QUIET_SECS - 1));
        assert!(quiet_over(Some(&dir), WARNED_FILE, 1_000 + QUIET_SECS));
        assert!(quiet_over(None, WARNED_FILE, 0));
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
