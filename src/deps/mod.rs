// SPDX-FileCopyrightText: 2026 Igor Santos
// SPDX-License-Identifier: MIT

//! `playbook deps ensure`: per-dependency check-then-install from a Brewfile.
//! Ported from `shell/ensure-deps.sh`. A tool already on PATH (from any
//! source) is kept; Homebrew installs only what is missing.

use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

/// The command found on PATH for `cmd`, if any.
fn which(cmd: &str) -> Option<PathBuf> {
    let path = std::env::var_os("PATH")?;
    std::env::split_paths(&path)
        .map(|dir| dir.join(cmd))
        .find(|candidate| is_executable(candidate))
}

#[cfg(unix)]
fn is_executable(path: &Path) -> bool {
    use std::os::unix::fs::PermissionsExt;
    path.metadata()
        .is_ok_and(|m| m.is_file() && m.permissions().mode() & 0o111 != 0)
}

#[cfg(not(unix))]
fn is_executable(path: &Path) -> bool {
    path.is_file()
}

fn has_brew() -> bool {
    which("brew").is_some()
}

/// Keep `cmd` if it is on PATH, else `brew install <formula>`. False only when
/// an attempted install fails. Without Homebrew it prints guidance and is true.
pub fn ensure_dep(cmd: &str, formula: &str) -> bool {
    if let Some(found) = which(cmd) {
        println!("ensure-deps: {cmd} already installed ({})", found.display());
        return true;
    }
    if !has_brew() {
        eprintln!(
            "ensure-deps: {cmd} not found and Homebrew is unavailable; install {formula} manually"
        );
        return true;
    }
    println!("ensure-deps: {cmd} not found; installing {formula} via Homebrew");
    let installed = Command::new("brew")
        .args(["install", formula])
        .stdin(Stdio::null())
        .status()
        .is_ok_and(|s| s.success());
    if installed {
        return true;
    }
    // Trusting a tap grants it code execution, so print the command, never run it.
    if let Some((tap, _)) = formula.rsplit_once('/') {
        eprintln!(
            "ensure-deps: if this failed as an untrusted tap, review it and run: brew trust {tap}"
        );
    }
    false
}

/// Quoted value of every `<kind> "X"` line in a Brewfile.
fn entries(text: &str, kind: &str) -> Vec<String> {
    let prefix = format!("{kind} \"");
    text.lines()
        .filter_map(|line| {
            let rest = line.strip_prefix(&prefix)?;
            Some(rest[..rest.find('"')?].to_string())
        })
        .collect()
}

/// Tap each `tap "X"` line first (a tapped formula cannot install before its
/// tap exists), then ensure each `brew "X"` formula. False if any step failed.
pub fn ensure_all(brewfile: &Path) -> bool {
    let Ok(text) = std::fs::read_to_string(brewfile) else {
        eprintln!("ensure-deps: no Brewfile at {}", brewfile.display());
        return true;
    };
    let mut ok = true;
    if has_brew() {
        let listed = Command::new("brew")
            .arg("tap")
            .stdin(Stdio::null())
            .output()
            .map(|o| String::from_utf8_lossy(&o.stdout).to_string())
            .unwrap_or_default();
        for tap in entries(&text, "tap") {
            if listed.lines().any(|l| l == tap) {
                println!("ensure-deps: tap {tap} already present");
            } else {
                println!("ensure-deps: adding tap {tap}");
                let added = Command::new("brew")
                    .args(["tap", &tap])
                    .stdin(Stdio::null())
                    .status()
                    .is_ok_and(|s| s.success());
                ok &= added;
            }
        }
    }
    for formula in entries(&text, "brew") {
        let cmd = formula.rsplit('/').next().unwrap_or(&formula);
        ok &= ensure_dep(cmd, &formula);
    }
    ok
}
