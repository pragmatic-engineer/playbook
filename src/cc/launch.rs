// SPDX-FileCopyrightText: 2026 Igor Santos
// SPDX-License-Identifier: MIT

//! `playbook cc launch`: the launcher the `cc` and `ccd` shell functions call.
//!
//! Subcommands: (none) resume or start, `clean`, `fresh`, `raw [sid]`, `list`,
//! `prune`, `worktree <branch>`. The shell function only wraps this and `cd`s
//! to the path written to `$PLAYBOOK_CC_CD_FILE`, the one thing a child cannot do.

use super::{bust_cache, clean_resume, config_drift, retention, sessions, worktree_run};
use std::fs::{self, File};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

/// Env var naming the file the launcher writes the entered worktree path to.
pub const CD_FILE_ENV: &str = "PLAYBOOK_CC_CD_FILE";

const SYSTEM_PROMPT_REL: &str = "prompts/SYSTEM_PROMPT.md";

/// Options that consume the next argument, so it is not mistaken for a subcommand.
fn takes_value(opt: &str) -> bool {
    matches!(
        opt,
        "--system-prompt-file"
            | "--system-prompt"
            | "--append-system-prompt"
            | "--append-system-prompt-file"
            | "--settings"
            | "--setting-sources"
            | "--model"
            | "--permission-mode"
            | "--name"
            | "-n"
    )
}

/// Splits leading flags (with their values) from the rest; the subcommand is
/// the first non-flag token, so `ccd clean` works.
pub fn split_flags(args: &[String]) -> (Vec<String>, Vec<String>) {
    let mut flags = Vec::new();
    let mut i = 0;
    while i < args.len() && args[i].starts_with('-') {
        let arg = &args[i];
        flags.push(arg.clone());
        i += 1;
        if !arg.contains('=') && takes_value(arg) && i < args.len() {
            flags.push(args[i].clone());
            i += 1;
        }
    }
    (flags, args[i..].to_vec())
}

/// Runs the launcher and returns the exit code of the session (or the failure).
pub fn run(skip_permissions: bool, args: &[String]) -> i32 {
    ignore_sigint_in_parent();
    let mut all: Vec<String> = Vec::new();
    if skip_permissions {
        all.push("--dangerously-skip-permissions".into());
    }
    let prompt = crate::common::paths::playbook_root().join(SYSTEM_PROMPT_REL);
    if prompt.is_file() {
        all.push("--system-prompt-file".into());
        all.push(prompt.to_string_lossy().into_owned());
    }
    all.extend_from_slice(args);

    let mut cwd = PathBuf::from(super::logical_cwd());
    let rc = dispatch(&mut cwd, &all);
    retention::prune(&cwd.to_string_lossy());
    rc
}

extern "C" fn noop(_: libc::c_int) {}

/// Ctrl-C reaches the whole foreground group; the parent must outlive the
/// session to prune and report the worktree. A handler (not SIG_IGN) resets on exec.
fn ignore_sigint_in_parent() {
    // SAFETY: installing an empty handler is async-signal-safe.
    unsafe {
        libc::signal(libc::SIGINT, noop as *const () as libc::sighandler_t);
    }
}

fn dispatch(cwd: &mut PathBuf, args: &[String]) -> i32 {
    bust_cache::bust();
    clear_screen();
    let (flags, rest) = split_flags(args);
    let sub = rest.first().map(String::as_str).unwrap_or("");
    let cwd_str = cwd.to_string_lossy().into_owned();
    let name = cwd
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_default();
    let project_dir = sessions::project_dir(&super::claude_dir(), &cwd_str);

    if !matches!(
        sub,
        "list"
            | "ls"
            | "--list"
            | "prune"
            | "--prune"
            | "worktree"
            | "--worktree"
            | "new"
            | "--new"
    ) {
        let _ = crate::trust::run(&cwd_str);
    }
    let tail = rest.get(1..).unwrap_or(&[]);

    match sub {
        "clean" | "--clean" => clean(cwd, &project_dir, &name, &flags, tail),
        "fresh" | "--fresh" => {
            println!("-> cc: fresh session (no resume; settings.json applied)");
            config_drift::stamp(&cwd_str);
            claude(cwd, [&flags[..], &["-n".into(), name], tail].concat())
        }
        "raw" | "--raw" => raw(cwd, &project_dir, &name, &flags, tail),
        "list" | "ls" | "--list" => {
            print!("{}", sessions::render_list(&project_dir, &cwd_str));
            0
        }
        "prune" | "--prune" => {
            retention::prune(&cwd_str);
            0
        }
        "worktree" | "--worktree" | "new" | "--new" => worktree(cwd, &flags, tail),
        _ => resume(cwd, &project_dir, &name, &flags, &rest),
    }
}

fn clean(cwd: &Path, project_dir: &Path, name: &str, flags: &[String], tail: &[String]) -> i32 {
    let Some(old) = sessions::find_by_title(project_dir, name) else {
        println!("-> cc clean: no matching session for '{name}'; starting fresh");
        return claude(
            cwd,
            [&["-n".into(), name.to_string()], flags, tail].concat(),
        );
    };
    match clean_resume::prepare(project_dir, &old) {
        Ok(done) => {
            println!(
                "-> cc clean: cloned {}... -> {}... (stripped {} override entries)",
                short(&old),
                short(&done.new_sid),
                done.stripped
            );
            println!("-> cc clean: settings.json + plugins + hooks reload from current config");
            config_drift::stamp(&cwd.to_string_lossy());
            let head = ["--resume".into(), done.new_sid, "-n".into(), name.into()];
            claude(cwd, [&head[..], flags, tail].concat())
        }
        Err(clean_resume::CleanError::TranscriptMissing(path)) => {
            println!("-> cc clean: transcript missing at {}", path.display());
            1
        }
        Err(clean_resume::CleanError::Io(err)) => {
            println!("-> cc clean: {err}");
            1
        }
    }
}

fn raw(cwd: &Path, project_dir: &Path, name: &str, flags: &[String], tail: &[String]) -> i32 {
    let (sid, tail) = match tail.first() {
        Some(first) => (Some(first.clone()), &tail[1..]),
        None => (sessions::find_by_title(project_dir, name), tail),
    };
    let name_args = ["-n".to_string(), name.to_string()];
    match sid {
        None => {
            println!("-> cc raw: no matching session; starting fresh");
            claude(cwd, [flags, &name_args[..], tail].concat())
        }
        Some(sid) => {
            println!("-> cc raw: resuming {sid} (no fork, overrides preserved)");
            let head = ["--resume".to_string(), sid];
            claude(cwd, [flags, &head[..], &name_args[..], tail].concat())
        }
    }
}

/// Default: resume the newest titled session, forking when config drifted.
fn resume(cwd: &Path, project_dir: &Path, name: &str, flags: &[String], rest: &[String]) -> i32 {
    let cwd_str = cwd.to_string_lossy().into_owned();
    let name_args = ["-n".to_string(), name.to_string()];
    let Some(sid) = sessions::find_by_title(project_dir, name) else {
        config_drift::stamp(&cwd_str);
        return claude(cwd, [flags, &name_args[..], rest].concat());
    };
    let mut head = vec!["--resume".to_string(), sid.clone()];
    if config_drift::drifted(&cwd_str) {
        head.push("--fork-session".into());
        println!("-> cc: config changed; forking to reload settings/plugins/hooks");
    }
    let err_path = std::env::temp_dir().join(format!("playbook-cc-err-{}", std::process::id()));
    let stderr = File::create(&err_path).map(Stdio::from).ok();
    let argv = [flags, &name_args[..], &head[..], rest].concat();
    let rc = spawn(cwd, &argv, stderr);
    let err = fs::read_to_string(&err_path).unwrap_or_default();
    let _ = fs::remove_file(&err_path);
    if err.contains("No conversation found") {
        println!(
            "-> cc: session {}... not found; starting fresh",
            short(&sid)
        );
        return claude(cwd, [flags, &name_args[..], rest].concat());
    }
    eprint!("{err}");
    rc
}

fn worktree(cwd: &mut PathBuf, flags: &[String], tail: &[String]) -> i32 {
    let mut positional: Vec<&str> = Vec::new();
    for arg in tail {
        match arg.as_str() {
            "--ai-resolve" | "--" => {}
            "--no-push" => std::env::set_var("WORKTREE_NO_PUSH", "1"),
            "-h" | "--help" => {
                eprintln!("usage: cc worktree <branch> [env-base-folder] [--no-push]");
                return 0;
            }
            other if other.starts_with('-') => {
                eprintln!("worktree: unknown option: {other}");
                return 2;
            }
            other => positional.push(other),
        }
    }
    let branch = positional.first().copied().unwrap_or("");
    let path = match worktree_run::setup(cwd, branch, positional.get(1).copied()) {
        Ok(path) => path,
        Err(code) => return code,
    };
    record_cd(&path);
    *cwd = path;
    dispatch(cwd, flags)
}

fn record_cd(path: &Path) {
    if let Some(file) = std::env::var_os(CD_FILE_ENV).filter(|f| !f.is_empty()) {
        let _ = fs::write(file, format!("{}\n", path.display()));
    }
}

fn claude(cwd: &Path, args: Vec<String>) -> i32 {
    spawn(cwd, &args, None)
}

fn spawn(cwd: &Path, args: &[String], stderr: Option<Stdio>) -> i32 {
    let mut cmd = Command::new("claude");
    cmd.args(args).current_dir(cwd);
    if let Some(stderr) = stderr {
        cmd.stderr(stderr);
    }
    match cmd.status() {
        Ok(status) => {
            use std::os::unix::process::ExitStatusExt;
            status
                .code()
                .unwrap_or_else(|| 128 + status.signal().unwrap_or(0))
        }
        Err(err) => {
            eprintln!("-> cc: could not run claude: {err}");
            127
        }
    }
}

fn clear_screen() {
    use std::io::{IsTerminal, Write};
    if std::io::stdout().is_terminal() {
        print!("\x1b[2J\x1b[H");
        let _ = std::io::stdout().flush();
    }
}

fn short(sid: &str) -> &str {
    sid.get(..8).unwrap_or(sid)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn v(items: &[&str]) -> Vec<String> {
        items.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn value_taking_flag_does_not_leak_its_value_as_the_subcommand() {
        let (flags, rest) = split_flags(&v(&["--model", "opus", "clean", "x"]));
        assert_eq!(flags, v(&["--model", "opus"]));
        assert_eq!(rest, v(&["clean", "x"]));
    }

    #[test]
    fn equals_form_and_boolean_flags_take_no_value() {
        let (flags, rest) = split_flags(&v(&["--model=opus", "--verbose", "list"]));
        assert_eq!(flags, v(&["--model=opus", "--verbose"]));
        assert_eq!(rest, v(&["list"]));
    }

    #[test]
    fn a_trailing_value_flag_with_no_value_is_kept_alone() {
        let (flags, rest) = split_flags(&v(&["--model"]));
        assert_eq!(flags, v(&["--model"]));
        assert!(rest.is_empty());
    }
}
