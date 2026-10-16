// SPDX-FileCopyrightText: 2026 Igor Santos
// SPDX-License-Identifier: Apache-2.0

//! Installs, starts and removes the background service for the usage
//! collector (ADR 0043). The files come from [`crate::usage::service`].
//!
//! Every call to `launchctl` or `systemctl` goes through [`Manager`]. Tests
//! pass a fake, and [`manager_for`] hands out the real one only when `home`
//! is the home of the account, so a run with a scratch `HOME` never loads a
//! real service.

use crate::usage::service::{checked_text, is_current, log_path, render, Kind, LABEL, UNIT};
use std::fs;
use std::path::{Path, PathBuf};

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

/// A manager that runs nothing: `HOME` is not the home of the account, so a
/// real service would run against other files.
pub struct NotThisHome;

impl Manager for NotThisHome {
    fn run(&self, program: &str, _: &[String]) -> Result<String, String> {
        Err(format!(
            "{program} not run, because HOME is not the home of your account"
        ))
    }
}

/// The real manager when `home` is the home of the account, else one that
/// runs nothing.
pub fn manager_for(home: &Path) -> Box<dyn Manager> {
    let same = account_home().is_some_and(|real| {
        fs::canonicalize(&real).unwrap_or(real) == fs::canonicalize(home).unwrap_or_default()
    });
    if same {
        Box::new(System)
    } else {
        Box::new(NotThisHome)
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

/// The command that tells whether the service runs.
fn check_command(kind: Kind, uid: u32) -> (&'static str, Vec<String>) {
    match kind {
        Kind::Launchd => (
            "launchctl",
            vec!["print".into(), format!("gui/{uid}/{LABEL}")],
        ),
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
            (
                "launchctl",
                vec![
                    "bootstrap".into(),
                    format!("gui/{uid}"),
                    file.to_string_lossy().into_owned(),
                ],
            ),
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
        Kind::Launchd => vec![(
            "launchctl",
            vec!["bootout".into(), format!("gui/{uid}/{LABEL}")],
        )],
        Kind::Systemd => vec![("systemctl", words(&["--user", "disable", "--now", UNIT]))],
    }
}

/// One word for a shell line, in single quotes when it needs them.
fn shell_word(word: &str) -> String {
    let plain = word
        .chars()
        .all(|c| c.is_ascii_alphanumeric() || "/._-:@%+=".contains(c));
    if plain && !word.is_empty() {
        word.to_string()
    } else {
        format!("'{}'", word.replace('\'', "'\\''"))
    }
}

/// The load commands as one line a person can paste into a shell.
pub fn load_hint(kind: Kind, file: &Path, uid: u32) -> String {
    load_commands(kind, file, uid)
        .into_iter()
        .map(|(program, args)| {
            std::iter::once(program.to_string())
                .chain(args.iter().map(|a| shell_word(a)))
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

/// Whether launchd still knows the agent, running or not.
fn known_to_launchd(manager: &dyn Manager, uid: u32) -> bool {
    let (program, args) = check_command(Kind::Launchd, uid);
    manager.run(program, &args).is_ok()
}

/// Waits up to two seconds for launchd to let go of a booted out agent, so
/// the next `bootstrap` does not fail while the old job is torn down.
fn wait_until_unloaded(manager: &dyn Manager, uid: u32) {
    for _ in 0..20 {
        if !known_to_launchd(manager, uid) {
            return;
        }
        std::thread::sleep(std::time::Duration::from_millis(100));
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
    if kind == Kind::Launchd && known_to_launchd(manager, uid) {
        // launchd keeps the old definition until the agent is booted out.
        let _ = run_all(manager, unload_commands(kind, uid));
        wait_until_unloaded(manager, uid);
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

#[cfg(test)]
mod tests {
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
}
