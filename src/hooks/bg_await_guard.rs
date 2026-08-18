// SPDX-FileCopyrightText: 2026 Igor Santos
// SPDX-License-Identifier: MIT

//! Ports hooks/bg-await-guard.sh: warns when a Bash call backgrounds an
//! install, build or typecheck whose output a later step usually needs.
//!
//! Warn, never block. Backgrounding a genuinely long job and awaiting its exit
//! notification is legitimate; racing it is the bug, and the guard cannot tell
//! the two apart from the command alone.
//!
//! Matching is token-based rather than a regex, since the crate graph is
//! deliberately clap plus serde only. The shell used a case-insensitive
//! extended regex, so comparisons here lowercase first.

use crate::common::emit_pre_context;
use crate::common::payload::Payload;

/// Package managers whose install output a later step almost always needs.
const PACKAGE_MANAGERS: [&str; 4] = ["npm", "pnpm", "yarn", "bun"];
const INSTALL_VERBS: [&str; 4] = ["install", "ci", "add", "i"];

/// Build tools that must start a command to count, so `foo --flag make` is not
/// a match.
const BUILD_TOOLS: [&str; 4] = ["tsc", "make", "gradle", "mvn"];

const COMPILERS: [&str; 2] = ["cargo", "go"];
const LANG_INSTALLERS: [&str; 3] = ["pip", "poetry", "bundle"];

/// Values jq renders for a JSON true, plus the shell's other accepted spellings.
const TRUTHY: [&str; 3] = ["true", "1", "yes"];

const WARNING: &str = "You backgrounded a command whose result a later step usually needs (install/build/typecheck). Backgrounding it and then running the next command is a common failure: e.g. `npm run build` before `npm install` finished gives `tsc: not found`. If anything downstream depends on this, run it in the FOREGROUND (run_in_background off) with an extended timeout (up to 600000ms) instead. A backgrounded job re-invokes you only when it exits; shell state and `wait` do NOT persist across Bash calls, so don't poll it (no Monitor/`wait` to synchronize).";

pub fn run(payload: &Payload) {
    if std::env::var("BG_AWAIT_GUARD").as_deref() == Ok("0") {
        return;
    }
    let cmd = payload.field(".tool_input.command");
    if cmd.is_empty() {
        return;
    }
    let backgrounded = payload.field(".tool_input.run_in_background");
    if !TRUTHY.contains(&backgrounded.as_str()) {
        return;
    }
    if is_await_sensitive(&cmd) {
        emit_pre_context("PreToolUse", WARNING);
    }
}

fn is_await_sensitive(cmd: &str) -> bool {
    let lower = cmd.to_lowercase();
    let tokens: Vec<&str> = lower.split_whitespace().collect();

    for (i, tok) in tokens.iter().enumerate() {
        let next = tokens.get(i + 1).copied();
        let after = tokens.get(i + 2).copied();

        if PACKAGE_MANAGERS.contains(tok) {
            if next.is_some_and(|n| INSTALL_VERBS.contains(&n)) {
                return true;
            }
            if next == Some("run") && after == Some("build") {
                return true;
            }
        }
        if COMPILERS.contains(tok) && next == Some("build") {
            return true;
        }
        if LANG_INSTALLERS.contains(tok) && next == Some("install") {
            return true;
        }
        if BUILD_TOOLS.contains(tok) && starts_a_command(&tokens, i) {
            return true;
        }
    }

    wipes_node_modules(&tokens)
}

/// True when the token begins a command: first overall, or preceded by a
/// separator. Without this, `tsc` appearing as an argument would match.
fn starts_a_command(tokens: &[&str], i: usize) -> bool {
    match i.checked_sub(1).and_then(|p| tokens.get(p)) {
        None => true,
        Some(prev) => prev.ends_with([';', '&', '|', '(']),
    }
}

/// `rm -rf node_modules` and friends: the reinstall that follows is always
/// awaited, so backgrounding the wipe races it.
fn wipes_node_modules(tokens: &[&str]) -> bool {
    tokens.iter().enumerate().any(|(i, tok)| {
        *tok == "rm"
            && tokens[i + 1..].iter().any(|t| t.contains("node_modules"))
            && tokens[i + 1..]
                .iter()
                .any(|t| t.starts_with('-') && t[1..].chars().all(|c| c.is_ascii_lowercase()))
    })
}
