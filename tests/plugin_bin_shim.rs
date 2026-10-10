// SPDX-FileCopyrightText: 2026 Igor Santos
// SPDX-License-Identifier: Apache-2.0

//! `bin/playbook`, the plugin shim: runs the real binary, or bootstraps it
//! through the shipped `install.sh` pinned to the plugin version. The shim is
//! the real script, `install.sh` is a stub that logs how it was called.

#[path = "support/shell_fixture.rs"]
mod fx;

use fx::{repo_root, stderr, stdout, write_exec, Work};
use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};

const REAL: &str = "#!/usr/bin/env bash\necho \"real playbook $*\"\n";

const INSTALL_STUB: &str = r#"echo "called" >> "$STUB_LOG"
echo "ref=${PLAYBOOK_REF:-} args=$*" >> "$STUB_LOG"
echo "guard=${PLAYBOOK_SHIM_BOOTSTRAP:-}" >> "$STUB_LOG"
if [ "${INSTALL_WRITES:-0}" = "1" ]; then
    mkdir -p "$PLAYBOOK_BIN_DIR"
    printf '#!/usr/bin/env bash\necho "real playbook $*"\n' > "$PLAYBOOK_BIN_DIR/playbook"
    chmod +x "$PLAYBOOK_BIN_DIR/playbook"
fi
exit "${INSTALL_EXIT:-0}"
"#;

struct Plugin {
    work: Work,
    dir: PathBuf,
}

impl Plugin {
    fn new(tag: &str) -> Plugin {
        let work = Work::new(tag);
        let dir = work.dir("plugin");
        fs::create_dir_all(dir.join("bin")).unwrap();
        fs::create_dir_all(dir.join(".claude-plugin")).unwrap();
        fs::copy(repo_root().join("bin/playbook"), dir.join("bin/playbook")).unwrap();
        write_exec(
            &dir.join("bin/playbook"),
            &fs::read_to_string(dir.join("bin/playbook")).unwrap(),
        );
        fs::write(
            dir.join(".claude-plugin/plugin.json"),
            "{\n  \"name\": \"playbook\",\n  \"version\": \"9.9.9\"\n}\n",
        )
        .unwrap();
        write_exec(&dir.join("install.sh"), INSTALL_STUB);
        Plugin { work, dir }
    }

    fn log(&self) -> String {
        fs::read_to_string(self.work.path("stub.log")).unwrap_or_default()
    }

    fn calls(&self) -> usize {
        self.log().lines().filter(|l| *l == "called").count()
    }

    /// PATH holds the shim dir and system dirs only, plus `extra`.
    fn run(
        &self,
        home: &Path,
        extra_path: Option<&Path>,
        envs: &[(&str, &str)],
        args: &[&str],
    ) -> Output {
        let mut path = self.dir.join("bin").display().to_string();
        if let Some(extra) = extra_path {
            path.push(':');
            path.push_str(&extra.display().to_string());
        }
        path.push_str(":/usr/bin:/bin");
        let mut cmd = Command::new("bash");
        cmd.arg(self.dir.join("bin/playbook"))
            .args(args)
            .env("HOME", home)
            .env("PLAYBOOK_BIN_DIR", home.join(".local/bin"))
            .env("STUB_LOG", self.work.path("stub.log"))
            .env("PATH", path);
        for (k, v) in envs {
            cmd.env(k, v);
        }
        cmd.output().expect("bash spawns")
    }
}

fn make_real(dir: &Path) {
    write_exec(&dir.join("playbook"), REAL);
}

fn has_system_binary() -> bool {
    [
        "/opt/homebrew/bin/playbook",
        "/usr/local/bin/playbook",
        "/home/linuxbrew/.linuxbrew/bin/playbook",
    ]
    .iter()
    .any(|p| Path::new(p).exists())
}

#[test]
fn runs_the_real_binary_with_its_arguments_and_installs_nothing() {
    let p = Plugin::new("shim-real");
    let home = p.work.dir("h1");
    make_real(&home.join(".local/bin"));
    let out = p.run(&home, None, &[], &["mode", "status"]);
    assert_eq!(stdout(&out).trim_end(), "real playbook mode status");
    assert!(p.log().is_empty(), "{}", p.log());
}

#[test]
fn skips_its_own_directory_and_finds_the_real_binary_further_down_path() {
    if has_system_binary() {
        eprintln!("SKIP: a playbook binary exists in a fixed system location");
        return;
    }
    let p = Plugin::new("shim-skip");
    let home = p.work.dir("h2");
    let elsewhere = p.work.dir("elsewhere");
    make_real(&elsewhere);
    let out = p.run(&home, Some(&elsewhere), &[], &["--version"]);
    assert_eq!(stdout(&out).trim_end(), "real playbook --version");
}

#[test]
fn bootstraps_through_install_sh_pinned_to_the_plugin_version() {
    if has_system_binary() {
        eprintln!("SKIP: a playbook binary exists in a fixed system location");
        return;
    }
    let p = Plugin::new("shim-boot");
    let home = p.work.dir("h3");
    let out = p.run(&home, None, &[("INSTALL_WRITES", "1")], &["init"]);
    let log = p.log();
    assert_eq!(stdout(&out).trim_end(), "real playbook init", "{log}");
    assert_eq!(p.calls(), 1, "{log}");
    assert!(log.contains("ref=v9.9.9 args=--yes --binary-only"), "{log}");
    assert!(log.contains("guard=1"), "{log}");
}

#[test]
fn a_failed_install_exits_127_after_one_attempt_and_prints_the_manual_command() {
    if has_system_binary() {
        return;
    }
    let p = Plugin::new("shim-fail");
    let home = p.work.dir("h4");
    let out = p.run(&home, None, &[("INSTALL_EXIT", "1")], &["init"]);
    assert_eq!(out.status.code(), Some(127));
    assert_eq!(p.calls(), 1);
    assert!(stderr(&out).contains("install.sh"), "{}", stderr(&out));
}

#[test]
fn an_installer_that_places_nothing_does_not_loop() {
    if has_system_binary() {
        return;
    }
    let p = Plugin::new("shim-noloop");
    let home = p.work.dir("h5");
    let out = p.run(&home, None, &[], &["init"]);
    assert_eq!(out.status.code(), Some(127));
    assert_eq!(p.calls(), 1);
}

#[test]
fn the_guard_variable_blocks_a_nested_install() {
    if has_system_binary() {
        return;
    }
    let p = Plugin::new("shim-guard");
    let home = p.work.dir("h6");
    let out = p.run(&home, None, &[("PLAYBOOK_SHIM_BOOTSTRAP", "1")], &["init"]);
    assert_eq!(out.status.code(), Some(127));
    assert!(p.log().is_empty(), "{}", p.log());
}

#[test]
fn the_worktree_hooks_fail_loudly_with_no_binary_and_never_install() {
    if has_system_binary() {
        eprintln!("SKIP: a playbook binary exists in a fixed system location");
        return;
    }
    for hook in ["worktree-create", "worktree-remove"] {
        let p = Plugin::new(&format!("shim-wt-{hook}"));
        let home = p.work.dir("h");
        let out = p.run(&home, None, &[], &["hook", hook]);
        assert_eq!(out.status.code(), Some(127), "{hook}");
        assert!(stdout(&out).is_empty(), "{hook}: stdout must stay empty");
        let err = stderr(&out);
        assert!(
            err.contains("worktree hook cannot run") && err.contains("install.sh"),
            "{err}"
        );
        assert_eq!(p.calls(), 0, "{hook} must not run the installer");
    }
}

#[test]
fn every_playbook_command_in_hooks_json_goes_through_the_shim() {
    let raw = fs::read_to_string(repo_root().join("hooks/hooks.json")).unwrap();
    let json: serde_json::Value = serde_json::from_str(&raw).unwrap();
    let mut seen = 0;
    for groups in json["hooks"].as_object().unwrap().values() {
        for group in groups.as_array().unwrap() {
            for hook in group["hooks"].as_array().unwrap() {
                let cmd = hook["command"].as_str().unwrap();
                if cmd.contains("playbook") {
                    seen += 1;
                    assert!(
                        cmd.starts_with("\"${CLAUDE_PLUGIN_ROOT}/bin/playbook\" "),
                        "bare playbook in hooks.json: {cmd}"
                    );
                }
            }
        }
    }
    assert_eq!(seen, 3);
}
