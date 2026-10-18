// SPDX-FileCopyrightText: 2026 Igor Santos
// SPDX-License-Identifier: Apache-2.0

//! The `collector-service` part of `playbook setup` (ADR 0043). With consent
//! it installs a per-user background service that runs `playbook usage
//! collect`, so the telemetry the telemetry part points Claude Code at has a
//! receiver after every login. The service files live in
//! [`crate::usage::service`]; this module decides, records the answer, and
//! reports.

use crate::init::run::{StepReport, StepStatus};
use crate::service_manager::{self as manager, Installed, Manager};
use crate::usage::service::{self, Kind};
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

/// The part id, also the name of the step and the setup flag.
pub const PART: &str = "collector-service";

/// The command that installs the service.
pub const COMMAND: &str = "playbook setup --only collector-service";

/// The command that runs the collector in the foreground.
pub const COLLECT: &str = "playbook usage collect";

/// How long setup waits for a service it just started to bring the
/// collector up, before its next steps call the collector down.
pub const START_WAIT: Duration = Duration::from_secs(2);

/// The state key that records a no, so the reminder stops.
const DECLINED_KEY: &str = "collector-service/declined";

/// The state key that records the one reminder of migration 0017.
const REMINDED_KEY: &str = "collector-service/reminded";

/// What setup does with the part, decided from the flags and the answer.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Choice {
    /// Not asked and no flag, as in a run with no terminal: nothing changes.
    Skip,
    /// The user said no: nothing changes and the reminder stops.
    Declined,
    /// Install the service, or bring it up to date.
    Install,
}

fn root(home: &Path) -> PathBuf {
    crate::common::paths::playbook_root_from(home)
}

fn has_key(home: &Path, key: &str) -> bool {
    crate::state::list_read_only(&root(home), key).is_ok_and(|rows| !rows.is_empty())
}

/// Whether the user said no, read without creating or changing the store.
pub fn declined(home: &Path) -> bool {
    has_key(home, DECLINED_KEY)
}

/// Records or clears the no. A failure only means the reminder may show again.
fn record_declined(home: &Path, no: bool) {
    let _ = match no {
        true => crate::state::set(&root(home), DECLINED_KEY, "1"),
        false if declined(home) => crate::state::delete(&root(home), DECLINED_KEY).map(|_| ()),
        false => Ok(()),
    };
}

/// The collector state recorded for the user at `home`.
fn collector_state(home: &Path) -> Option<crate::usage::collector::State> {
    let paths = crate::usage::run::Paths::from_home(home);
    crate::usage::collector::read_state(&crate::usage::collector::state_path(&paths))
}

/// Whether the Claude Code user settings point telemetry at the playbook
/// collector, the case where a receiver is needed.
pub fn telemetry_points_here(home: &Path) -> bool {
    use crate::usage::telemetry_detect::{configured_from_files, Configured};
    let port = collector_state(home).map(|s| s.port).filter(|p| *p != 0);
    configured_from_files(home, home, port) == Configured::Playbook
}

/// Whether a collector answers for the user at `home`.
pub fn collector_running(home: &Path) -> bool {
    collector_state(home).is_some_and(|s| crate::usage::collector::running(&s))
}

/// The `playbook` the service starts: the copy on PATH that is this binary,
/// so a Homebrew upgrade keeps working, else this binary's own path.
pub fn program() -> PathBuf {
    let exe = std::env::current_exe().unwrap_or_else(|_| PathBuf::from("playbook"));
    let name = exe
        .file_name()
        .map_or_else(|| "playbook".into(), |n| n.to_os_string());
    crate::init::path::resolve_bin_dir(&std::env::var_os("PATH").unwrap_or_default(), &exe)
        .join(name)
}

/// The line that replaces the question when a service is already installed
/// and its program exists, else `None`.
pub fn skip_line(home: &Path) -> Option<String> {
    let found = service::find(home).filter(service::Found::program_exists)?;
    Some(format!(
        "collector-service: already installed as the {}",
        found.describe()
    ))
}

/// The detail for a service file that is in place but did not start.
fn not_started(kind: Kind, file: &Path, why: &str) -> String {
    format!(
        "{} is in place but did not start ({why}); to start it, run: {}",
        file.display(),
        manager::load_hint(kind, file, manager::uid())
    )
}

/// The setup step for `choice`. A no is recorded, and a yes clears it.
pub fn step(
    home: &Path,
    program: &Path,
    choice: Choice,
    kind: Option<Kind>,
    manager: &dyn Manager,
) -> StepReport {
    match choice {
        Choice::Skip => {
            return StepReport::skipped(PART, "not requested; pass --collector-service to opt in")
        }
        Choice::Declined => {
            record_declined(home, true);
            let mut detail = format!(
                "you said no, so playbook stops reminding you; to add it later, run: {COMMAND}"
            );
            if let Some(found) = service::find(home) {
                detail.push_str(&format!(
                    "; the {} that is installed stays, and `playbook uninstall` removes it",
                    found.kind.name()
                ));
            }
            return StepReport::skipped(PART, detail);
        }
        Choice::Install => {}
    }
    if !telemetry_points_here(home) {
        return StepReport::skipped(
            PART,
            "telemetry does not point at the playbook collector, so no service is needed; turn telemetry on first with: playbook setup --only telemetry",
        );
    }
    let Some(kind) = kind else {
        return StepReport::skipped(
            PART,
            format!(
                "this system has no launchd or systemd, so playbook writes no service; keep this running yourself: {}",
                service::manual_command(&program.to_string_lossy())
            ),
        );
    };
    let file = kind.file(home);
    let outdated =
        collector_state(home).is_some_and(|s| crate::usage::collector::runs_other_build(&s));
    let report = match manager::install(kind, home, program, manager, manager::uid(), outdated) {
        Ok(Installed::Unchanged) => {
            StepReport::already_correct(PART, format!("the {} runs the collector", kind.name()))
        }
        Ok(Installed::Restarted(Ok(()))) => StepReport::wired(
            PART,
            format!("restarted the {} so it runs this version", kind.name()),
        ),
        Ok(Installed::Restarted(Err(why))) => StepReport::already_correct(
            PART,
            format!(
                "the {} runs an older collector and did not restart ({why})",
                kind.name()
            ),
        ),
        Ok(Installed::Started(Ok(()))) => StepReport::wired(
            PART,
            format!("started the {} that was not running", kind.name()),
        ),
        Ok(Installed::Started(Err(why))) => {
            StepReport::already_correct(PART, not_started(kind, &file, &why))
        }
        Ok(Installed::Wrote { started: Ok(()) }) => StepReport::wired(
            PART,
            format!(
                "installed the {} {}; it runs `{COLLECT}` now and after every login",
                kind.name(),
                file.display()
            ),
        ),
        Ok(Installed::Wrote { started: Err(why) }) => {
            StepReport::wired(PART, not_started(kind, &file, &why))
        }
        Err(err) => StepReport::failed(PART, err),
    };
    if report.status != StepStatus::Failed {
        record_declined(home, false);
    }
    report
}

/// What to do when telemetry points at the collector and none runs, as one
/// line for the fix hints of setup and doctor.
pub fn collector_fix(home: &Path) -> String {
    service::collector_fix(home)
}

/// The line for setup's "Next steps" when telemetry points at the collector
/// and none runs. When this run installed, started or kept the service, it
/// first waits up to `wait` for the collector to come up.
pub fn next_step(home: &Path, steps: &[StepReport], wait: Duration) -> Option<String> {
    if !telemetry_points_here(home) {
        return None;
    }
    let service_ran = steps.iter().any(|s| {
        s.name == PART && matches!(s.status, StepStatus::Wired | StepStatus::AlreadyCorrect)
    });
    let deadline = Instant::now() + if service_ran { wait } else { Duration::ZERO };
    loop {
        if collector_running(home) {
            return None;
        }
        if Instant::now() >= deadline {
            break;
        }
        std::thread::sleep(Duration::from_millis(100));
    }
    Some(format!(
        "Telemetry is on, but the collector is not running, so nothing is stored. To fix it, {}.",
        collector_fix(home)
    ))
}

/// What `--dry-run` says for the part, without writing or running anything.
pub fn preview(home: &Path, telemetry_on: bool, program: &Path) -> String {
    let Some(kind) = Kind::current() else {
        return format!(
            "no launchd or systemd here, so nothing is written; run it yourself: {}",
            service::manual_command(&program.to_string_lossy())
        );
    };
    let file = kind.file(home).display().to_string();
    if !telemetry_on {
        return format!("{file}: skipped, telemetry does not point at the playbook collector");
    }
    if service::is_current(kind, home, program) {
        format!("{file}: no change, already installed")
    } else {
        format!("{file}: would write the {} and start it", kind.name())
    }
}

/// The `Consent` check of migration 0017: one line when telemetry points at
/// the collector and no service keeps it running. Nothing after the user
/// said no or after the reminder was shown. It writes nothing, so a plan or
/// a status read never uses up the reminder; see [`record_reminder`].
pub fn pending(home: &Path) -> Option<String> {
    if Kind::current().is_none()
        || declined(home)
        || has_key(home, REMINDED_KEY)
        || service::find(home).is_some()
        || service::find_old(home).is_some_and(|o| o.ours)
        || !telemetry_points_here(home)
    {
        return None;
    }
    Some(
        "telemetry is on, but nothing keeps the usage collector running, so telemetry is lost while it is down"
            .to_string(),
    )
}

/// Records that the reminder of migration 0017 was shown, so later checks
/// stay quiet. The heal calls it after it printed the line.
pub fn record_reminder(home: &Path) {
    let _ = crate::state::set(&root(home), REMINDED_KEY, "1");
}

/// The commands that remove an agent file under the old label that
/// playbook did not write, which playbook never touches itself.
pub fn foreign_old_agent_fix(file: &Path) -> String {
    format!(
        "launchctl bootout gui/{}/{}; rm {}",
        manager::uid(),
        service::OLD_LABEL,
        crate::common::shell::word(&file.to_string_lossy())
    )
}

/// The step `playbook uninstall` runs: stops and deletes the service file,
/// and the agent playbook wrote under the label 0.37.0 used. `Ok(None)` when
/// there is neither.
pub fn remove(home: &Path, dry_run: bool) -> Result<Option<String>, String> {
    let found = service::find(home);
    let old = service::find_old(home).filter(|o| o.ours);
    let names: Vec<String> = found
        .iter()
        .map(service::Found::describe)
        .chain(old.iter().map(service::OldAgent::describe))
        .collect();
    if names.is_empty() {
        return Ok(None);
    }
    let names = names.join(" and the ");
    if dry_run {
        return Ok(Some(format!("would stop and remove the {names}")));
    }
    let manager = manager::manager_for(home);
    if let Some(found) = &found {
        manager::remove(found.kind, home, manager.as_ref(), manager::uid())?;
    }
    manager::remove_old(home, manager.as_ref(), manager::uid())?;
    Ok(Some(format!("stopped and removed the {names}")))
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use crate::common::test_support::scratch_dir;
    use std::fs;

    /// Runs nothing and reports every service as stopped.
    struct Stopped;

    impl Manager for Stopped {
        fn run(&self, _: &str, args: &[String]) -> Result<String, String> {
            if args.iter().any(|a| a == "print" || a == "is-active") {
                Err("not loaded".into())
            } else {
                Ok(String::new())
            }
        }
    }

    /// A scratch home whose settings point telemetry at the collector.
    pub(crate) fn telemetry_home(tag: &str) -> PathBuf {
        let home = scratch_dir(&format!("collector-service-{tag}"));
        fs::create_dir_all(home.join(".claude")).unwrap();
        let paths = crate::usage::run::Paths::from_home(&home);
        let state = crate::usage::collector::state_path(&paths);
        fs::create_dir_all(state.parent().unwrap()).unwrap();
        fs::write(&state, r#"{"pid":0,"port":47318,"token":"t"}"#).unwrap();
        fs::write(
            home.join(".claude/settings.json"),
            r#"{"env":{"CLAUDE_CODE_ENABLE_TELEMETRY":"1","OTEL_EXPORTER_OTLP_ENDPOINT":"http://127.0.0.1:47318","OTEL_LOGS_EXPORTER":"otlp","OTEL_EXPORTER_OTLP_PROTOCOL":"http/json","OTEL_EXPORTER_OTLP_HEADERS":"Authorization=Bearer t"}}"#,
        )
        .unwrap();
        home
    }

    #[test]
    fn install_writes_the_service_when_telemetry_points_at_the_collector() {
        // Arrange
        let home = telemetry_home("install");

        // Act
        let first = step(
            &home,
            Path::new("/bin/playbook"),
            Choice::Install,
            Some(Kind::Launchd),
            &Stopped,
        );

        // Assert
        assert!(telemetry_points_here(&home));
        assert_eq!(first.status, StepStatus::Wired, "{}", first.detail);
        assert!(first.detail.contains("launchd agent"), "{}", first.detail);
        let plist = fs::read_to_string(Kind::Launchd.file(&home)).unwrap();
        assert!(!plist.contains("Bearer"), "{plist}");
    }

    #[test]
    fn telemetry_off_skips_with_the_telemetry_command() {
        let home = scratch_dir("collector-service-off");

        let report = step(
            &home,
            Path::new("/bin/playbook"),
            Choice::Install,
            Some(Kind::Systemd),
            &Stopped,
        );

        assert_eq!(report.status, StepStatus::Skipped);
        assert!(
            report.detail.contains("--only telemetry"),
            "{}",
            report.detail
        );
        assert!(!Kind::Systemd.file(&home).exists());
    }

    #[test]
    fn a_system_without_a_manager_prints_the_command_to_run() {
        let home = telemetry_home("manual");

        let report = step(
            &home,
            Path::new("/bin/playbook"),
            Choice::Install,
            None,
            &Stopped,
        );

        assert_eq!(report.status, StepStatus::Skipped);
        assert!(
            report
                .detail
                .contains("/bin/playbook usage collect --idle-seconds 0"),
            "{}",
            report.detail
        );
    }

    #[test]
    fn a_no_is_remembered_and_a_later_yes_clears_it() {
        let home = telemetry_home("declined");

        let no = step(
            &home,
            Path::new("/p"),
            Choice::Declined,
            Some(Kind::Systemd),
            &Stopped,
        );
        let after_no = declined(&home);
        let yes = step(
            &home,
            Path::new("/p"),
            Choice::Install,
            Some(Kind::Systemd),
            &Stopped,
        );

        assert_eq!(no.status, StepStatus::Skipped);
        assert!(no.detail.contains(COMMAND), "{}", no.detail);
        assert!(after_no);
        assert_eq!(yes.status, StepStatus::Wired, "{}", yes.detail);
        assert!(!declined(&home));
    }

    #[test]
    fn skip_writes_and_records_nothing() {
        let home = telemetry_home("skip");

        let report = step(
            &home,
            Path::new("/p"),
            Choice::Skip,
            Some(Kind::Launchd),
            &Stopped,
        );

        assert_eq!(report.status, StepStatus::Skipped);
        assert!(!Kind::Launchd.file(&home).exists());
        assert!(!declined(&home));
    }

    #[test]
    fn a_start_that_fails_names_the_commands_to_start_it() {
        struct Refuses;
        impl Manager for Refuses {
            fn run(&self, _: &str, _: &[String]) -> Result<String, String> {
                Err("no session".into())
            }
        }
        let home = telemetry_home("refused");

        let report = step(
            &home,
            Path::new("/p"),
            Choice::Install,
            Some(Kind::Systemd),
            &Refuses,
        );

        assert_eq!(report.status, StepStatus::Wired);
        assert!(report.detail.contains("no session"), "{}", report.detail);
        assert!(
            report.detail.contains("systemctl --user enable"),
            "{}",
            report.detail
        );
    }

    #[test]
    fn next_steps_name_the_collector_when_telemetry_is_on_and_none_runs() {
        // Arrange
        let on = telemetry_home("next");
        let off = scratch_dir("collector-service-next-off");
        let up = telemetry_home("next-up");
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        let paths = crate::usage::run::Paths::from_home(&up);
        fs::write(
            crate::usage::collector::state_path(&paths),
            format!(
                r#"{{"pid":{},"port":{port},"token":"t"}}"#,
                std::process::id()
            ),
        )
        .unwrap();
        fs::write(
            up.join(".claude/settings.json"),
            format!(r#"{{"env":{{"CLAUDE_CODE_ENABLE_TELEMETRY":"1","OTEL_EXPORTER_OTLP_ENDPOINT":"http://127.0.0.1:{port}"}}}}"#),
        )
        .unwrap();
        let kept = [StepReport::already_correct(PART, "runs")];

        // Act
        let down = next_step(&on, &[], Duration::ZERO);
        let kept_but_down = next_step(&on, &kept, Duration::from_millis(200));
        let no_telemetry = next_step(&off, &[], Duration::ZERO);

        // Assert
        let down = down.expect("a next step");
        // The commands come from the usage host, which a unit test may not set.
        assert!(
            down.contains(" collect`") && down.contains("--only collector-service"),
            "{down}"
        );
        assert!(
            kept_but_down.is_some(),
            "a service that did not start still needs the line"
        );
        assert_eq!(no_telemetry, None);
        // A collector that answers needs no line.
        assert!(telemetry_points_here(&up));
        assert_eq!(next_step(&up, &[], Duration::ZERO), None);
        drop(listener);
    }

    #[test]
    fn preview_and_skip_line_follow_the_file_on_disk() {
        let Some(kind) = Kind::current() else {
            return;
        };
        let home = telemetry_home("preview");
        let program = home.join("bin/playbook");
        fs::create_dir_all(program.parent().unwrap()).unwrap();
        fs::write(&program, "").unwrap();

        let before = preview(&home, true, &program);
        let off = preview(&home, false, &program);
        let no_line = skip_line(&home);
        step(&home, &program, Choice::Install, Some(kind), &Stopped);
        let after = preview(&home, true, &program);
        let line = skip_line(&home);
        fs::remove_file(&program).unwrap();

        assert!(
            before.ends_with(&format!("would write the {} and start it", kind.name())),
            "{before}"
        );
        assert!(off.contains("skipped, telemetry does not point"), "{off}");
        assert_eq!(no_line, None);
        assert!(after.ends_with("no change, already installed"), "{after}");
        assert!(line.is_some_and(|l| l.starts_with("collector-service: already installed")));
        assert_eq!(skip_line(&home), None, "a gone program asks again");
    }

    #[test]
    fn the_reminder_shows_once_and_never_after_a_no() {
        if Kind::current().is_none() {
            return;
        }
        let home = telemetry_home("remind");
        let said_no = telemetry_home("remind-no");
        step(
            &said_no,
            Path::new("/p"),
            Choice::Declined,
            Kind::current(),
            &Stopped,
        );

        let first = pending(&home);
        let again = pending(&home);
        record_reminder(&home);
        let after = pending(&home);

        assert!(first.is_some_and(|l| l.contains("collector")));
        assert!(again.is_some(), "the check alone writes nothing");
        assert_eq!(after, None);
        assert_eq!(pending(&said_no), None);
    }

    #[test]
    fn an_installed_service_needs_no_reminder() {
        if Kind::current().is_none() {
            return;
        }
        let home = telemetry_home("remind-installed");
        step(
            &home,
            Path::new("/p"),
            Choice::Install,
            Kind::current(),
            &Stopped,
        );

        assert_eq!(pending(&home), None);
    }

    #[test]
    fn uninstall_removes_the_file_and_a_dry_run_only_names_it() {
        let home = telemetry_home("remove");
        let Some(kind) = Kind::current() else {
            return;
        };
        step(
            &home,
            Path::new("/p"),
            Choice::Install,
            Some(kind),
            &Stopped,
        );

        let dry = remove(&home, true).unwrap().unwrap();
        let still_there = kind.file(&home).exists();
        let done = remove(&home, false).unwrap().unwrap();

        assert!(dry.starts_with("would stop and remove"), "{dry}");
        assert!(still_there);
        assert!(done.starts_with("stopped and removed"), "{done}");
        assert!(!kind.file(&home).exists());
        assert_eq!(remove(&home, false).unwrap(), None);
    }
}
