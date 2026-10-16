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
//! This module writes nothing and runs nothing: it renders the files and
//! finds an installed one for `usage status` and `doctor`. The playbook
//! binary installs, starts and removes the service.

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

/// A path for the `append:` of `StandardOutput=`: systemd takes the rest of
/// the line as the path and expands only `%` specifiers, so only `%` is
/// doubled.
fn systemd_path(text: &str) -> String {
    text.replace('%', "%%")
}

/// A path is written into a service file as text: refuse one that is not
/// UTF-8 or holds a control character such as a line break, which would
/// end the line and start a new directive.
pub fn checked_text(what: &str, path: &Path) -> Result<(), String> {
    match path.to_str() {
        Some(text) if !text.chars().any(char::is_control) => Ok(()),
        _ => Err(format!(
            "the {what} path {} has characters a service file cannot hold",
            path.display()
        )),
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
            let log = systemd_path(&log);
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

/// Whether the service file under `home` is the one setup would write
/// for `program`.
pub fn is_current(kind: Kind, home: &Path, program: &Path) -> bool {
    let want = render(kind, program, &log_path(home));
    fs::read_to_string(kind.file(home)).is_ok_and(|now| now == want)
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

/// What to do when telemetry points at the collector and none runs, in one
/// line: start it by hand, and either check the installed service's log or
/// install the service.
pub fn collector_fix(home: &Path) -> String {
    let collect = crate::host::command("usage collect");
    match find(home) {
        Some(found) => format!(
            "start it with `{collect}`; the {} is installed but the collector is not up, see {}",
            found.kind.name(),
            log_path(home).display()
        ),
        None => format!(
            "start it with `{collect}`, or keep it running in the background with `{}`",
            crate::host::setup_command("collector-service")
        ),
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::util::scratch_dir;

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
        let unit = render(Kind::Systemd, odd, Path::new("/home/a b/$x/5%.log"));
        assert!(
            unit.contains("\"/Users/a b/100%% & <x>/$$HOME/playbook\""),
            "{unit}"
        );
        // The append path takes no quotes and no `$` doubling, only `%%`.
        assert!(
            unit.contains("StandardOutput=append:/home/a b/$x/5%%.log\n"),
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
    fn a_path_with_a_line_break_or_control_character_is_refused() {
        assert!(checked_text("program", Path::new("/bin/playbook")).is_ok());
        let err = checked_text("program", Path::new("/bin/a\nExecStartPre=/x")).unwrap_err();
        assert!(err.contains("cannot hold"), "{err}");
    }

    #[test]
    fn find_reports_the_file_and_whether_its_program_exists() {
        let home = scratch_dir("service-find");
        assert_eq!(find_kind(Kind::Launchd, &home), None);
        let program = home.join("bin/playbook");
        let file = Kind::Launchd.file(&home);
        fs::create_dir_all(file.parent().unwrap()).unwrap();
        fs::create_dir_all(program.parent().unwrap()).unwrap();
        fs::write(&file, render(Kind::Launchd, &program, &log_path(&home))).unwrap();

        let missing = find_kind(Kind::Launchd, &home).unwrap().program_exists();
        let current = is_current(Kind::Launchd, &home, &program);
        fs::write(&program, "").unwrap();
        let present = find_kind(Kind::Launchd, &home).unwrap();

        assert!(!missing);
        assert!(current);
        assert!(!is_current(Kind::Launchd, &home, Path::new(PROGRAM)));
        assert!(present.program_exists());
        assert!(
            present.describe().starts_with("launchd agent io.github"),
            "{}",
            present.describe()
        );
    }
}
