// SPDX-FileCopyrightText: 2026 Igor Santos
// SPDX-License-Identifier: Apache-2.0

//! Installs, starts and removes the background service for the usage
//! collector (ADR 0043). The files come from [`crate::usage::service`].
//!
//! Every call to `launchctl` or `systemctl` goes through [`Manager`]. Tests
//! pass a fake, and [`manager_for`] hands out the real one only when `home`
//! is the home of the account, so a run with a scratch `HOME` never loads a
//! real service.

use crate::usage::service::{
    checked_text, find_old, is_current, log_path, render, Kind, LABEL, OLD_LABEL, UNIT,
};
use std::fs;
use std::path::{Path, PathBuf};
use std::time::Duration;

/// Runs the service manager's command line tool.
pub trait Manager {
    /// Runs `program` with `args` and returns what it printed. `Err` holds
    /// what it printed on a non-zero exit, or why it could not run.
    fn run(&self, program: &str, args: &[String]) -> Result<String, String>;
}

/// The real tools on this machine.
pub struct System;

impl Manager for System {
    fn run(&self, program: &str, args: &[String]) -> Result<String, String> {
        let out = std::process::Command::new(program)
            .args(args)
            .stdin(std::process::Stdio::null())
            .output()
            .map_err(|e| format!("could not run {program}: {e}"))?;
        if out.status.success() {
            return Ok(String::from_utf8_lossy(&out.stdout).into_owned());
        }
        let text = String::from_utf8_lossy(&out.stderr);
        let text = text.trim();
        Err(if text.is_empty() {
            format!("{program} {} exited with {}", args.join(" "), out.status)
        } else {
            format!("{program} {}: {text}", args.join(" "))
        })
    }
}

/// A manager that runs nothing and says why: `HOME` is not the home of the
/// account, so a real service would run against other files, or this is a
/// debug build that cargo started (`common::real_home`).
pub struct Refuses(pub &'static str);

impl Manager for Refuses {
    fn run(&self, program: &str, _: &[String]) -> Result<String, String> {
        Err(format!("{program} not run, because {}", self.0))
    }
}

/// Why the real tools may not run for `home`, or `None` when they may.
pub fn refusal(home: &Path) -> Option<&'static str> {
    use crate::common::real_home;
    if !real_home::is_account_home(home) {
        Some("HOME is not the home of your account")
    } else if real_home::refused(home) {
        Some(real_home::NOTE)
    } else {
        None
    }
}

/// The real manager when [`refusal`] allows it, else one that runs nothing.
pub fn manager_for(home: &Path) -> Box<dyn Manager> {
    match refusal(home) {
        None => Box::new(System),
        Some(why) => Box::new(Refuses(why)),
    }
}

/// The current user id.
#[cfg(unix)]
pub fn uid() -> u32 {
    // SAFETY: getuid has no preconditions and cannot fail.
    unsafe { libc::getuid() }
}

#[cfg(not(unix))]
pub fn uid() -> u32 {
    0
}

/// The home directory the account database lists for the current user.
#[cfg(unix)]
pub fn account_home() -> Option<PathBuf> {
    use std::ffi::CStr;
    use std::os::unix::ffi::OsStrExt;
    let mut buf = vec![0 as libc::c_char; 16 * 1024];
    // SAFETY: an all-zero passwd is a valid value to pass for getpwuid_r to fill.
    let mut pwd: libc::passwd = unsafe { std::mem::zeroed() };
    let mut result: *mut libc::passwd = std::ptr::null_mut();
    // SAFETY: every pointer is valid for the call and `buf.len()` is its size.
    let rc = unsafe { libc::getpwuid_r(uid(), &mut pwd, buf.as_mut_ptr(), buf.len(), &mut result) };
    if rc != 0 || result.is_null() || pwd.pw_dir.is_null() {
        return None;
    }
    // SAFETY: getpwuid_r succeeded, so pw_dir points at a NUL-terminated
    // string inside `buf`, which is still alive.
    let dir = unsafe { CStr::from_ptr(pwd.pw_dir) };
    Some(PathBuf::from(std::ffi::OsStr::from_bytes(dir.to_bytes())))
}

#[cfg(not(unix))]
pub fn account_home() -> Option<PathBuf> {
    None
}

fn words(list: &[&str]) -> Vec<String> {
    list.iter().map(|s| (*s).to_string()).collect()
}

/// The `launchctl print` command for the agent `label`.
fn print_command(label: &str, uid: u32) -> (&'static str, Vec<String>) {
    (
        "launchctl",
        vec!["print".into(), format!("gui/{uid}/{label}")],
    )
}

/// The `launchctl bootout` command for the agent `label`.
fn bootout_command(label: &str, uid: u32) -> (&'static str, Vec<String>) {
    (
        "launchctl",
        vec!["bootout".into(), format!("gui/{uid}/{label}")],
    )
}

/// The `launchctl bootstrap` command for the agent file `file`.
fn bootstrap_command(file: &Path, uid: u32) -> (&'static str, Vec<String>) {
    (
        "launchctl",
        vec![
            "bootstrap".into(),
            format!("gui/{uid}"),
            file.to_string_lossy().into_owned(),
        ],
    )
}

/// The command that tells whether the service runs.
fn check_command(kind: Kind, uid: u32) -> (&'static str, Vec<String>) {
    match kind {
        Kind::Launchd => print_command(LABEL, uid),
        Kind::Systemd => (
            "systemctl",
            words(&["--user", "is-active", "--quiet", UNIT]),
        ),
    }
}

/// The commands that start the service from `file`, in order.
pub fn load_commands(kind: Kind, file: &Path, uid: u32) -> Vec<(&'static str, Vec<String>)> {
    match kind {
        Kind::Launchd => vec![
            (
                "launchctl",
                vec!["enable".into(), format!("gui/{uid}/{LABEL}")],
            ),
            bootstrap_command(file, uid),
        ],
        Kind::Systemd => vec![
            ("systemctl", words(&["--user", "daemon-reload"])),
            ("systemctl", words(&["--user", "enable", UNIT])),
            ("systemctl", words(&["--user", "restart", UNIT])),
        ],
    }
}

/// The commands that restart a running service, so it runs the program
/// that is on disk now.
pub fn restart_commands(kind: Kind, uid: u32) -> Vec<(&'static str, Vec<String>)> {
    match kind {
        Kind::Launchd => vec![(
            "launchctl",
            vec![
                "kickstart".into(),
                "-k".into(),
                format!("gui/{uid}/{LABEL}"),
            ],
        )],
        Kind::Systemd => vec![("systemctl", words(&["--user", "restart", UNIT]))],
    }
}

/// The commands that stop the service and keep it from starting again.
pub fn unload_commands(kind: Kind, uid: u32) -> Vec<(&'static str, Vec<String>)> {
    match kind {
        Kind::Launchd => vec![bootout_command(LABEL, uid)],
        Kind::Systemd => vec![("systemctl", words(&["--user", "disable", "--now", UNIT]))],
    }
}

/// The load commands as one line a person can paste into a shell.
pub fn load_hint(kind: Kind, file: &Path, uid: u32) -> String {
    load_commands(kind, file, uid)
        .into_iter()
        .map(|(program, args)| {
            std::iter::once(program.to_string())
                .chain(args.iter().map(|a| crate::common::shell::word(a)))
                .collect::<Vec<_>>()
                .join(" ")
        })
        .collect::<Vec<_>>()
        .join(" && ")
}

/// Whether the service manager runs the service now. For launchd a loaded
/// agent is not enough: `launchctl print` must say `state = running`.
pub fn loaded(kind: Kind, manager: &dyn Manager, uid: u32) -> bool {
    let (program, args) = check_command(kind, uid);
    match (kind, manager.run(program, &args)) {
        (Kind::Launchd, Ok(out)) => out.contains("state = running"),
        (Kind::Systemd, Ok(_)) => true,
        (_, Err(_)) => false,
    }
}

/// Whether launchd still knows the agent `label`, running or not.
fn known_to_launchd(manager: &dyn Manager, label: &str, uid: u32) -> bool {
    let (program, args) = print_command(label, uid);
    manager.run(program, &args).is_ok()
}

/// How long to wait for launchd to let go of a booted out agent. Short in
/// unit tests, where the fake answers at once or never.
const UNLOAD_WAIT: Duration = if cfg!(test) {
    Duration::from_millis(200)
} else {
    Duration::from_secs(2)
};

/// Checks `done` every 100 ms until it holds or `wait` passes, and says
/// whether it held.
fn poll_until(wait: Duration, mut done: impl FnMut() -> bool) -> bool {
    let deadline = std::time::Instant::now() + wait;
    loop {
        if done() {
            return true;
        }
        if std::time::Instant::now() >= deadline {
            return false;
        }
        std::thread::sleep(Duration::from_millis(100));
    }
}

/// Waits for launchd to let go of the booted out agent `label`, so the next
/// `bootstrap` does not fail while the old job is torn down.
fn wait_until_unloaded(manager: &dyn Manager, label: &str, uid: u32) {
    poll_until(UNLOAD_WAIT, || !known_to_launchd(manager, label, uid));
}

/// `Err` with the note when this build may not change the real home
/// (`common::real_home`), so files there change only where launchctl may
/// run.
fn off_limits(home: &Path) -> Result<(), String> {
    match crate::common::real_home::refused(home) {
        true => Err(crate::common::real_home::NOTE.to_string()),
        false => Ok(()),
    }
}

/// What [`install`] did.
#[derive(Debug, PartialEq, Eq)]
pub enum Installed {
    /// The file was already right and the service runs.
    Unchanged,
    /// The file was already right and the service ran an older program, so
    /// it was restarted. `Err` holds why the restart failed.
    Restarted(Result<(), String>),
    /// The file was already right but the service was not running, so it
    /// was started. `Err` holds why it did not start.
    Started(Result<(), String>),
    /// The file was written. `started` is `Err` with the reason when the
    /// service manager did not start it.
    Wrote { started: Result<(), String> },
}

impl Installed {
    /// `Ok` when the service runs after the install, else why it does not.
    pub fn outcome(self) -> Result<(), String> {
        match self {
            Installed::Unchanged => Ok(()),
            Installed::Restarted(result)
            | Installed::Started(result)
            | Installed::Wrote { started: result } => result,
        }
    }
}

/// Runs `commands` in order and stops at the first failure.
fn run_all(
    manager: &dyn Manager,
    commands: Vec<(&'static str, Vec<String>)>,
) -> Result<(), String> {
    commands
        .into_iter()
        .try_for_each(|(program, args)| manager.run(program, &args).map(|_| ()))
}

/// Writes the service file for `program` under `home` and starts it.
/// Idempotent: a file that matches is never written again, and a service
/// that runs is left alone unless `outdated` says it runs an older program,
/// which restarts it. `Err` means the file could not be written.
pub fn install(
    kind: Kind,
    home: &Path,
    program: &Path,
    manager: &dyn Manager,
    uid: u32,
    outdated: bool,
) -> Result<Installed, String> {
    off_limits(home)?;
    let file = kind.file(home);
    let log = log_path(home);
    checked_text("program", program)?;
    checked_text("log", &log)?;
    let running = loaded(kind, manager, uid);
    if is_current(kind, home, program) {
        return Ok(match (running, outdated) {
            (true, false) => Installed::Unchanged,
            (true, true) => Installed::Restarted(run_all(manager, restart_commands(kind, uid))),
            (false, _) => Installed::Started(run_all(manager, load_commands(kind, &file, uid))),
        });
    }
    prepare_log(&log)?;
    write_file(&file, &render(kind, program, &log))?;
    if kind == Kind::Launchd && known_to_launchd(manager, LABEL, uid) {
        // launchd keeps the old definition until the agent is booted out.
        let _ = run_all(manager, unload_commands(kind, uid));
        wait_until_unloaded(manager, LABEL, uid);
    }
    Ok(Installed::Wrote {
        started: run_all(manager, load_commands(kind, &file, uid)),
    })
}

/// Stops the service and deletes its file. `Ok(false)` when there was no file.
pub fn remove(kind: Kind, home: &Path, manager: &dyn Manager, uid: u32) -> Result<bool, String> {
    let file = kind.file(home);
    if !file.exists() {
        return Ok(false);
    }
    off_limits(home)?;
    for (program, args) in unload_commands(kind, uid) {
        // Not loaded is fine: the file goes either way.
        let _ = manager.run(program, &args);
    }
    fs::remove_file(&file).map_err(|e| format!("could not remove {}: {e}", file.display()))?;
    if kind == Kind::Systemd {
        let _ = manager.run("systemctl", &words(&["--user", "daemon-reload"]));
    }
    Ok(true)
}

/// Stops the agent playbook wrote under [`OLD_LABEL`] and deletes its file.
/// `Ok(None)` when there is no such file, or it is not one playbook wrote,
/// which stays.
pub fn remove_old(home: &Path, manager: &dyn Manager, uid: u32) -> Result<Option<PathBuf>, String> {
    let Some(old) = find_old(home).filter(|o| o.ours) else {
        return Ok(None);
    };
    off_limits(home)?;
    let (program, args) = bootout_command(OLD_LABEL, uid);
    // Not loaded is fine: the file goes either way.
    let _ = manager.run(program, &args);
    fs::remove_file(&old.file)
        .map_err(|e| format!("could not remove {}: {e}", old.file.display()))?;
    Ok(Some(old.file))
}

/// What [`move_old_agent`] did.
#[derive(Debug, PartialEq, Eq)]
pub enum Moved {
    /// No agent file under [`OLD_LABEL`].
    Nothing,
    /// A file under [`OLD_LABEL`] that playbook did not write. It stays.
    NotOurs(PathBuf),
    /// The old agent is gone and the agent under [`LABEL`] is loaded. `up`
    /// says whether the collector ran within the wait.
    Done { up: bool },
}

/// Moves the agent playbook wrote under [`OLD_LABEL`] to [`LABEL`], with the
/// same program and log: boots out the old job, loads the new one, then
/// deletes the old file. The old job stops before the new one starts, and a
/// new agent that is loaded but not running is booted out first, so at most
/// one agent is loaded at any time. When the new agent does not start, it
/// is booted out, its file goes back to what it was, and the old agent is
/// loaded again if it was loaded before. `fallback` is the program to run
/// when the one the old file names is gone. `collector_answers` is checked
/// for up to `wait` after the move.
pub fn move_old_agent(
    home: &Path,
    fallback: &Path,
    manager: &dyn Manager,
    uid: u32,
    collector_answers: &dyn Fn() -> bool,
    wait: Duration,
) -> Result<Moved, String> {
    let Some(old) = find_old(home) else {
        return Ok(Moved::Nothing);
    };
    if !old.ours {
        return Ok(Moved::NotOurs(old.file));
    }
    off_limits(home)?;
    let program = old
        .program
        .filter(|p| p.is_file())
        .unwrap_or_else(|| fallback.to_path_buf());
    let was_loaded = stop(manager, OLD_LABEL, uid)?;
    if known_to_launchd(manager, LABEL, uid) && !loaded(Kind::Launchd, manager, uid) {
        stop(manager, LABEL, uid)?;
    }
    let new_file = Kind::Launchd.file(home);
    let before = fs::read(&new_file).ok();
    let started =
        install(Kind::Launchd, home, &program, manager, uid, false).and_then(Installed::outcome);
    if let Err(why) = started {
        let _ = stop(manager, LABEL, uid);
        let _ = match &before {
            Some(bytes) => fs::write(&new_file, bytes),
            None => fs::remove_file(&new_file),
        };
        let back = match was_loaded {
            false => "the old agent was not loaded before, so it stays stopped".to_string(),
            true => match manager.run("launchctl", &bootstrap_command(&old.file, uid).1) {
                Ok(_) => "the old agent runs again".to_string(),
                Err(e) => format!("the old agent did not start again either ({e})"),
            },
        };
        return Err(format!("the agent {LABEL} did not start ({why}); {back}"));
    }
    fs::remove_file(&old.file).map_err(|e| {
        format!(
            "the collector now runs as {LABEL}, but {} could not be removed ({e}), so launchd loads both at the next login",
            old.file.display()
        )
    })?;
    let up = poll_until(wait, || {
        loaded(Kind::Launchd, manager, uid) || collector_answers()
    });
    Ok(Moved::Done { up })
}

/// Boots out the agent `label` and waits for launchd to let go of it.
/// `Ok(true)` when it was loaded, `Ok(false)` when it was not, `Err` when it
/// is still loaded after the wait.
fn stop(manager: &dyn Manager, label: &str, uid: u32) -> Result<bool, String> {
    let was_loaded = known_to_launchd(manager, label, uid);
    if !was_loaded {
        return Ok(false);
    }
    let (program, args) = bootout_command(label, uid);
    let result = manager.run(program, &args);
    wait_until_unloaded(manager, label, uid);
    if known_to_launchd(manager, label, uid) {
        let why = result.err().unwrap_or_else(|| "it is still loaded".into());
        return Err(format!("could not stop the agent {label} ({why})"));
    }
    Ok(true)
}

/// Creates the log owner-only, or tightens an older one, so the service
/// manager appends to a private file.
fn prepare_log(log: &Path) -> Result<(), String> {
    if let Some(dir) = log.parent() {
        fs::create_dir_all(dir).map_err(|e| format!("could not prepare {}: {e}", log.display()))?;
    }
    crate::usage::util::atomic::make_db_file_private(log)
}

/// Writes the service file through a temp file and a rename, readable by
/// all and writable by the owner only, as both managers expect.
fn write_file(file: &Path, contents: &str) -> Result<(), String> {
    let error = |e: std::io::Error| format!("could not write {}: {e}", file.display());
    if let Some(dir) = file.parent() {
        fs::create_dir_all(dir).map_err(error)?;
    }
    crate::usage::util::atomic::write_atomic_private(file, contents).map_err(error)?;
    set_mode(file, 0o644).map_err(error)
}

#[cfg(unix)]
fn set_mode(path: &Path, mode: u32) -> std::io::Result<()> {
    use std::os::unix::fs::PermissionsExt;
    fs::set_permissions(path, fs::Permissions::from_mode(mode))
}

#[cfg(not(unix))]
fn set_mode(_: &Path, _: u32) -> std::io::Result<()> {
    Ok(())
}

/// A fake launchd for the tests of this module and of migration 0020.
#[cfg(test)]
pub(crate) mod fake {
    use super::{Manager, OLD_LABEL};
    use std::cell::RefCell;

    /// launchd as a set of loaded labels. `bootstrap` loads the label the
    /// file is named after and fails for one already loaded, as launchctl
    /// does. `bootout` unloads one, and `fail` makes any call naming that
    /// word fail. `stuck` keeps the old label loaded. A label in `waiting`
    /// is loaded but not running.
    #[derive(Default)]
    pub(crate) struct Launchd {
        pub(crate) loaded: RefCell<Vec<String>>,
        pub(crate) calls: RefCell<Vec<String>>,
        pub(crate) fail: Option<&'static str>,
        pub(crate) stuck: bool,
        pub(crate) waiting: Vec<&'static str>,
    }

    impl Launchd {
        pub(crate) fn with(labels: &[&str]) -> Launchd {
            Launchd {
                loaded: RefCell::new(labels.iter().map(|l| (*l).to_string()).collect()),
                ..Launchd::default()
            }
        }

        pub(crate) fn has(&self, label: &str) -> bool {
            self.loaded.borrow().iter().any(|l| l == label)
        }
    }

    impl Manager for Launchd {
        fn run(&self, program: &str, args: &[String]) -> Result<String, String> {
            let line = format!("{program} {}", args.join(" "));
            self.calls.borrow_mut().push(line.clone());
            let label = |arg: &str| arg.rsplit('/').next().unwrap_or_default().to_string();
            if let Some(word) = self.fail.filter(|w| line.contains(*w)) {
                return Err(format!("{word} failed"));
            }
            match args.first().map(String::as_str) {
                Some("print") if self.has(&label(&args[1])) => {
                    let gone = label(&args[1]);
                    Ok(match self.waiting.iter().any(|w| *w == gone) {
                        true => "\tstate = waiting\n".into(),
                        false => "\tstate = running\n".into(),
                    })
                }
                Some("print") => Err("Could not find service".into()),
                Some("bootout") if self.stuck && label(&args[1]) == OLD_LABEL => Ok(String::new()),
                Some("bootout") => {
                    let gone = label(&args[1]);
                    let had = self.has(&gone);
                    self.loaded.borrow_mut().retain(|l| *l != gone);
                    if had {
                        Ok(String::new())
                    } else {
                        Err("No such process".into())
                    }
                }
                Some("bootstrap") => {
                    let file = label(&args[2]);
                    let name = file.trim_end_matches(".plist").to_string();
                    if self.has(&name) {
                        return Err("Bootstrap failed: 5: Input/output error".into());
                    }
                    self.loaded.borrow_mut().push(name);
                    Ok(String::new())
                }
                _ => Ok(String::new()),
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::fake::Launchd;
    use super::*;
    use crate::common::test_support::scratch_dir;
    use std::cell::RefCell;

    /// Records each call. `running` answers the check command the way the
    /// real tool does, and `fail` makes any call naming that word fail.
    #[derive(Default)]
    struct Fake {
        calls: RefCell<Vec<String>>,
        booted_out: RefCell<bool>,
        running: bool,
        fail: Option<&'static str>,
    }

    impl Manager for Fake {
        fn run(&self, program: &str, args: &[String]) -> Result<String, String> {
            let line = format!("{program} {}", args.join(" "));
            self.calls.borrow_mut().push(line.clone());
            if line.contains("bootout") {
                *self.booted_out.borrow_mut() = true;
            }
            if line.contains(" print ") || line.contains("is-active") {
                return if self.running && !*self.booted_out.borrow() {
                    Ok("\tstate = running\n".into())
                } else {
                    Err("not loaded".into())
                };
            }
            match self.fail {
                Some(word) if line.contains(word) => Err(format!("{word} failed")),
                _ => Ok(String::new()),
            }
        }
    }

    impl Fake {
        fn calls(&self) -> Vec<String> {
            self.calls.borrow().clone()
        }
    }

    const PROGRAM: &str = "/opt/homebrew/bin/playbook";

    fn put(kind: Kind, home: &Path, program: &str, fake: &Fake) -> Installed {
        install(kind, home, Path::new(program), fake, 7, false).unwrap()
    }

    #[test]
    fn a_path_with_a_line_break_is_refused_and_nothing_is_written() {
        let home = scratch_dir("service-newline");

        let err = install(
            Kind::Systemd,
            &home,
            Path::new("/bin/playbook\nExecStartPre=/bin/evil"),
            &Fake::default(),
            1,
            false,
        )
        .unwrap_err();

        assert!(err.contains("cannot hold"), "{err}");
        assert!(!Kind::Systemd.file(&home).exists());
    }

    #[test]
    fn install_writes_the_file_and_the_log_with_their_modes_and_starts_it() {
        // Arrange
        let home = scratch_dir("service-install");
        let fake = Fake::default();

        // Act
        let done = install(Kind::Launchd, &home, Path::new(PROGRAM), &fake, 501, false).unwrap();

        // Assert
        assert_eq!(done, Installed::Wrote { started: Ok(()) });
        let file = Kind::Launchd.file(&home);
        assert!(file.starts_with(home.join("Library/LaunchAgents")));
        assert_eq!(
            fake.calls(),
            [
                format!("launchctl print gui/501/{LABEL}"),
                format!("launchctl print gui/501/{LABEL}"),
                format!("launchctl enable gui/501/{LABEL}"),
                format!("launchctl bootstrap gui/501 {}", file.display()),
            ]
        );
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = |p: &Path| fs::metadata(p).unwrap().permissions().mode() & 0o777;
            assert_eq!(mode(&file), 0o644);
            assert_eq!(mode(&log_path(&home)), 0o600);
        }
    }

    #[test]
    fn a_second_install_with_the_service_running_changes_nothing() {
        let home = scratch_dir("service-again");
        put(Kind::Systemd, &home, PROGRAM, &Fake::default());
        let running = Fake {
            running: true,
            ..Fake::default()
        };

        let done = put(Kind::Systemd, &home, PROGRAM, &running);

        assert_eq!(done, Installed::Unchanged);
        assert_eq!(
            running.calls(),
            [format!("systemctl --user is-active --quiet {UNIT}")]
        );
    }

    #[test]
    fn an_outdated_running_service_is_restarted_in_place() {
        let home = scratch_dir("service-outdated");
        put(Kind::Launchd, &home, PROGRAM, &Fake::default());
        let running = Fake {
            running: true,
            ..Fake::default()
        };

        let done = install(Kind::Launchd, &home, Path::new(PROGRAM), &running, 7, true).unwrap();

        assert_eq!(done, Installed::Restarted(Ok(())));
        assert!(running
            .calls()
            .contains(&format!("launchctl kickstart -k gui/7/{LABEL}")));
    }

    #[test]
    fn a_loaded_agent_that_is_not_running_counts_as_stopped() {
        struct Loaded;
        impl Manager for Loaded {
            fn run(&self, _: &str, _: &[String]) -> Result<String, String> {
                Ok("\tstate = waiting\n".into())
            }
        }

        assert!(!loaded(Kind::Launchd, &Loaded, 1));
        assert!(loaded(Kind::Systemd, &Loaded, 1));
    }

    #[test]
    fn a_matching_file_with_the_service_stopped_is_started_and_not_written() {
        let home = scratch_dir("service-stopped");
        put(Kind::Launchd, &home, PROGRAM, &Fake::default());
        let file = Kind::Launchd.file(&home);
        let before = fs::metadata(&file).unwrap().modified().unwrap();
        let fake = Fake::default();

        let done = put(Kind::Launchd, &home, PROGRAM, &fake);

        assert_eq!(done, Installed::Started(Ok(())));
        assert_eq!(fs::metadata(&file).unwrap().modified().unwrap(), before);
        assert!(fake.calls().iter().any(|c| c.contains("bootstrap")));
    }

    #[test]
    fn a_changed_program_boots_out_the_old_agent_before_loading_the_new_one() {
        // Arrange
        let home = scratch_dir("service-moved");
        put(Kind::Launchd, &home, "/old/playbook", &Fake::default());
        let running = Fake {
            running: true,
            ..Fake::default()
        };

        // Act
        let done = put(Kind::Launchd, &home, PROGRAM, &running);

        // Assert
        assert_eq!(done, Installed::Wrote { started: Ok(()) });
        let calls: Vec<String> = running
            .calls()
            .into_iter()
            .filter(|c| !c.contains(" print "))
            .collect();
        assert_eq!(
            calls,
            [
                format!("launchctl bootout gui/7/{LABEL}"),
                format!("launchctl enable gui/7/{LABEL}"),
                format!(
                    "launchctl bootstrap gui/7 {}",
                    Kind::Launchd.file(&home).display()
                ),
            ]
        );
        assert_eq!(
            crate::usage::service::find_kind(Kind::Launchd, &home)
                .unwrap()
                .program
                .unwrap(),
            Path::new(PROGRAM)
        );
    }

    #[test]
    fn a_start_that_fails_keeps_the_file_and_says_why() {
        let home = scratch_dir("service-fail");
        let fake = Fake {
            fail: Some("bootstrap"),
            ..Fake::default()
        };

        let done = put(Kind::Launchd, &home, PROGRAM, &fake);

        assert_eq!(
            done,
            Installed::Wrote {
                started: Err("bootstrap failed".into())
            }
        );
        assert!(Kind::Launchd.file(&home).is_file());
    }

    #[test]
    fn remove_stops_the_service_and_deletes_only_its_file() {
        let home = scratch_dir("service-remove");
        put(Kind::Systemd, &home, PROGRAM, &Fake::default());
        let fake = Fake::default();

        let removed = remove(Kind::Systemd, &home, &fake, 1).unwrap();
        let again = remove(Kind::Systemd, &home, &fake, 1).unwrap();

        assert!(removed && !again);
        assert!(!Kind::Systemd.file(&home).exists());
        assert!(log_path(&home).exists(), "the log stays");
        assert_eq!(
            fake.calls(),
            [
                format!("systemctl --user disable --now {UNIT}"),
                "systemctl --user daemon-reload".to_string(),
            ]
        );
    }

    #[test]
    fn a_scratch_home_never_gets_the_real_manager() {
        let home = scratch_dir("service-scratch");
        fs::create_dir_all(&home).unwrap();

        let err = manager_for(&home).run("launchctl", &[]).unwrap_err();

        assert!(err.contains("not the home of your account"), "{err}");
    }

    #[test]
    fn the_load_hint_is_the_commands_a_person_runs_with_odd_paths_quoted() {
        let hint = load_hint(Kind::Systemd, Path::new("/x"), 1);
        let agent = load_hint(Kind::Launchd, Path::new("/Users/a b/it's;x.plist"), 5);

        assert_eq!(
            hint,
            format!("systemctl --user daemon-reload && systemctl --user enable {UNIT} && systemctl --user restart {UNIT}")
        );
        assert!(
            agent.ends_with("launchctl bootstrap gui/5 '/Users/a b/it'\\''s;x.plist'"),
            "{agent}"
        );
    }

    /// A scratch home with the agent 0.37.0 wrote for `program`.
    fn old_home(tag: &str, program: &Path) -> PathBuf {
        let home = scratch_dir(tag);
        let file = crate::usage::service::old_file(&home);
        fs::create_dir_all(file.parent().unwrap()).unwrap();
        fs::write(
            &file,
            crate::usage::service::render_old(program, &log_path(&home)),
        )
        .unwrap();
        home
    }

    fn move_now(home: &Path, fake: &Launchd) -> Result<Moved, String> {
        move_old_agent(home, Path::new(PROGRAM), fake, 7, &|| false, Duration::ZERO)
    }

    #[test]
    fn the_old_agent_is_booted_out_before_the_new_one_loads_and_its_file_goes() {
        // Arrange
        let bin = scratch_dir("service-move-bin").join("playbook");
        fs::create_dir_all(bin.parent().unwrap()).unwrap();
        fs::write(&bin, "").unwrap();
        let home = old_home("service-move", &bin);
        let fake = Launchd::with(&[OLD_LABEL]);

        // Act
        let moved = move_now(&home, &fake);

        // Assert
        assert_eq!(moved, Ok(Moved::Done { up: true }));
        assert!(!crate::usage::service::old_file(&home).exists());
        assert!(
            fake.has(LABEL) && !fake.has(OLD_LABEL),
            "{:?}",
            fake.loaded.borrow()
        );
        let found = crate::usage::service::find_kind(Kind::Launchd, &home).unwrap();
        assert_eq!(
            found.program.as_deref(),
            Some(bin.as_path()),
            "same program as before"
        );
        let calls = fake.calls.borrow();
        let bootout = calls
            .iter()
            .position(|c| c.contains(&format!("bootout gui/7/{OLD_LABEL}")))
            .unwrap();
        let bootstrap = calls.iter().position(|c| c.contains("bootstrap")).unwrap();
        assert!(bootout < bootstrap, "{calls:?}");
    }

    #[test]
    fn a_second_move_finds_nothing_and_runs_nothing() {
        let home = old_home("service-move-again", Path::new("/gone/playbook"));
        let fake = Launchd::with(&[OLD_LABEL]);
        move_now(&home, &fake).unwrap();
        let again = Launchd::with(&[LABEL]);

        let moved = move_now(&home, &again);

        assert_eq!(moved, Ok(Moved::Nothing));
        assert!(again.calls.borrow().is_empty());
    }

    #[test]
    fn a_gone_program_moves_to_the_fallback_and_a_not_loaded_old_agent_is_fine() {
        let home = old_home("service-move-fallback", Path::new("/gone/playbook"));
        let fake = Launchd::default();

        let moved = move_now(&home, &fake);

        assert_eq!(moved, Ok(Moved::Done { up: true }));
        let found = crate::usage::service::find_kind(Kind::Launchd, &home).unwrap();
        assert_eq!(found.program.as_deref(), Some(Path::new(PROGRAM)));
    }

    #[test]
    fn a_file_playbook_did_not_write_is_left_alone() {
        let home = scratch_dir("service-move-foreign");
        let file = crate::usage::service::old_file(&home);
        fs::create_dir_all(file.parent().unwrap()).unwrap();
        fs::write(&file, "<plist>mine</plist>").unwrap();
        let fake = Launchd::with(&[OLD_LABEL]);

        let moved = move_now(&home, &fake);
        let removed = remove_old(&home, &fake, 7).unwrap();

        assert_eq!(moved, Ok(Moved::NotOurs(file.clone())));
        assert_eq!(removed, None);
        assert!(file.is_file());
        assert!(fake.calls.borrow().is_empty());
        assert!(!Kind::Launchd.file(&home).exists());
    }

    #[test]
    fn a_new_agent_that_does_not_start_is_rolled_back_and_the_old_one_loads_again() {
        // Arrange
        let home = old_home("service-move-rollback", Path::new(PROGRAM));
        let fake = Launchd {
            fail: Some(LABEL),
            ..Launchd::with(&[OLD_LABEL])
        };

        // Act
        let err = move_now(&home, &fake).unwrap_err();

        // Assert
        assert!(
            err.contains("did not start") && err.contains("the old agent runs again"),
            "{err}"
        );
        assert!(crate::usage::service::old_file(&home).is_file());
        assert!(!Kind::Launchd.file(&home).exists());
        assert!(fake.has(OLD_LABEL));
    }

    #[test]
    fn an_old_agent_that_will_not_stop_blocks_the_move() {
        let home = old_home("service-move-stuck", Path::new(PROGRAM));
        let fake = Launchd {
            stuck: true,
            ..Launchd::with(&[OLD_LABEL])
        };

        let err = move_now(&home, &fake).unwrap_err();

        assert!(err.contains("could not stop"), "{err}");
        assert!(crate::usage::service::old_file(&home).is_file());
        assert!(!Kind::Launchd.file(&home).exists());
        assert!(!fake.has(LABEL), "no second collector");
    }

    #[test]
    fn a_new_agent_that_loads_but_does_not_run_reports_it_is_down() {
        let home = old_home("service-move-down", Path::new(PROGRAM));
        let waits = || Launchd {
            waiting: vec![LABEL],
            ..Launchd::with(&[OLD_LABEL])
        };

        let moved = move_old_agent(
            &home,
            Path::new(PROGRAM),
            &waits(),
            7,
            &|| false,
            Duration::ZERO,
        );
        let answered = move_old_agent(
            &old_home("service-move-up", Path::new(PROGRAM)),
            Path::new(PROGRAM),
            &waits(),
            7,
            &|| true,
            Duration::ZERO,
        );

        assert_eq!(moved, Ok(Moved::Done { up: false }));
        assert_eq!(answered, Ok(Moved::Done { up: true }));
        assert!(!crate::usage::service::old_file(&home).exists());
    }

    #[test]
    fn a_loaded_new_agent_that_is_not_running_is_booted_out_first_so_one_agent_stays() {
        // Arrange: a half moved machine, both files on disk and both loaded.
        let home = old_home("service-move-half", Path::new(PROGRAM));
        put(Kind::Launchd, &home, PROGRAM, &Fake::default());
        let fake = Launchd {
            waiting: vec![LABEL],
            ..Launchd::with(&[OLD_LABEL, LABEL])
        };

        // Act
        let moved = move_now(&home, &fake);

        // Assert
        assert!(moved.is_ok(), "{moved:?}");
        assert_eq!(*fake.loaded.borrow(), vec![LABEL.to_string()]);
        assert!(!crate::usage::service::old_file(&home).exists());
    }

    #[test]
    fn a_failed_start_restores_a_new_file_that_was_there_and_loads_only_the_old_agent() {
        let home = old_home("service-move-restore", Path::new(PROGRAM));
        put(Kind::Launchd, &home, "/other/playbook", &Fake::default());
        let file = Kind::Launchd.file(&home);
        let before = fs::read(&file).unwrap();
        let fake = Launchd {
            fail: Some("enable"),
            ..Launchd::with(&[OLD_LABEL])
        };

        let err = move_now(&home, &fake).unwrap_err();

        assert!(err.contains("the old agent runs again"), "{err}");
        assert_eq!(fs::read(&file).unwrap(), before);
        assert_eq!(*fake.loaded.borrow(), vec![OLD_LABEL.to_string()]);
    }

    #[test]
    fn a_failed_start_leaves_an_old_agent_that_was_not_loaded_stopped() {
        let home = old_home("service-move-was-stopped", Path::new(PROGRAM));
        let fake = Launchd {
            fail: Some(LABEL),
            ..Launchd::default()
        };

        let err = move_now(&home, &fake).unwrap_err();

        assert!(err.contains("stays stopped"), "{err}");
        assert!(fake.loaded.borrow().is_empty());
        assert!(crate::usage::service::old_file(&home).is_file());
    }

    #[cfg(unix)]
    #[test]
    fn an_old_file_that_cannot_be_removed_is_reported_and_the_new_agent_keeps_running() {
        use std::os::unix::fs::PermissionsExt;
        let home = old_home("service-move-locked", Path::new(PROGRAM));
        let fake = Launchd::with(&[OLD_LABEL]);
        // The new file is written first, then the folder is locked.
        put(Kind::Launchd, &home, PROGRAM, &Fake::default());
        let dir = crate::usage::service::old_file(&home)
            .parent()
            .unwrap()
            .to_path_buf();
        fs::set_permissions(&dir, fs::Permissions::from_mode(0o555)).unwrap();

        let err = move_now(&home, &fake).unwrap_err();
        fs::set_permissions(&dir, fs::Permissions::from_mode(0o755)).unwrap();

        assert!(err.contains("could not be removed"), "{err}");
        assert!(fake.has(LABEL) && !fake.has(OLD_LABEL));
    }

    #[test]
    fn the_real_home_net_stops_every_file_change() {
        let home = old_home("service-move-net", Path::new(PROGRAM));
        let _pretend = crate::common::real_home::pretend_account_home(&home);
        let fake = Launchd::with(&[OLD_LABEL]);

        let moved = move_now(&home, &fake);
        let removed = remove_old(&home, &fake, 7);
        let installed = install(Kind::Launchd, &home, Path::new(PROGRAM), &fake, 7, false);

        if std::env::var_os(crate::common::real_home::ALLOW_ENV).is_none() {
            for err in [
                moved.unwrap_err(),
                removed.unwrap_err(),
                installed.unwrap_err(),
            ] {
                assert!(err.contains("PLAYBOOK_ALLOW_REAL_HOME"), "{err}");
            }
            assert!(crate::usage::service::old_file(&home).is_file());
            assert!(fake.calls.borrow().is_empty());
        }
    }

    #[test]
    fn remove_old_boots_out_and_deletes_only_the_old_file() {
        let home = old_home("service-remove-old", Path::new(PROGRAM));
        put(Kind::Launchd, &home, PROGRAM, &Fake::default());
        let fake = Launchd::with(&[OLD_LABEL, LABEL]);

        let removed = remove_old(&home, &fake, 7).unwrap();
        let again = remove_old(&home, &fake, 7).unwrap();

        assert_eq!(removed, Some(crate::usage::service::old_file(&home)));
        assert_eq!(again, None);
        assert!(!fake.has(OLD_LABEL));
        assert!(Kind::Launchd.file(&home).is_file(), "the new agent stays");
        assert!(fake.has(LABEL), "and stays loaded");
    }
}
