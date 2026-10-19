// SPDX-FileCopyrightText: 2026 Igor Santos
// SPDX-License-Identifier: Apache-2.0

//! The files of the retired per-user background service for `playbook
//! usage collect` (ADR 0043): a launchd agent on macOS, a systemd user unit
//! on Linux. Nothing installs it any more.
//!
//! This module writes nothing and runs nothing: it finds a file left by an
//! older setup, so `doctor`, `playbook uninstall` and the
//! `collector-service-removed` migration can name and remove it. It also
//! renders such a file, so tests can set up a machine that still has one.

use std::fs;
use std::path::{Path, PathBuf};

/// The launchd label as a literal, so other crates can build text with
/// `concat!`. Use [`LABEL`] everywhere else.
#[macro_export]
macro_rules! collector_label {
    () => {
        "com.github.pragmatic-engineer.playbook.collector"
    };
}

/// The launchd label, also the file name of the agent.
pub const LABEL: &str = collector_label!();
/// The launchd label that 0.37.0 shipped. The `collector-service-removed`
/// migration and `playbook uninstall` remove an agent installed under it.
pub const OLD_LABEL: &str = "io.github.pragmatic-engineer.playbook.collector";
/// The comment every service file playbook writes carries.
const MARKER: &str = "The playbook usage collector. `playbook uninstall` removes this file.";
/// The systemd user unit.
pub const UNIT: &str = "playbook-collector.service";
/// The log file, next to `collector.json`.
const LOG_FILE: &str = "collector-service.log";
/// The arguments after the program in a service file.
const ARGS: [&str; 4] = ["usage", "collect", "--idle-seconds", "0"];
/// Seconds before the service manager started a stopped collector again.
const RESTART_SECONDS: u32 = 30;

/// The service managers playbook wrote a file for.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    Launchd,
    Systemd,
}

impl Kind {
    /// The manager of this system, `None` when playbook wrote no file here.
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
            Kind::Launchd => agent_file(home, LABEL),
            Kind::Systemd => home.join(".config").join("systemd").join("user").join(UNIT),
        }
    }
}

/// The launchd agent file for `label` under `home`.
fn agent_file(home: &Path, label: &str) -> PathBuf {
    home.join("Library")
        .join("LaunchAgents")
        .join(format!("{label}.plist"))
}

/// The log file for the user at `home`, in the folder of `collector.json`.
pub fn log_path(home: &Path) -> PathBuf {
    crate::data::data_dir(home).join(LOG_FILE)
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

/// The service file older setups wrote for `program`, writing its output
/// to `log`.
pub fn render(kind: Kind, program: &Path, log: &Path) -> String {
    render_labeled(kind, LABEL, program, log)
}

/// [`render`] with the launchd label `label`. The systemd unit has none.
fn render_labeled(kind: Kind, label: &str, program: &Path, log: &Path) -> String {
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
                 <!-- {MARKER} -->\n\
                 <plist version=\"1.0\">\n\
                 <dict>\n\
                 \t<key>Label</key>\n\
                 \t<string>{label}</string>\n\
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
                "# {MARKER}\n\
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

/// The service file playbook wrote, when there is one. It reads only the
/// file and runs nothing.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Found {
    pub kind: Kind,
    pub file: PathBuf,
}

impl Found {
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
    file.is_file().then_some(Found { kind, file })
}

/// A launchd agent file under [`OLD_LABEL`], found by [`find_old`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OldAgent {
    pub file: PathBuf,
    /// Whether playbook wrote the file: it carries playbook's comment and
    /// the old label. Only such a file is ever removed.
    pub ours: bool,
}

impl OldAgent {
    /// One line that names the old agent.
    pub fn describe(&self) -> String {
        format!("launchd agent {OLD_LABEL} ({})", self.file.display())
    }
}

/// The agent file 0.37.0 wrote, under `home`.
pub fn old_file(home: &Path) -> PathBuf {
    agent_file(home, OLD_LABEL)
}

/// The agent file under [`OLD_LABEL`] in `home`, if one exists. It reads
/// only the file and runs nothing, on any system.
pub fn find_old(home: &Path) -> Option<OldAgent> {
    let file = old_file(home);
    let contents = fs::read_to_string(&file).ok()?;
    let ours = contents.contains(&format!("<!-- {MARKER} -->"))
        && contents.contains(&format!("<key>Label</key>\n\t<string>{OLD_LABEL}</string>"));
    Some(OldAgent { ours, file })
}

/// The agent file 0.37.0 wrote for `program`, so tests in other crates
/// can set up a machine that still has it.
pub fn render_old(program: &Path, log: &Path) -> String {
    render_labeled(Kind::Launchd, OLD_LABEL, program, log)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::util::scratch_dir;

    const PROGRAM: &str = "/opt/homebrew/bin/playbook";

    #[test]
    fn find_reports_the_file_and_names_it() {
        let home = scratch_dir("service-find");
        assert_eq!(find_kind(Kind::Launchd, &home), None);
        let file = Kind::Launchd.file(&home);
        fs::create_dir_all(file.parent().unwrap()).unwrap();
        fs::write(
            &file,
            render(Kind::Launchd, Path::new(PROGRAM), &log_path(&home)),
        )
        .unwrap();

        let found = find_kind(Kind::Launchd, &home).unwrap();

        assert_eq!(found.file, file);
        assert!(
            found
                .describe()
                .starts_with("launchd agent com.github.pragmatic-engineer.playbook.collector ("),
            "{}",
            found.describe()
        );
    }

    /// The agent file 0.37.0 wrote, byte for byte, so the match does not
    /// depend on today's renderer.
    const OLD_PLIST: &str = "<?xml version=\"1.0\" encoding=\"UTF-8\"?>
<!DOCTYPE plist PUBLIC \"-//Apple//DTD PLIST 1.0//EN\" \"http://www.apple.com/DTDs/PropertyList-1.0.dtd\">
<!-- The playbook usage collector. `playbook uninstall` removes this file. -->
<plist version=\"1.0\">
<dict>
\t<key>Label</key>
\t<string>io.github.pragmatic-engineer.playbook.collector</string>
\t<key>ProgramArguments</key>
\t<array>
\t\t<string>/opt/homebrew/bin/playbook</string>
\t\t<string>usage</string>
\t\t<string>collect</string>
\t\t<string>--idle-seconds</string>
\t\t<string>0</string>
\t</array>
\t<key>RunAtLoad</key>
\t<true/>
\t<key>KeepAlive</key>
\t<true/>
\t<key>ThrottleInterval</key>
\t<integer>30</integer>
\t<key>ProcessType</key>
\t<string>Background</string>
\t<key>StandardOutPath</key>
\t<string>/h/log</string>
\t<key>StandardErrorPath</key>
\t<string>/h/log</string>
</dict>
</plist>
";

    #[test]
    fn the_old_agent_renders_as_0_37_0_wrote_it_and_differs_only_in_the_label() {
        let old = render_old(Path::new(PROGRAM), Path::new("/h/log"));
        let new = render(Kind::Launchd, Path::new(PROGRAM), Path::new("/h/log"));

        assert_eq!(old, OLD_PLIST);
        assert_eq!(new.replace(LABEL, OLD_LABEL), OLD_PLIST);
        assert_eq!(
            Kind::Launchd.file(Path::new("/h")),
            Path::new(
                "/h/Library/LaunchAgents/com.github.pragmatic-engineer.playbook.collector.plist"
            )
        );
        assert_eq!(
            old_file(Path::new("/h")),
            Path::new(
                "/h/Library/LaunchAgents/io.github.pragmatic-engineer.playbook.collector.plist"
            )
        );
    }

    #[test]
    fn find_old_reads_the_old_agent_and_says_whether_playbook_wrote_it() {
        let home = scratch_dir("service-old");
        assert_eq!(find_old(&home), None);
        let file = old_file(&home);
        fs::create_dir_all(file.parent().unwrap()).unwrap();
        fs::write(&file, OLD_PLIST).unwrap();
        let ours = find_old(&home).unwrap();
        fs::write(
            &file,
            OLD_PLIST.replace("<!-- The playbook", "<!-- Mine, the playbook"),
        )
        .unwrap();
        let no_marker = find_old(&home).unwrap();
        fs::write(
            &file,
            render(Kind::Launchd, Path::new(PROGRAM), Path::new("/h/log")),
        )
        .unwrap();
        let other_label = find_old(&home).unwrap();

        assert!(ours.ours);
        assert_eq!(ours.file, file);
        assert!(!no_marker.ours);
        assert!(!other_label.ours);
    }
}
