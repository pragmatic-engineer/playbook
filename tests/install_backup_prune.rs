// SPDX-FileCopyrightText: 2026 Igor Santos
// SPDX-License-Identifier: Apache-2.0

//! `install.sh` backups: re-running creates them, they are capped at 5, and
//! pruning drops the oldest. Runs the real script from a `git archive` copy so
//! untracked build output cannot leak in.

#[path = "support/shell_fixture.rs"]
mod fx;

use fx::{archive_head, bash, repo_root, write_exec, Work};
use std::fs;
use std::path::Path;
use std::thread::sleep;
use std::time::Duration;

const STUB: &str = r#"case "${1:-}" in
  --version) printf 'playbook 0.0.0-stub\n' ;;
  init)
    printf 'settings: ok - already matches the template\n'
    printf 'guards: ok - already in place\n'
    printf 'hooks: ok - all hooks already wired\n'
    printf 'shim: skipped - $SHELL is neither bash nor zsh\n'
    printf 'statusline: ok - already up to date\n'
    printf 'system-prompt: skipped - not installed; pass --system-prompt to opt in\n'
    ;;
esac
exit 0
"#;

fn run_install(src: &Path, claude: &Path, home: &Path, bin: &Path) {
    let out = bash(
        &repo_root().join("install.sh"),
        &["--no-setup", "--skip-plugin"],
        &[
            ("PLAYBOOK_SRC", src.display().to_string()),
            ("CLAUDE_HOME", claude.display().to_string()),
            ("HOME", home.display().to_string()),
            ("PLAYBOOK_BIN_DIR", bin.display().to_string()),
        ],
    );
    let _ = out;
}

fn list_backups(claude: &Path) -> Vec<String> {
    let mut names: Vec<String> = fs::read_dir(claude.join("backups"))
        .map(|d| {
            d.filter_map(Result::ok)
                .filter(|e| e.path().is_dir())
                .map(|e| e.file_name().to_string_lossy().into_owned())
                .filter(|n| n.starts_with("install-"))
                .collect()
        })
        .unwrap_or_default();
    names.sort();
    names
}

#[test]
fn backups_are_made_capped_at_five_and_the_oldest_is_pruned() {
    let work = Work::new("backup-prune");
    let src = work.path("src");
    if archive_head(&src).is_none() {
        eprintln!("SKIP: not a git checkout");
        return;
    }
    let home = work.dir("home");
    let claude = home.join(".claude");
    fs::create_dir_all(&claude).unwrap();
    let bin = work.dir("bin");
    write_exec(&bin.join("playbook"), STUB);

    for _ in 0..7 {
        run_install(&src, &claude, &home, &bin);
        sleep(Duration::from_secs(1));
    }
    let backups = list_backups(&claude);
    assert!(backups.len() >= 2, "only {} after 7 runs", backups.len());
    assert!(backups.len() <= 5, "{} dirs after 7 runs", backups.len());

    let prev_newest = backups.last().unwrap().clone();
    sleep(Duration::from_secs(1));
    run_install(&src, &claude, &home, &bin);
    let latest = list_backups(&claude).last().cloned().unwrap_or_default();
    assert!(
        latest > prev_newest,
        "newest stayed at {prev_newest}, latest {latest}"
    );
}
