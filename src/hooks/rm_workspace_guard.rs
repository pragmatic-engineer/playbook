// SPDX-FileCopyrightText: 2026 Igor Santos
// SPDX-License-Identifier: MIT

//! Ports hooks/rm-workspace-guard.sh: denies an `rm` whose target sits outside
//! the safe roots, `~/.claude/**`, and the scratch trees `/tmp` and
//! `~/.cache` (contents only, not those roots themselves).
//!
//! Best-effort protection against an accidental `rm`, NOT a security boundary.
//! It only sees `rm`, so `find -delete`, `unlink` and `>` truncation pass, and
//! anything it cannot resolve is blocked rather than evaluated.
//!
//! Four deliberate conservative blocks. Three are carried over unchanged: a
//! `cd` anywhere makes relative targets unresolvable, a command substitution
//! could expand to anything, and a quoted path containing a space still splits
//! into two tokens and is judged as two paths. The fourth was added later: a
//! target containing `$` or a backtick is unresolvable, because it expands at
//! runtime to a path the guard never sees. That one fixed a FAIL-OPEN rather
//! than tightening an existing block, so it is the one to be careful about
//! reverting.
//!
//! A heredoc body is DATA, not commands, so a mention inside one is prose
//! unless it starts a line. That was previously an accepted false positive,
//! revised on 2026-08-23 because commit messages written through a heredoc were
//! blocking real work. A body line that STARTS with a deletion is still in
//! command position and still blocks, which is what keeps `bash <<EOF` honest.
//!
//! **ACCEPTED MISS**, pinned so it is not re-reported as a vulnerability later:
//! a command name that is obfuscated or built at runtime is not resolved, so it
//! is not recognised as a deletion. Escaped and quote-split spellings, and names
//! produced by command or parameter substitution, all fall in this class.
//! Closing it needs word expansion, which for the substitution cases cannot be
//! done statically at all. This is deliberate and in scope for a guard that
//! exists to catch an ACCIDENT: no agent writes an obfuscated command name by
//! accident, and the ordinary forms it would write, including `sudo`, `xargs`,
//! `/bin/rm`, env prefixes and multi-line commands, are all still caught.
//!
//! **One documented divergence: JSON formatting.** This is the only guard that
//! never sourced `lib/common.sh`; it piped through `jq -n`, so its output was
//! pretty-printed while every other hook emits compact JSON. The port uses the
//! shared `emit_pre_deny`, making the bytes differ but the parsed object
//! identical, verified across all 22 scenarios. Reproducing jq's indentation
//! would mean a second emitter used by exactly one hook, for a consumer that
//! parses the JSON either way.

use crate::common::emit_pre_deny;
use crate::common::payload::Payload;
use crate::common::proc::run_with_timeout;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::Duration;

/// The shell ran `git rev-parse` unbounded. A wedged git would have stalled the
/// PreToolUse event, so the port bounds it and falls back to the cwd.
const GIT_TIMEOUT: Duration = Duration::from_secs(5);

/// Placeholder reported when `rm` is reached through `$(...)` or a backtick.
const SUBSTITUTION_LABEL: &str = "<command substitution>";

const SEPARATORS: [&str; 5] = [";", "&&", "||", "|", "&"];

/// Scratch space allowed regardless of `PLAYBOOK_SAFE_ROOTS`, the same standing
/// exemption `~/.claude` already has.
///
/// BOTH spellings are listed because `canon` is lexical and never resolves a
/// symlink: on macOS `/tmp` is a symlink to `/private/tmp`, so listing one would
/// allow `/tmp/x` while still blocking the identical `/private/tmp/x`.
///
/// The temp root ITSELF stays blocked, unlike a configured safe root, which may
/// be deleted whole. `rm -rf /tmp` takes out sockets and runtime state that live
/// processes depend on, which is the class of accident this guard exists to
/// stop, and no ordinary cleanup needs it.
const TEMP_ROOTS: [&str; 2] = ["/tmp", "/private/tmp"];

pub fn run(payload: &Payload) {
    let cmd = payload.field(".tool_input.command");
    if cmd.is_empty() {
        return;
    }

    let home = std::env::var("HOME").unwrap_or_default();
    let claude_dir = PathBuf::from(&home).join(".claude");
    let safe_roots = safe_roots();

    let outside = offending_targets(&cmd, &home, &claude_dir, &safe_roots);
    if outside.is_empty() {
        return;
    }

    let roots = if safe_roots.is_empty() {
        "(no safe roots configured)".to_string()
    } else {
        safe_roots
            .iter()
            .map(|r| format!("{}/**", r.display()))
            .collect::<Vec<_>>()
            .join(", ")
    };
    // Comma with no space: the shell built this with `IFS=', '` and `${arr[*]}`,
    // which joins on the FIRST character of IFS only. Matching bash's quirk
    // rather than the prettier form keeps the message byte-identical.
    emit_pre_deny(&format!(
        "rm blocked: {} is outside {roots} and ~/.claude/**",
        outside.join(",")
    ));
}

/// `PLAYBOOK_SAFE_ROOTS` is colon-separated like `$PATH`. Unset or empty falls
/// back to the git repo root, then the cwd. `$HOME` is deliberately not the
/// default, since that would unblock `~/.ssh` and `~/.aws`.
fn safe_roots() -> Vec<PathBuf> {
    let configured = std::env::var("PLAYBOOK_SAFE_ROOTS").unwrap_or_default();
    if configured.is_empty() {
        let root = git_repo_root().unwrap_or_else(|| std::env::current_dir().unwrap_or_default());
        return vec![canon(&root)];
    }
    configured
        .split(':')
        .filter(|r| !r.is_empty())
        .map(|r| canon(Path::new(r)))
        .collect()
}

fn git_repo_root() -> Option<PathBuf> {
    let mut command = Command::new("git");
    command.args(["rev-parse", "--show-toplevel"]);
    let out = run_with_timeout(&mut command, GIT_TIMEOUT)?;
    if !out.status.success() {
        return None;
    }
    let path = String::from_utf8_lossy(&out.stdout).trim().to_string();
    (!path.is_empty()).then(|| PathBuf::from(path))
}

/// Resolves `.` and `..` lexically, never touching the filesystem, because the
/// target of an `rm` may not exist. This is what closes the `..` traversal
/// bypass: `~/Workspace/../secrets` collapses outside the allowlist and blocks.
fn canon(path: &Path) -> PathBuf {
    let absolute = if path.is_absolute() {
        path.to_path_buf()
    } else {
        std::env::current_dir().unwrap_or_default().join(path)
    };

    let text = absolute.to_string_lossy().into_owned();
    let mut out: Vec<&str> = Vec::new();
    for seg in text.split('/') {
        match seg {
            "" | "." => {}
            ".." => {
                out.pop();
            }
            other => out.push(other),
        }
    }
    if out.is_empty() {
        return PathBuf::from("/");
    }
    PathBuf::from(format!("/{}", out.join("/")))
}

fn is_allowed(target: &str, home: &str, claude_dir: &Path, safe_roots: &[PathBuf]) -> bool {
    let expanded = match target.strip_prefix('~') {
        Some(rest) if !home.is_empty() => format!("{home}{rest}"),
        _ => target.to_string(),
    };
    let path = canon(Path::new(&expanded));
    if path == claude_dir || path.starts_with(claude_dir) {
        return true;
    }
    // Scratch space, allowed whatever the roots say. `starts_with` compares whole
    // components, so `/tmpfoo` does not match `/tmp`, and `canon` has already
    // collapsed `..`, so `/tmp/../etc` is judged as `/etc` and still blocks.
    if TEMP_ROOTS
        .iter()
        .any(|root| path.starts_with(root) && path != Path::new(root))
    {
        return true;
    }
    // `~/.cache` is regenerable scratch by definition, and it is where this
    // repo's own test fixtures live. Derived from HOME the same way `~/.claude`
    // is, deliberately NOT from `XDG_CACHE_HOME`: that variable can be set to
    // `/`, which would hand the whole filesystem to the allowlist, whereas the
    // worst a bad HOME yields here is a narrow `<junk>/.cache`.
    //
    // Contents only, following TEMP_ROOTS rather than `~/.claude`: no cleanup
    // needs to delete the cache root itself, and doing so by accident costs
    // every rebuild on the machine.
    if !home.is_empty() {
        let cache_dir = Path::new(home).join(".cache");
        if path.starts_with(&cache_dir) && path != cache_dir {
            return true;
        }
    }
    safe_roots
        .iter()
        .any(|root| path == *root || path.starts_with(root))
}

/// The delimiter of a heredoc opened on this line, if any.
///
/// The delimiter must start with a letter or `_`, which is what keeps an
/// arithmetic shift like `$((1 << 2))` from being read as a heredoc.
fn heredoc_delimiter(line: &str) -> Option<String> {
    let chars: Vec<char> = line.chars().collect();
    let (mut in_single, mut in_double) = (false, false);
    let mut i = 0;
    while i < chars.len() {
        match chars[i] {
            '\'' if !in_double => in_single = !in_single,
            '"' if !in_single => in_double = !in_double,
            '<' if !in_single && !in_double && chars.get(i + 1) == Some(&'<') => {
                let mut j = i + 2;
                if chars.get(j) == Some(&'-') {
                    j += 1;
                }
                while chars.get(j) == Some(&' ') {
                    j += 1;
                }
                let quote = match chars.get(j) {
                    Some(&q @ ('\'' | '"')) => {
                        j += 1;
                        Some(q)
                    }
                    _ => None,
                };
                let start = j;
                while j < chars.len() && (chars[j].is_alphanumeric() || chars[j] == '_') {
                    j += 1;
                }
                let word: String = chars[start..j].iter().collect();
                let named = word
                    .chars()
                    .next()
                    .is_some_and(|c| c.is_alphabetic() || c == '_');
                let closed = quote.is_none_or(|q| chars.get(j) == Some(&q));
                if named && closed {
                    return Some(word);
                }
                i = j;
                continue;
            }
            _ => {}
        }
        i += 1;
    }
    None
}

/// Marks which lines are heredoc BODY, which is data rather than commands.
///
/// Heredoc mode is only entered when the terminator actually appears later. An
/// unterminated `<<` would otherwise turn the rest of the command into data and
/// hide a deletion, which is the wrong way for this to fail.
fn heredoc_body_lines(cmd: &str) -> Vec<bool> {
    let lines: Vec<&str> = cmd.lines().collect();
    let mut body = vec![false; lines.len()];
    let mut i = 0;
    while i < lines.len() {
        if let Some(delim) = heredoc_delimiter(lines[i]) {
            if let Some(end) = lines
                .iter()
                .enumerate()
                .skip(i + 1)
                .find(|(_, l)| l.trim() == delim)
                .map(|(j, _)| j)
            {
                for flag in body.iter_mut().take(end).skip(i + 1) {
                    *flag = true;
                }
                i = end + 1;
                continue;
            }
        }
        i += 1;
    }
    body
}

/// Splits on spaces, keeping quote characters in the token text, and reports
/// whether each token is DATA rather than a command: either inside shell quotes
/// or inside a heredoc body. A newline becomes a `;` token, so the existing
/// command-position rule still sees the start of each body line.
///
/// Splitting continues INSIDE quotes, which is deliberate: identical tokens mean
/// target judging is byte-for-byte unchanged, and it is what still catches
/// `sh -c "cd /x && rm -rf /etc"`.
///
/// An unbalanced quote leaves the remaining tokens marked as data. That can only
/// make the guard demand a separator before believing a deletion, never fewer
/// checks on the unquoted path, so the failure direction is unchanged.
fn tokenize(cmd: &str) -> Vec<(String, bool)> {
    let body = heredoc_body_lines(cmd);
    let mut tokens = Vec::new();
    let (mut in_single, mut in_double) = (false, false);

    for (index, line) in cmd.lines().enumerate() {
        if index > 0 {
            // A newline resets in-rm state exactly as `;` does.
            tokens.push((";".to_string(), false));
        }
        let is_body = body.get(index).copied().unwrap_or(false);
        let mut current = String::new();
        let mut current_data = false;

        for ch in line.replace('\t', " ").chars() {
            if ch == ' ' {
                if !current.is_empty() {
                    tokens.push((std::mem::take(&mut current), current_data));
                }
                continue;
            }
            if current.is_empty() {
                current_data = is_body || in_single || in_double;
            }
            current.push(ch);
            // Quote state is NOT tracked inside a heredoc body: the body is
            // literal text, and an ordinary apostrophe in prose would otherwise
            // flip the state and mis-mark every token after it.
            if !is_body {
                match ch {
                    '\'' if !in_double => in_single = !in_single,
                    '"' if !in_single => in_double = !in_double,
                    _ => {}
                }
            }
        }
        if !current.is_empty() {
            tokens.push((current, current_data));
        }
    }
    tokens
}

/// Walks the command left to right tracking whether the cursor sits inside an
/// `rm`'s argument list, which is what lets `rm a && ls b` judge only `a`.
fn offending_targets(
    cmd: &str,
    home: &str,
    claude_dir: &Path,
    safe_roots: &[PathBuf],
) -> Vec<String> {
    let mut outside = Vec::new();
    let mut in_rm = false;
    let mut saw_cd = false;
    let mut saw_rm = false;
    // The very start of a command is a command position, same as after a `;`.
    let mut prev_was_separator = true;

    for (token, is_data) in tokenize(cmd) {
        let token = token.as_str();

        // A word that is DATA, meaning inside quotes or inside a heredoc body,
        // is prose unless a separator put it in command position. `-m "fix: stop
        // using rm"` is a message and a commit message mentioning `rm` is prose,
        // while `sh -c "cd /x && rm -rf /etc"` and a heredoc line that STARTS
        // with `rm` really do delete. Outside those regions nothing changes, so
        // `sudo rm`, `xargs rm` and `find -exec rm` are all still caught.
        let is_command = !is_data || prev_was_separator;

        if is_command && (token == "cd" || token.ends_with("/cd")) {
            saw_cd = true;
            prev_was_separator = false;
            continue;
        }
        if is_command && (token == "rm" || token.ends_with("/rm")) {
            in_rm = true;
            saw_rm = true;
            prev_was_separator = false;
            continue;
        }
        if SEPARATORS.contains(&token) {
            in_rm = false;
            prev_was_separator = true;
            continue;
        }
        prev_was_separator = false;
        if !in_rm || token.starts_with('-') {
            continue;
        }
        // A target carrying `$` or a backtick expands at runtime to a path the
        // guard cannot see, so it is unresolvable in the same way a relative
        // target after a `cd` is.
        //
        // This one closes a FAIL-OPEN, which is why it is worth the false
        // positives it adds. `canon` treats a leading `$` as a relative path and
        // joins it to the cwd, so `rm -rf "$HOME/.cache/x"` resolved to
        // `<repo>/$HOME/.cache/x`, landed inside a safe root, and was ALLOWED.
        // The shell then expanded it to a real path outside the workspace, which
        // is precisely the accident this guard exists to prevent, and agents
        // write `$VAR` paths as a matter of course.
        //
        // The cost is that `rm -rf "$REPO/target"` now blocks too, even though
        // it would have expanded to somewhere allowed. That is the correct
        // direction to be wrong in: the caller can retry with a literal path.
        let unresolvable = token.contains('$')
            || token.contains('`')
            || (saw_cd && !token.starts_with('/') && !token.starts_with('~'));
        if unresolvable || !is_allowed(token, home, claude_dir, safe_roots) {
            outside.push(token.to_string());
        }
    }

    if saw_rm && (cmd.contains("$(") || cmd.contains('`')) {
        outside.push(SUBSTITUTION_LABEL.to_string());
    }
    outside
}
