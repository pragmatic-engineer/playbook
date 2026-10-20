// SPDX-FileCopyrightText: 2026 Igor Santos
// SPDX-License-Identifier: Apache-2.0

//! The `collector-service` part of `playbook setup` (ADR 0043). With consent
//! it installs a per-user background service that runs `playbook usage
//! collect`, so the telemetry the telemetry part points Claude Code at has a
//! receiver after every login. The service files live in
//! [`crate::usage::service`]; this module decides, records the answer, and
//! reports.

use crate::cc::telemetry::telemetry_points_here;
use crate::init::run::{StepReport, StepStatus};
use crate::service_manager::{self as manager, Installed, Manager};
use crate::usage::service::{self, Kind};
use std::path::{Path, PathBuf};
use std::time::Duration;

/// The part id, also the name of the step and the setup flag.
pub const PART: &str = "collector-service";

/// The command that installs the service.
pub const COMMAND: &str = "playbook setup --only collector-service";

/// The command that runs the collector in the foreground.
pub const COLLECT: &str = "playbook usage collect";

/// The state key that records a no, so setup stops asking. The whole part
/// goes in a later segment.
pub(crate) const DECLINED_KEY: &str = "collector-service/declined";

/// The state key the retired `collector-service` migration wrote after its
/// one reminder. Only `forget_answers` still names it.
pub(crate) const REMINDED_KEY: &str = "collector-service/reminded";

/// What setup does with the part, decided from the flags and the answer.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Choice {
    /// Not asked and no flag, as in a run with no terminal: nothing changes.
    Skip,
    /// The user said no: nothing changes and setup stops asking.
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

/// Records or clears the no. A failure only means setup may ask again.
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
    if kind == Kind::Launchd {
        // An agent left under the old label moves first, so setup never
        // loads a second collector next to it.
        let up = || collector_running(home);
        if let Err(why) =
            manager::move_old_agent(home, program, manager, manager::uid(), &up, Duration::ZERO)
        {
            return StepReport::failed(PART, format!("{why}; to try again, run: playbook migrate"));
        }
    }
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

/// The service managers playbook ever wrote a file for. Removal checks each
/// one on any system, so a file a synced home carried over goes too.
const KINDS: [Kind; 2] = [Kind::Launchd, Kind::Systemd];

/// The service files playbook wrote under `home`, in words, or `None` when
/// there are none. It reads only the files and runs nothing.
pub fn installed_names(home: &Path) -> Option<String> {
    let names: Vec<String> = KINDS
        .iter()
        .filter_map(|kind| service::find_kind(*kind, home))
        .map(|found| found.describe())
        .chain(
            service::find_old(home)
                .filter(|o| o.ours)
                .map(|o| o.describe()),
        )
        .collect();
    (!names.is_empty()).then(|| names.join(" and the "))
}

/// The step `playbook uninstall` and the `collector-service-removed`
/// migration run: stops and deletes every service file playbook wrote,
/// including the agent under the label 0.37.0 used. `Ok(None)` when there
/// is none.
pub fn remove(home: &Path, dry_run: bool) -> Result<Option<String>, String> {
    if dry_run {
        return Ok(installed_names(home).map(|names| format!("would stop and remove the {names}")));
    }
    let manager = manager::manager_for(home);
    remove_with(home, manager.as_ref())
}

/// [`remove`] with the service manager passed in, so tests use a fake one.
pub fn remove_with(home: &Path, manager: &dyn Manager) -> Result<Option<String>, String> {
    let Some(names) = installed_names(home) else {
        return Ok(None);
    };
    for kind in KINDS {
        manager::remove(kind, home, manager, manager::uid())?;
    }
    manager::remove_old(home, manager, manager::uid())?;
    Ok(Some(format!("stopped and removed the {names}")))
}

/// Deletes the answers the retired setup part stored. A key that stays is
/// read by nothing that changes behavior, so a failure is dropped and the
/// next run of the migration tries again.
pub fn forget_answers(home: &Path) {
    for key in [DECLINED_KEY, REMINDED_KEY] {
        if has_key(home, key) {
            let _ = crate::state::delete(&root(home), key);
        }
    }
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

    /// Writes the agent 0.37.0 wrote into `home`.
    fn put_old(home: &Path) {
        let file = service::old_file(home);
        fs::create_dir_all(file.parent().unwrap()).unwrap();
        fs::write(
            &file,
            service::render_old(Path::new("/p"), &service::log_path(home)),
        )
        .unwrap();
    }

    #[test]
    fn setup_moves_an_old_agent_before_it_installs_so_one_agent_is_loaded() {
        use crate::service_manager::fake::Launchd;
        let home = telemetry_home("setup-old");
        put_old(&home);
        let fake = Launchd::with(&[service::OLD_LABEL]);

        let report = step(
            &home,
            Path::new("/p"),
            Choice::Install,
            Some(Kind::Launchd),
            &fake,
        );

        assert_ne!(report.status, StepStatus::Failed, "{}", report.detail);
        assert_eq!(*fake.loaded.borrow(), vec![service::LABEL.to_string()]);
        assert!(!service::old_file(&home).exists());
    }

    #[test]
    fn uninstall_removes_both_agents_and_leaves_a_foreign_old_file() {
        if Kind::current() != Some(Kind::Launchd) {
            return;
        }
        let both = telemetry_home("remove-both");
        step(
            &both,
            Path::new("/p"),
            Choice::Install,
            Some(Kind::Launchd),
            &Stopped,
        );
        put_old(&both);
        let foreign = telemetry_home("remove-foreign");
        let file = service::old_file(&foreign);
        fs::create_dir_all(file.parent().unwrap()).unwrap();
        fs::write(&file, "<plist/>").unwrap();

        let done = remove(&both, false).unwrap().unwrap();
        let none = remove(&foreign, false).unwrap();

        assert!(
            done.contains(service::LABEL) && done.contains(service::OLD_LABEL),
            "{done}"
        );
        assert!(!Kind::Launchd.file(&both).exists() && !service::old_file(&both).exists());
        assert_eq!(none, None);
        assert!(file.is_file(), "a file playbook did not write stays");
    }
}
