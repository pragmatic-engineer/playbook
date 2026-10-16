// SPDX-FileCopyrightText: 2026 Igor Santos
// SPDX-License-Identifier: Apache-2.0

//! The `collector-service` part of `playbook setup` (ADR 0043). With consent
//! it installs a per-user background service that runs `playbook usage
//! collect`, so the telemetry the telemetry part points Claude Code at has a
//! receiver after every login. The service files live in
//! [`crate::usage::service`]; this module decides, records the answer, and
//! reports.

use crate::init::run::StepReport;
use crate::usage::service::{self, Installed, Kind, Manager};
use std::path::{Path, PathBuf};

/// The part id, also the name of the step and the setup flag.
pub const PART: &str = "collector-service";

/// The command that installs the service.
pub const COMMAND: &str = "playbook setup --only collector-service";

/// The command that runs the collector in the foreground.
pub const COLLECT: &str = "playbook usage collect";

/// The state key that records a no, so the reminder stops.
const DECLINED_KEY: &str = "collector-service/declined";

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

/// Whether the Claude Code user settings point telemetry at the playbook
/// collector, the case where a receiver is needed.
pub fn telemetry_points_here(home: &Path) -> bool {
    use crate::usage::telemetry_detect::{configured_from_files, Configured};
    let paths = crate::usage::run::Paths::from_home(home);
    let port = crate::usage::collector::read_state(&crate::usage::collector::state_path(&paths))
        .map(|s| s.port)
        .filter(|p| *p != 0);
    configured_from_files(home, home, port) == Configured::Playbook
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
    let report = match service::install(kind, home, program, manager, service::uid()) {
        Ok(Installed::Unchanged) => {
            StepReport::already_correct(PART, format!("the {} runs the collector", kind.name()))
        }
        Ok(Installed::Started(Ok(()))) => StepReport::wired(
            PART,
            format!("started the {} that was not running", kind.name()),
        ),
        Ok(Installed::Started(Err(why))) => StepReport::already_correct(
            PART,
            format!(
                "{} is in place but did not start ({why}); to start it, run: {}",
                kind.file(home).display(),
                service::load_hint(kind, &kind.file(home), service::uid())
            ),
        ),
        Ok(Installed::Wrote { started: Ok(()) }) => StepReport::wired(
            PART,
            format!(
                "installed the {} {}; it runs `{COLLECT}` now and after every login",
                kind.name(),
                kind.file(home).display()
            ),
        ),
        Ok(Installed::Wrote { started: Err(why) }) => StepReport::wired(
            PART,
            format!(
                "wrote {}, but it did not start ({why}); to start it, run: {}",
                kind.file(home).display(),
                service::load_hint(kind, &kind.file(home), service::uid())
            ),
        ),
        Err(err) => StepReport::failed(PART, err),
    };
    if report.status != crate::init::run::StepStatus::Failed {
        record_declined(home, false);
    }
    report
}

/// Whether a collector answers for the user at `home`.
pub fn collector_running(home: &Path) -> bool {
    let paths = crate::usage::run::Paths::from_home(home);
    crate::usage::collector::read_state(&crate::usage::collector::state_path(&paths))
        .is_some_and(|s| crate::usage::collector::running(&s))
}

/// What to do when telemetry points at the collector and none runs, as one
/// line for the fix hints of setup and doctor.
pub fn collector_fix(home: &Path) -> String {
    match service::find(home) {
        Some(found) => format!(
            "start it with `{COLLECT}`; the {} is installed but the collector is not up, see {}",
            found.kind.name(),
            service::log_path(home).display()
        ),
        None => format!(
            "start it with `{COLLECT}`, or keep it running in the background with `{COMMAND}`"
        ),
    }
}

/// The line for setup's "Next steps" when telemetry points at the collector
/// and none runs. `None` when this run installed or kept the service, since
/// the service manager starts it.
pub fn next_step(home: &Path, steps: &[StepReport]) -> Option<String> {
    use crate::init::run::StepStatus;
    let service_ran = steps.iter().any(|s| {
        s.name == PART && matches!(s.status, StepStatus::Wired | StepStatus::AlreadyCorrect)
    });
    if service_ran || !telemetry_points_here(home) || collector_running(home) {
        return None;
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
    let want = service::render(kind, program, &service::log_path(home));
    if std::fs::read_to_string(kind.file(home)).is_ok_and(|now| now == want) {
        format!("{file}: no change, already installed")
    } else {
        format!("{file}: would write the {} and start it", kind.name())
    }
}

/// The step `playbook uninstall` runs: stops and deletes the service file.
/// `Ok(None)` when there is none.
pub fn remove(home: &Path, dry_run: bool) -> Result<Option<String>, String> {
    let Some(found) = service::find(home) else {
        return Ok(None);
    };
    if dry_run {
        return Ok(Some(format!(
            "would stop and remove the {}",
            found.describe()
        )));
    }
    let manager = service::manager_for(home);
    service::remove(found.kind, home, manager.as_ref(), service::uid())?;
    Ok(Some(format!(
        "stopped and removed the {}",
        found.describe()
    )))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::common::test_support::scratch_dir;
    use crate::init::run::StepStatus;
    use std::fs;

    /// Runs nothing and reports every service as stopped.
    struct Stopped;

    impl Manager for Stopped {
        fn run(&self, _: &str, args: &[String]) -> Result<(), String> {
            if args.iter().any(|a| a == "print" || a == "is-active") {
                Err("not loaded".into())
            } else {
                Ok(())
            }
        }
    }

    /// A scratch home whose settings point telemetry at the collector.
    fn telemetry_home(tag: &str) -> PathBuf {
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
            fn run(&self, _: &str, _: &[String]) -> Result<(), String> {
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
    fn a_scratch_home_never_loads_a_real_service() {
        let home = telemetry_home("scratch");
        let manager = service::manager_for(&home);

        let report = step(
            &home,
            Path::new("/p"),
            Choice::Install,
            Some(Kind::Launchd),
            manager.as_ref(),
        );

        assert!(
            report
                .detail
                .contains("HOME is not the home of your account"),
            "{}",
            report.detail
        );
    }

    #[test]
    fn next_steps_name_the_collector_only_when_telemetry_is_on_and_it_is_down() {
        // Arrange
        let on = telemetry_home("next");
        let off = scratch_dir("collector-service-next-off");
        let kept = [StepReport::already_correct(PART, "runs")];

        // Act
        let down = next_step(&on, &[]);
        let after_service = next_step(&on, &kept);
        let no_telemetry = next_step(&off, &[]);

        // Assert
        let down = down.expect("a next step");
        assert!(down.contains(COLLECT) && down.contains(COMMAND), "{down}");
        assert_eq!(after_service, None);
        assert_eq!(no_telemetry, None);
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
