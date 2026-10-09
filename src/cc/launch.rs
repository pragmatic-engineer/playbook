// SPDX-FileCopyrightText: 2026 Igor Santos
// SPDX-License-Identifier: Apache-2.0

//! `playbook cc launch`: the launcher the `ccc` and `ccd` shell functions call.
//!
//! Subcommands: (none) resume or start, `clean`, `fresh`, `raw [sid]`, `list`,
//! `prune`, `worktree <branch>`. The shell function only wraps this and `cd`s
//! to the path written to `$PLAYBOOK_CC_CD_FILE`, the one thing a child cannot do.

use super::{bust_cache, clean_resume, config_drift, retention, sessions, worktree_run};
use std::fs;
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
            | "--fallback-model"
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

/// Claude Code's documented switch for its own auto memory.
const DISABLE_AUTO_MEMORY_ENV: &str = "CLAUDE_CODE_DISABLE_AUTO_MEMORY";

/// Whether `memory.source` is `playbook`, so Claude Code's auto memory stays
/// off in this session and only playbook memory is used.
fn playbook_memory_only(home: &Path) -> bool {
    matches!(
        crate::config::resolve("memory.source", home, None),
        Ok((serde_json::Value::String(v), _)) if v == "playbook"
    )
}

/// Runs the launcher and returns the exit code of the session (or the failure).
pub fn run(skip_permissions: bool, args: &[String]) -> i32 {
    let mut all: Vec<String> = Vec::new();
    if skip_permissions {
        all.push("--dangerously-skip-permissions".into());
    }
    let prompt = crate::common::paths::playbook_root().join(SYSTEM_PROMPT_REL);
    if prompt.is_file() {
        all.push("--system-prompt-file".into());
        all.push(prompt.to_string_lossy().into_owned());
    }
    let cwd_now = PathBuf::from(super::logical_cwd());
    let has_settings = args
        .iter()
        .any(|a| a == "--settings" || a.starts_with("--settings="));
    if !has_settings {
        let home = crate::common::home_dir();
        if let Some(json) = crate::effort::launcher_settings(&home, &home.join(".claude"), &cwd_now)
        {
            all.push("--settings".into());
            all.push(json);
        }
    }
    let user_settings = crate::common::home_dir().join(".claude/settings.json");
    if let Some(flags) = crate::models::launcher_flags(args, &user_settings) {
        all.extend(flags);
    }
    all.extend_from_slice(args);
    if playbook_memory_only(&crate::common::home_dir()) {
        // The session's own environment only. Claude Code's settings files
        // and memory directories are never touched.
        std::env::set_var(DISABLE_AUTO_MEMORY_ENV, "1");
    }

    let mut cwd = cwd_now;
    let rc = dispatch(&mut cwd, &all);
    retention::prune(&cwd.to_string_lossy());
    rc
}

#[cfg(unix)]
extern "C" fn noop(_: libc::c_int) {}

/// Runs `f` with a no-op SIGINT handler: Ctrl-C reaches the whole foreground
/// group, and the launcher must outlive the session to prune and report. A
/// handler (not SIG_IGN) resets on exec, so claude keeps its own behaviour.
#[cfg(unix)]
fn with_sigint_held<T>(f: impl FnOnce() -> T) -> T {
    // SAFETY: installing an empty handler is async-signal-safe.
    let previous = unsafe { libc::signal(libc::SIGINT, noop as *const () as libc::sighandler_t) };
    let out = f();
    // SAFETY: restores the handler captured above.
    unsafe { libc::signal(libc::SIGINT, previous) };
    out
}

#[cfg(not(unix))]
fn with_sigint_held<T>(f: impl FnOnce() -> T) -> T {
    f()
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
        println!("-> ccc clean: no matching session for '{name}'; starting fresh");
        return claude(
            cwd,
            [&["-n".into(), name.to_string()], flags, tail].concat(),
        );
    };
    match clean_resume::prepare(project_dir, &old) {
        Ok(done) => {
            println!(
                "-> ccc clean: cloned {}... -> {}... (stripped {} override entries)",
                short(&old),
                short(&done.new_sid),
                done.stripped
            );
            println!("-> ccc clean: settings.json + plugins + hooks reload from current config");
            config_drift::stamp(&cwd.to_string_lossy());
            let head = ["--resume".into(), done.new_sid, "-n".into(), name.into()];
            claude(cwd, [&head[..], flags, tail].concat())
        }
        Err(clean_resume::CleanError::TranscriptMissing(path)) => {
            println!("-> ccc clean: transcript missing at {}", path.display());
            1
        }
        Err(clean_resume::CleanError::Io(err)) => {
            println!("-> ccc clean: {err}");
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
            println!("-> ccc raw: no matching session; starting fresh");
            claude(cwd, [flags, &name_args[..], tail].concat())
        }
        Some(sid) => {
            println!("-> ccc raw: resuming {sid} (no fork, overrides preserved)");
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
    let (err_path, stderr) = private_err_file();
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
    let mut no_push = false;
    for arg in tail {
        match arg.as_str() {
            "--ai-resolve" | "--" => {}
            "--no-push" => no_push = true,
            "-h" | "--help" => {
                eprintln!("usage: ccc worktree <branch> [env-base-folder] [--no-push]");
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
    // The flag reaches the worktree code through the same variable the
    // shell used; it is cleared again so the session does not inherit it.
    let had_no_push = std::env::var_os("WORKTREE_NO_PUSH").is_some();
    if no_push {
        std::env::set_var("WORKTREE_NO_PUSH", "1");
    }
    let setup = worktree_run::setup(cwd, branch, positional.get(1).copied());
    if no_push && !had_no_push {
        std::env::remove_var("WORKTREE_NO_PUSH");
    }
    let path = match setup {
        Ok((path, housekeep)) => {
            housekeep.spawn_detached();
            path
        }
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

/// A fresh 0600 file that fails rather than follow a planted path, standing
/// in for the `mktemp` the shell used.
fn private_err_file() -> (PathBuf, Option<Stdio>) {
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    let path = std::env::temp_dir().join(format!("playbook-cc-err-{}-{nanos}", std::process::id()));
    let mut options = fs::OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    std::os::unix::fs::OpenOptionsExt::mode(&mut options, 0o600);
    let file = options.open(&path).ok();
    (path, file.map(Stdio::from))
}

fn claude(cwd: &Path, args: Vec<String>) -> i32 {
    spawn(cwd, &args, None)
}

fn spawn(cwd: &Path, args: &[String], stderr: Option<Stdio>) -> i32 {
    let mut cmd = Command::new("claude");
    cmd.args(args)
        .current_dir(cwd)
        .env("PWD", cwd)
        .env_remove(CD_FILE_ENV);
    if let Some(stderr) = stderr {
        cmd.stderr(stderr);
    }
    match with_sigint_held(|| cmd.status()) {
        Ok(status) => exit_code(status),
        Err(err) => {
            eprintln!("-> cc: could not run claude: {err}");
            127
        }
    }
}

#[cfg(unix)]
fn exit_code(status: std::process::ExitStatus) -> i32 {
    use std::os::unix::process::ExitStatusExt;
    status
        .code()
        .unwrap_or_else(|| 128 + status.signal().unwrap_or(0))
}

#[cfg(not(unix))]
fn exit_code(status: std::process::ExitStatus) -> i32 {
    status.code().unwrap_or(1)
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
