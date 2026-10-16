// SPDX-FileCopyrightText: 2026 Igor Santos
// SPDX-License-Identifier: Apache-2.0

//! The per-user background service that keeps `playbook usage collect`
//! running (ADR 0043): a launchd agent on macOS, a systemd user unit on
//! Linux. Other systems get the command to run by hand.
//!
//! The service file holds only the program path, its arguments and the log
//! path. The bearer token stays in `collector.json`, which the collector
//! reads on start, so no secret is ever written into the service file.
//!
//! Every call to `launchctl` or `systemctl` goes through [`Manager`]. Tests
//! pass a fake, and [`manager_for`] hands out the real one only when `home`
//! is the home of the account, so a run with a scratch `HOME` never loads a
//! real service.

use std::fs;
use std::path::{Path, PathBuf};

/// The launchd label, also the file name of the agent.
pub const LABEL: &str = "io.github.pragmatic-engineer.playbook.collector";
/// The systemd user unit.
pub const UNIT: &str = "playbook-collector.service";
/// The log file, next to `collector.json`.
pub const LOG_FILE: &str = "collector-service.log";
/// The arguments after the program. The idle limit is off, so the service
/// never stops by itself in a quiet hour.
pub const ARGS: [&str; 4] = ["usage", "collect", "--idle-seconds", "0"];
/// Seconds before a stopped collector starts again. A start that finds a
/// collector running exits at once, so this also paces those starts.
const RESTART_SECONDS: u32 = 30;

/// The service managers playbook writes a file for.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    Launchd,
    Systemd,
}

impl Kind {
    /// The manager of this system, `None` when playbook writes no file here.
    pub fn current() -> Option<Kind> {
        if cfg!(target_os = "macos") {
            Some(Kind::Launchd)
        } else if cfg!(target_os = "linux") {
            Some(Kind::Systemd)
        } else {
            None
        }
    }

    /// What the service is, in words.
    pub fn name(self) -> &'static str {
        match self {
            Kind::Launchd => "launchd agent",
            Kind::Systemd => "systemd user unit",
        }
    }

    /// The label or unit name.
    pub fn id(self) -> &'static str {
        match self {
            Kind::Launchd => LABEL,
            Kind::Systemd => UNIT,
        }
    }

    /// Where the service file lives for the user at `home`.
    pub fn file(self, home: &Path) -> PathBuf {
        match self {
            Kind::Launchd => home
                .join("Library")
                .join("LaunchAgents")
                .join(format!("{LABEL}.plist")),
            Kind::Systemd => home.join(".config").join("systemd").join("user").join(UNIT),
        }
    }
}

/// The log file for the user at `home`, in the folder of `collector.json`.
pub fn log_path(home: &Path) -> PathBuf {
    crate::data::data_dir(home).join(LOG_FILE)
}

/// The command line a person runs when no service file can be written.
pub fn manual_command(program: &str) -> String {
    format!("{program} {}", ARGS.join(" "))
}

fn xml_escape(text: &str) -> String {
    text.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
}

/// One word of a systemd command line, quoted when it needs to be. `%` and
/// `$` are doubled, since systemd expands them.
fn systemd_word(text: &str) -> String {
    let escaped = text
        .replace('\\', "\\\\")
        .replace('"', "\\\"")
        .replace('%', "%%")
        .replace('$', "$$");
    if escaped.chars().any(char::is_whitespace) || escaped != text {
        format!("\"{escaped}\"")
    } else {
        escaped
    }
}

/// The service file for `program`, writing its output to `log`.
pub fn render(kind: Kind, program: &Path, log: &Path) -> String {
    let program = program.to_string_lossy();
    let log = log.to_string_lossy();
    match kind {
        Kind::Launchd => {
            let args: String = std::iter::once(program.as_ref())
                .chain(ARGS)
                .map(|a| format!("\t\t<string>{}</string>\n", xml_escape(a)))
                .collect();
            let log = xml_escape(&log);
            format!(
                "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n\
                 <!DOCTYPE plist PUBLIC \"-//Apple//DTD PLIST 1.0//EN\" \"http://www.apple.com/DTDs/PropertyList-1.0.dtd\">\n\
                 <!-- The playbook usage collector. `playbook uninstall` removes this file. -->\n\
                 <plist version=\"1.0\">\n\
                 <dict>\n\
                 \t<key>Label</key>\n\
                 \t<string>{LABEL}</string>\n\
                 \t<key>ProgramArguments</key>\n\
                 \t<array>\n\
                 {args}\
                 \t</array>\n\
                 \t<key>RunAtLoad</key>\n\
                 \t<true/>\n\
                 \t<key>KeepAlive</key>\n\
                 \t<true/>\n\
                 \t<key>ThrottleInterval</key>\n\
                 \t<integer>{RESTART_SECONDS}</integer>\n\
                 \t<key>ProcessType</key>\n\
                 \t<string>Background</string>\n\
                 \t<key>StandardOutPath</key>\n\
                 \t<string>{log}</string>\n\
                 \t<key>StandardErrorPath</key>\n\
                 \t<string>{log}</string>\n\
                 </dict>\n\
                 </plist>\n"
            )
        }
        Kind::Systemd => {
            let exec: Vec<String> = std::iter::once(program.as_ref())
                .chain(ARGS)
                .map(systemd_word)
                .collect();
            let log = systemd_word(&log);
            format!(
                "# The playbook usage collector. `playbook uninstall` removes this file.\n\
                 [Unit]\n\
                 Description=playbook usage collector\n\
                 \n\
                 [Service]\n\
                 ExecStart={}\n\
                 Restart=always\n\
                 RestartSec={RESTART_SECONDS}\n\
                 StandardOutput=append:{log}\n\
                 StandardError=append:{log}\n\
                 \n\
                 [Install]\n\
                 WantedBy=default.target\n",
                exec.join(" ")
            )
        }
    }
}

/// The program a service file starts, read back from a file [`render`] wrote.
pub fn program_in(kind: Kind, contents: &str) -> Option<PathBuf> {
    let raw = match kind {
        Kind::Launchd => {
            let after = contents.split("<key>ProgramArguments</key>").nth(1)?;
            let start = after.find("<string>")? + "<string>".len();
            let end = after[start..].find("</string>")? + start;
            after[start..end]
                .replace("&quot;", "\"")
                .replace("&gt;", ">")
                .replace("&lt;", "<")
                .replace("&amp;", "&")
        }
        Kind::Systemd => {
            let line = contents
                .lines()
                .find_map(|l| l.strip_prefix("ExecStart="))?;
            match line.strip_prefix('"') {
                Some(rest) => rest
                    .split_once("\" ")
                    .map(|(p, _)| p)?
                    .replace("\\\"", "\"")
                    .replace("\\\\", "\\")
                    .replace("%%", "%")
                    .replace("$$", "$"),
                None => line.split(' ').next()?.to_string(),
            }
        }
    };
    Some(PathBuf::from(raw))
}

/// Runs the service manager's command line tool.
pub trait Manager {
    /// Runs `program` with `args`. `Err` holds what it printed on a non-zero
    /// exit, or why it could not run.
    fn run(&self, program: &str, args: &[String]) -> Result<(), String>;
}

/// The real tools on this machine.
pub struct System;

impl Manager for System {
    fn run(&self, program: &str, args: &[String]) -> Result<(), String> {
        let out = std::process::Command::new(program)
            .args(args)
            .stdin(std::process::Stdio::null())
            .output()
            .map_err(|e| format!("could not run {program}: {e}"))?;
        if out.status.success() {
            return Ok(());
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
    fn run(&self, program: &str, _: &[String]) -> Result<(), String> {
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

/// The load commands as one line a person can paste.
pub fn load_hint(kind: Kind, file: &Path, uid: u32) -> String {
    load_commands(kind, file, uid)
        .into_iter()
        .map(|(program, args)| format!("{program} {}", args.join(" ")))
        .collect::<Vec<_>>()
        .join(" && ")
}

/// Whether the service manager runs the service now.
pub fn loaded(kind: Kind, manager: &dyn Manager, uid: u32) -> bool {
    let (program, args) = check_command(kind, uid);
    manager.run(program, &args).is_ok()
}

/// What [`install`] did.
#[derive(Debug, PartialEq, Eq)]
pub enum Installed {
    /// The file was already right and the service runs.
    Unchanged,
    /// The file was already right but the service was not running, so it
    /// was started. `Err` holds why it did not start.
    Started(Result<(), String>),
    /// The file was written. `started` is `Err` with the reason when the
    /// service manager did not start it.
    Wrote { started: Result<(), String> },
}

/// Writes the service file for `program` under `home` and starts it.
/// Idempotent: a file that matches is never written again, and a service
/// that runs is left alone. `Err` means the file could not be written.
pub fn install(
    kind: Kind,
    home: &Path,
    program: &Path,
    manager: &dyn Manager,
    uid: u32,
) -> Result<Installed, String> {
    let file = kind.file(home);
    let log = log_path(home);
    let contents = render(kind, program, &log);
    let running = loaded(kind, manager, uid);
    let start = || {
        load_commands(kind, &file, uid)
            .into_iter()
            .try_for_each(|(program, args)| manager.run(program, &args))
    };
    if fs::read_to_string(&file).is_ok_and(|now| now == contents) {
        return Ok(if running {
            Installed::Unchanged
        } else {
            Installed::Started(start())
        });
    }
    prepare_log(&log)?;
    write_file(&file, &contents)?;
    if running && kind == Kind::Launchd {
        // launchd keeps the old definition until the agent is booted out.
        for (program, args) in unload_commands(kind, uid) {
            let _ = manager.run(program, &args);
        }
    }
    Ok(Installed::Wrote { started: start() })
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

/// The service file playbook wrote, when there is one, for `usage status`
/// and `doctor`. It reads only the file and runs nothing.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Found {
    pub kind: Kind,
    pub file: PathBuf,
    /// The program the file starts, when it can be read back.
    pub program: Option<PathBuf>,
}

impl Found {
    /// Whether the program the file starts still exists.
    pub fn program_exists(&self) -> bool {
        self.program.as_deref().is_some_and(Path::is_file)
    }

    /// One line that names the service.
    pub fn describe(&self) -> String {
        format!(
            "{} {} ({})",
            self.kind.name(),
            self.kind.id(),
            self.file.display()
        )
    }
}

/// The service file under `home` for this system, if one exists.
pub fn find(home: &Path) -> Option<Found> {
    let kind = Kind::current()?;
    find_kind(kind, home)
}

/// [`find`] for a given manager, so both can be tested on any system.
pub fn find_kind(kind: Kind, home: &Path) -> Option<Found> {
    let file = kind.file(home);
    let contents = fs::read_to_string(&file).ok()?;
    Some(Found {
        kind,
        program: program_in(kind, &contents),
        file,
    })
}

/// Creates the log owner-only, or tightens an older one, so the service
/// manager appends to a private file.
fn prepare_log(log: &Path) -> Result<(), String> {
    let error = |e: std::io::Error| format!("could not prepare {}: {e}", log.display());
    if let Some(dir) = log.parent() {
        fs::create_dir_all(dir).map_err(error)?;
    }
    let mut options = fs::OpenOptions::new();
    options.append(true).create(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    options.open(log).map_err(error)?;
    set_mode(log, 0o600).map_err(error)
}

/// Writes the service file through a temp file and a rename, readable by
/// all and writable by the owner only, as both managers expect.
fn write_file(file: &Path, contents: &str) -> Result<(), String> {
    let error = |e: std::io::Error| format!("could not write {}: {e}", file.display());
    if let Some(dir) = file.parent() {
        fs::create_dir_all(dir).map_err(error)?;
    }
    crate::util::atomic::write_atomic_private(file, contents).map_err(error)?;
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
    use crate::util::scratch_dir;
    use std::cell::RefCell;

    /// Records each call; `running` answers the check command.
    #[derive(Default)]
    struct Fake {
        calls: RefCell<Vec<String>>,
        running: bool,
        fail: Option<&'static str>,
    }

    impl Manager for Fake {
        fn run(&self, program: &str, args: &[String]) -> Result<(), String> {
            let line = format!("{program} {}", args.join(" "));
            self.calls.borrow_mut().push(line.clone());
            if line.contains("print") || line.contains("is-active") {
                return if self.running {
                    Ok(())
                } else {
                    Err("not loaded".into())
                };
            }
            match self.fail {
                Some(word) if line.contains(word) => Err(format!("{word} failed")),
                _ => Ok(()),
            }
        }
    }

    impl Fake {
        fn calls(&self) -> Vec<String> {
            self.calls.borrow().clone()
        }
    }

    const PROGRAM: &str = "/opt/homebrew/bin/playbook";

    #[test]
    fn the_plist_runs_the_collector_at_load_keeps_it_alive_and_logs_to_the_state_dir() {
        let plist = render(Kind::Launchd, Path::new(PROGRAM), Path::new("/h/log"));

        for want in [
            "<string>io.github.pragmatic-engineer.playbook.collector</string>",
            "<string>/opt/homebrew/bin/playbook</string>\n\t\t<string>usage</string>\n\t\t<string>collect</string>\n\t\t<string>--idle-seconds</string>\n\t\t<string>0</string>",
            "<key>RunAtLoad</key>\n\t<true/>",
            "<key>KeepAlive</key>\n\t<true/>",
            "<key>StandardOutPath</key>\n\t<string>/h/log</string>",
            "<key>StandardErrorPath</key>\n\t<string>/h/log</string>",
        ] {
            assert!(plist.contains(want), "missing {want}\n{plist}");
        }
        assert_eq!(
            program_in(Kind::Launchd, &plist).unwrap(),
            Path::new(PROGRAM)
        );
    }

    #[test]
    fn the_unit_restarts_the_collector_and_appends_to_the_log() {
        let unit = render(Kind::Systemd, Path::new(PROGRAM), Path::new("/h/log"));

        assert!(
            unit.contains("ExecStart=/opt/homebrew/bin/playbook usage collect --idle-seconds 0\n"),
            "{unit}"
        );
        assert!(unit.contains("Restart=always\n"), "{unit}");
        assert!(unit.contains("StandardError=append:/h/log\n"), "{unit}");
        assert!(unit.contains("WantedBy=default.target"), "{unit}");
        assert_eq!(
            program_in(Kind::Systemd, &unit).unwrap(),
            Path::new(PROGRAM)
        );
    }

    #[test]
    fn odd_characters_in_a_path_are_escaped_and_read_back() {
        let odd = Path::new("/Users/a b/100% & <x>/$HOME/playbook");

        for kind in [Kind::Launchd, Kind::Systemd] {
            let text = render(kind, odd, Path::new("/l"));
            assert_eq!(program_in(kind, &text).unwrap(), odd, "{text}");
        }
        let unit = render(Kind::Systemd, odd, Path::new("/l"));
        assert!(
            unit.contains("\"/Users/a b/100%% & <x>/$$HOME/playbook\""),
            "{unit}"
        );
    }

    #[test]
    fn no_token_or_header_is_written_into_the_service_file() {
        for kind in [Kind::Launchd, Kind::Systemd] {
            let text = render(kind, Path::new(PROGRAM), Path::new("/l")).to_lowercase();
            for word in ["token", "authorization", "bearer", "otel_", "header"] {
                assert!(!text.contains(word), "{word} in {text}");
            }
        }
    }

    #[test]
    fn install_writes_the_file_and_the_log_with_their_modes_and_starts_it() {
        // Arrange
        let home = scratch_dir("service-install");
        let fake = Fake::default();

        // Act
        let done = install(Kind::Launchd, &home, Path::new(PROGRAM), &fake, 501).unwrap();

        // Assert
        assert_eq!(done, Installed::Wrote { started: Ok(()) });
        let file = Kind::Launchd.file(&home);
        assert!(file.starts_with(home.join("Library/LaunchAgents")));
        assert_eq!(
            fake.calls(),
            [
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
        install(
            Kind::Systemd,
            &home,
            Path::new(PROGRAM),
            &Fake::default(),
            1,
        )
        .unwrap();
        let running = Fake {
            running: true,
            ..Fake::default()
        };

        let done = install(Kind::Systemd, &home, Path::new(PROGRAM), &running, 1).unwrap();

        assert_eq!(done, Installed::Unchanged);
        assert_eq!(
            running.calls(),
            [format!("systemctl --user is-active --quiet {UNIT}")]
        );
    }

    #[test]
    fn a_matching_file_with_the_service_stopped_is_started_and_not_written() {
        let home = scratch_dir("service-stopped");
        install(
            Kind::Launchd,
            &home,
            Path::new(PROGRAM),
            &Fake::default(),
            1,
        )
        .unwrap();
        let file = Kind::Launchd.file(&home);
        let before = fs::metadata(&file).unwrap().modified().unwrap();
        let fake = Fake::default();

        let done = install(Kind::Launchd, &home, Path::new(PROGRAM), &fake, 1).unwrap();

        assert_eq!(done, Installed::Started(Ok(())));
        assert_eq!(fs::metadata(&file).unwrap().modified().unwrap(), before);
        assert!(fake.calls().iter().any(|c| c.contains("bootstrap")));
    }

    #[test]
    fn a_changed_program_reloads_a_running_agent() {
        let home = scratch_dir("service-moved");
        install(
            Kind::Launchd,
            &home,
            Path::new("/old/playbook"),
            &Fake::default(),
            7,
        )
        .unwrap();
        let running = Fake {
            running: true,
            ..Fake::default()
        };

        install(Kind::Launchd, &home, Path::new(PROGRAM), &running, 7).unwrap();

        assert!(running
            .calls()
            .contains(&format!("launchctl bootout gui/7/{LABEL}")));
        assert_eq!(
            find_kind(Kind::Launchd, &home).unwrap().program.unwrap(),
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

        let done = install(Kind::Launchd, &home, Path::new(PROGRAM), &fake, 1).unwrap();

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
        install(
            Kind::Systemd,
            &home,
            Path::new(PROGRAM),
            &Fake::default(),
            1,
        )
        .unwrap();
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

        let err = manager_for(&home).run("launchctl", &[]).unwrap_err();

        assert!(err.contains("not the home of your account"), "{err}");
    }

    #[test]
    fn find_reports_the_file_and_whether_its_program_exists() {
        let home = scratch_dir("service-find");
        assert_eq!(find_kind(Kind::Launchd, &home), None);
        let program = home.join("bin/playbook");
        fs::create_dir_all(program.parent().unwrap()).unwrap();
        install(Kind::Launchd, &home, &program, &Fake::default(), 1).unwrap();

        let missing = find_kind(Kind::Launchd, &home).unwrap().program_exists();
        fs::write(&program, "").unwrap();
        let present = find_kind(Kind::Launchd, &home).unwrap();

        assert!(!missing);
        assert!(present.program_exists());
        assert!(
            present.describe().starts_with("launchd agent io.github"),
            "{}",
            present.describe()
        );
    }

    #[test]
    fn the_load_hint_is_the_commands_a_person_runs() {
        let hint = load_hint(Kind::Systemd, Path::new("/x"), 1);
        assert_eq!(
            hint,
            format!("systemctl --user daemon-reload && systemctl --user enable {UNIT} && systemctl --user restart {UNIT}")
        );
    }
}
